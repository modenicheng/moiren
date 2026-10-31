//! Cached controls behind a [`PanelHost`] and the display-rate meter layer
//! behind a [`HotLayerView`]: the meter canvas is a sibling, never a
//! descendant of the cached subtree — gpui-fast replays a cached panel's
//! paint without re-running its closures, so live bars must be painted by
//! the hot layer at the frame the panel's prepaint registered geometry for.
use super::*;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::window::hot::{HotLayerView, HotSurface};
use crate::ui::window::panel::{CachedPanel, PanelHost, cached_or_live};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Placement {
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
}

impl Placement {
    fn decorations_visible(self) -> bool {
        // Clip lamps are 10 px above the track; labels extend 16 px below.
        // Culling only the track drops those decorations during vertical scroll.
        let bounds = Bounds::new(
            self.bounds.origin - point(px(0.), px(10.)),
            size(self.bounds.size.width, self.bounds.size.height + px(26.)),
        );
        let visible = bounds.intersect(&self.mask.bounds);
        visible.size.width > px(0.) && visible.size.height > px(0.)
    }
}

#[derive(Clone, Default)]
pub(in crate::ui::window) struct MeterGeometry {
    tracks: BTreeMap<NodeId, (Placement, usize, Option<bool>)>,
    readouts: BTreeMap<NodeId, Placement>,
}

/// Where the strip's meter tracks and readouts sit, registered by the
/// cached strip's canvases during a rebuild and read by the hot layer at
/// paint time. Stale by design between rebuilds.
pub(in crate::ui::window) type Geometry = SharedFrame<MeterGeometry>;

#[cfg(all(test, feature = "gpui-test"))]
impl Geometry {
    pub(super) fn track_bounds(&self, id: NodeId) -> Option<Bounds<Pixels>> {
        self.read()?
            .tracks
            .get(&id)
            .map(|(placement, _, _)| placement.bounds)
    }
}

/// One strip of the Mixer page as its own cached panel, keyed on that
/// strip's projected inputs. The page mounts one host per node as siblings
/// (never nested in another cached panel), so a publish during a fader drag
/// rebuilds only the strip whose gain actually changed — the rest compare
/// their keys equal and replay. This is the cache split the design's
/// "split where the change rate differs" rule asks for: the dragged strip
/// changes at the dispatch rate, its siblings at the session rate.
pub(in crate::ui::window) struct StripPanel {
    pub(in crate::ui::window) id: NodeId,
}

/// Only this strip's control-tree inputs, projected: derived booleans over
/// the snapshot (`managed`, `probe`) instead of the snapshot fields behind
/// them, so a key capture stays allocation-free apart from the graph Arc.
/// Elapsed time and audio observations must not appear here.
pub(in crate::ui::window) struct StripKey {
    id: NodeId,
    graph: std::sync::Arc<crate::service::GraphSnapshot>,
    catalog: std::sync::Arc<crate::service::DeviceCatalog>,
    /// The node's level topic exists: the managed-meter visibility canvas is
    /// mounted and the hot layer expects a track.
    managed: bool,
    /// The strip is the session's observed source/sink: the legacy probe
    /// canvas claims one of the shared `meter_points` flags.
    probe: bool,
    selected: bool,
    compact: bool,
    position: usize,
    editing_value: bool,
    editing_balance: bool,
    editing_send: Option<EdgeId>,
    collapsed: (bool, bool),
    /// Registered meter geometry goes stale under scroll just like under any
    /// other layout change, so the offset rides in the key.
    scroll: Point<Pixels>,
    meters: crate::project::MeterDisplayPreferences,
}

impl PartialEq for StripKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.managed == other.managed
            && self.probe == other.probe
            && self.selected == other.selected
            && self.compact == other.compact
            && self.position == other.position
            && self.editing_value == other.editing_value
            && self.editing_balance == other.editing_balance
            && self.editing_send == other.editing_send
            && self.collapsed == other.collapsed
            && self.scroll == other.scroll
            && self.meters == other.meters
            && self.catalog == other.catalog
            && strip_content_eq(&self.graph, &other.graph, self.id)
    }
}

impl StripPanel {
    fn key_for(id: NodeId, shell: &StudioShell, cx: &App) -> StripKey {
        let state = shell.state.read(cx);
        let snapshot = &state.snapshot;
        let graph = &snapshot.graph;
        let probe = graph
            .graph
            .get_node(id)
            .ok()
            .zip(graph.node(id))
            .is_some_and(|(node, entry)| strip_probe_candidate(snapshot, entry, node.kind()));
        StripKey {
            id,
            graph: snapshot.graph.clone(),
            catalog: snapshot.catalog.clone(),
            managed: snapshot.signal_topics.node_levels.contains_key(&id),
            probe,
            selected: shell.selected_node == Some(id),
            compact: shell.compact,
            position: shell
                .mixer
                .order
                .iter()
                .position(|node| *node == id)
                .unwrap_or(0),
            editing_value: shell.mixer.editing_value == Some(id),
            editing_balance: shell.mixer.editing_balance == Some(id),
            editing_send: shell.mixer.editing_send,
            collapsed: (shell.mixer.inputs_collapsed, shell.mixer.sends_collapsed),
            scroll: shell.mixer_scroll.offset(),
            meters: shell.preferences.meters,
        }
    }
}

/// Whether a strip's rendered content is identical between two snapshots: its
/// own node state, kind, channels and ports, the edges terminating at or
/// originating from it in graph order (input/send rows render in that order,
/// with each far end's display name), and the export intents that can name a
/// rendered node. Deliberately ignores session bookkeeping — revisions,
/// faults, undo flags — that no strip renders: a whole-snapshot compare would
/// drag every sibling strip into a rebuild on every edit publish.
fn strip_content_eq(
    a: &std::sync::Arc<crate::service::GraphSnapshot>,
    b: &std::sync::Arc<crate::service::GraphSnapshot>,
    id: NodeId,
) -> bool {
    if std::sync::Arc::ptr_eq(a, b) {
        return true;
    }
    let (a, b) = (&**a, &**b);
    if a.node(id) != b.node(id) {
        return false;
    }
    let touching = |snapshot: &crate::service::GraphSnapshot| {
        snapshot
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.dst() == id || edge.src() == id)
            .map(|edge| {
                (
                    edge.id(),
                    edge.src(),
                    edge.dst(),
                    edge.src_port(),
                    edge.dst_port(),
                    *edge.params(),
                )
            })
            .collect::<Vec<_>>()
    };
    if touching(a) != touching(b) {
        return false;
    }
    let far_ends = |snapshot: &crate::service::GraphSnapshot| {
        snapshot
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.dst() == id || edge.src() == id)
            .flat_map(|edge| [edge.src(), edge.dst()])
            .filter(|end| *end != id)
            .collect::<std::collections::BTreeSet<NodeId>>()
    };
    let (a_ends, b_ends) = (far_ends(a), far_ends(b));
    if a_ends != b_ends {
        return false;
    }
    for end in a_ends {
        // A far end renders only its display identity (the name in input and
        // send rows), not its mixer state: muting node 0 must not rebuild
        // node 1 merely because node 1 shows node 0's name.
        let (Some(a_end), Some(b_end)) = (a.node(end), b.node(end)) else {
            return false;
        };
        if a_end.label != b_end.label
            || a_end.source != b_end.source
            || a_end.output_endpoint_id != b_end.output_endpoint_id
        {
            return false;
        }
        match (a.graph.get_node(end), b.graph.get_node(end)) {
            (Ok(x), Ok(y)) if x.kind() == y.kind() => {}
            _ => return false,
        }
    }
    a.exports == b.exports
}

pub(in crate::ui::window) type StripPanelHost = PanelHost<StripPanel>;

impl CachedPanel for StripPanel {
    type Owner = StudioShell;
    type Key = StripKey;

    fn key(&self, shell: &StudioShell, cx: &App) -> Self::Key {
        Self::key_for(self.id, shell, cx)
    }

    fn render(
        &self,
        shell: &mut StudioShell,
        _window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        let state = shell.state.read(cx);
        let graph = &state.snapshot.graph;
        let Some(entry) = graph.node(self.id) else {
            return div().into_any_element();
        };
        let Some(node) = graph.graph.get_node(self.id).ok() else {
            return div().into_any_element();
        };
        super::strip(shell, entry, node.kind(), node.channels(), cx).into_any_element()
    }

    fn on_rebuild(&self, shell: &mut StudioShell) {
        // The accumulating geometry frame must exist before this strip's
        // canvases extend it at prepaint — `update` is a deliberate no-op on
        // an empty slot. Only create it when missing: a whole-table reset
        // here would wipe the registrations of siblings that are not
        // rebuilding. This strip's own stale placements are removed so the
        // canvases re-register fresh during this same layout pass.
        if shell.mixer_geometry.read().is_none() {
            shell.mixer_geometry.reset();
        }
        shell.mixer_geometry.update(|geometry| {
            geometry.tracks.remove(&self.id);
            geometry.readouts.remove(&self.id);
        });
        shell.node_meter_points.borrow_mut().remove(&self.id);
    }

    fn observe_nested(
        &self,
        shell: &Entity<StudioShell>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // Every model entity nested in THIS strip: its fader, its readout
        // and balance editors, and the send sliders and editors of its own
        // out-edges. Their notifications cannot invalidate this cached
        // ancestor by themselves.
        let (sliders, inputs) = shell.read_with(cx, |shell, _| {
            let mut sliders = Vec::new();
            let mut inputs = Vec::new();
            if let Some(slot) = shell.mixer.faders.get(&self.id) {
                sliders.push(slot.state.clone());
            }
            if let Some(input) = shell.mixer.value_inputs.get(&self.id) {
                inputs.push(input.clone());
            }
            if let Some(input) = shell.mixer.balance_inputs.get(&self.id) {
                inputs.push(input.clone());
            }
            for edge in shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .edges()
                .iter()
                .filter(|edge| edge.src() == self.id)
            {
                if let Some(slot) = shell.mixer.sends.get(&edge.id()) {
                    sliders.push(slot.state.clone());
                }
                if let Some(input) = shell.mixer.send_inputs.get(&edge.id()) {
                    inputs.push(input.clone());
                }
            }
            (sliders, inputs)
        });
        let mut subscriptions: Vec<Subscription> = sliders
            .into_iter()
            .map(|slider| cx.observe(&slider, |host, _, cx| host.notify_nested(cx)))
            .collect();
        subscriptions.extend(
            inputs
                .into_iter()
                .map(|input| cx.observe(&input, |host, _, cx| host.notify_nested(cx))),
        );
        subscriptions
    }
}

/// Whether the strip carries the session's legacy source/output probe: the
/// observed source or sink matching the active desired spec. Shared by the
/// strip render and the hot layer's legacy fallback so the two cannot drift.
pub(in crate::ui::window) fn meter_probe_eligible(
    shell: &StudioShell,
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    kind: NodeKind,
) -> bool {
    let input = kind == NodeKind::Source;
    matches!(kind, NodeKind::Source | NodeKind::Sink)
        && shell
            .observation_nodes
            .is_some_and(|(generation, source, sink)| {
                generation == snapshot.generation && entry.id == if input { source } else { sink }
            })
        && snapshot.desired.as_ref().is_some_and(|active| {
            if input {
                entry.source.as_ref() == Some(&active.source)
            } else {
                entry.output_endpoint_id.as_ref() == Some(&active.output_endpoint_id)
            }
        })
}

/// Snapshot-only probe candidacy for the strip key: the session runs and the
/// node matches the desired spec's source/output. The key must never depend
/// on render-ordered shell state (`observation_nodes` is refreshed when the
/// shell renders) because keys are captured at notify time, before that
/// refresh — a probe transition would pin a stale key until the next notify.
/// The canvas MOUNT keeps the full [`meter_probe_eligible`] predicate, which
/// runs at render where the observation is fresh.
fn strip_probe_candidate(
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    kind: NodeKind,
) -> bool {
    let input = kind == NodeKind::Source;
    snapshot.phase == SessionPhase::Running
        && matches!(kind, NodeKind::Source | NodeKind::Sink)
        && snapshot.desired.as_ref().is_some_and(|active| {
            if input {
                entry.source.as_ref() == Some(&active.source)
            } else {
                entry.output_endpoint_id.as_ref() == Some(&active.output_endpoint_id)
            }
        })
}

/// The Mixer page: a light container (rebuilt per shell render) splicing the
/// per-strip cached panels, with the meter hot layer as a sibling above them.
pub(in crate::ui::window) fn page(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let graph = &shell.state.read(cx).snapshot.graph;
    let mut channels = div()
        .id("mixer-strips")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .track_scroll(&shell.mixer_scroll)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    let mut dock = div().flex().flex_none().min_w_full().h_full().justify_end();
    let dock_container = div()
        .id("mixer-dock")
        .flex()
        .flex_none()
        .w(px(MixerPresentation::DOCK_WIDTH))
        .max_w(relative(0.55))
        .min_h_0()
        .border_l_1()
        .border_color(theme::BORDER_STRONG)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    for id in &shell.mixer.order {
        let Ok(node) = graph.graph.get_node(*id) else {
            continue;
        };
        let Some(host) = shell.mixer.strips.get(id) else {
            continue;
        };
        // The strip is cached behind a wrapper carrying its load-bearing
        // outer sizing (cached views lay out by their wrapper).
        let strip = cached_or_live(
            host.clone(),
            window,
            {
                let width = px(MixerPresentation::STRIP_WIDTH);
                move |entity| {
                    entity
                        .cached(
                            StyleRefinement::default()
                                .flex_none()
                                .w(width)
                                .h_full()
                                .min_h_0(),
                        )
                        .into_any_element()
                }
            },
            |entity| entity.into_any_element(),
        );
        if matches!(node.kind(), NodeKind::Bus | NodeKind::Sink) {
            dock = dock.child(strip);
        } else {
            channels = channels.child(strip);
        }
    }
    div()
        .id("mixer-page")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(channels.gpui_test_observe())
                .child(dock_container.child(dock).gpui_test_observe()),
        )
        .child(shell.mixer_frame.clone())
        .gpui_test_observe()
        .into_any_element()
}

pub(in crate::ui::window) fn track(
    shell: &StudioShell,
    id: NodeId,
    channels: usize,
    legacy_input: Option<bool>,
) -> Div {
    let geometry = shell.mixer_geometry.clone();
    div()
        .h_full()
        .flex_none()
        .w(px(crate::ui::widgets::meters::channel_meter_width(
            channels,
        )))
        .child(
            canvas(
                move |bounds, window, _| {
                    geometry.update(|geometry| {
                        geometry.tracks.insert(
                            id,
                            (
                                Placement {
                                    bounds,
                                    mask: window.content_mask(),
                                },
                                channels,
                                legacy_input,
                            ),
                        );
                    });
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
}

pub(in crate::ui::window) fn readout(shell: &StudioShell, id: NodeId) -> impl IntoElement {
    let geometry = shell.mixer_geometry.clone();
    canvas(
        move |bounds, window, _| {
            geometry.update(|geometry| {
                geometry.readouts.insert(
                    id,
                    Placement {
                        bounds,
                        mask: window.content_mask(),
                    },
                );
            });
        },
        |_, _, _, _| {},
    )
    .size_full()
}

/// The Mixer page's hot surface: live bars and peak readouts over the
/// cached strip tree, painting from the geometry the strips registered and
/// the observations of the current frame.
pub(in crate::ui::window) struct MixerMeters {
    shell: WeakEntity<StudioShell>,
    geometry: Geometry,
    /// One shaped peak readout per track, re-shaped only when its text
    /// changes. The readout holds its value most of the time (peak hold),
    /// and shaping every track's text every frame dominated the page's
    /// per-frame budget at display refresh rates.
    readout_cache: std::cell::RefCell<BTreeMap<NodeId, (String, Option<gpui_kit::ShapedLine>)>>,
}

impl MixerMeters {
    pub(in crate::ui::window) fn new(shell: WeakEntity<StudioShell>, geometry: Geometry) -> Self {
        Self {
            shell,
            geometry,
            readout_cache: std::cell::RefCell::new(BTreeMap::new()),
        }
    }
}

pub(in crate::ui::window) type MixerMeterLayer = HotLayerView<MixerMeters>;

impl HotSurface for MixerMeters {
    type Frame = MeterGeometry;

    fn pump(&mut self, _window: &mut Window, cx: &mut App) -> bool {
        let Some(shell) = self.shell.upgrade() else {
            return false;
        };
        shell.update(cx, |shell, cx| {
            #[cfg(feature = "gpui-test")]
            shell.mixer.trace.borrow_mut().begin(true);
            #[cfg(feature = "gpui-test")]
            let began = std::time::Instant::now();
            shell.pump_meters(cx);
            #[cfg(feature = "gpui-test")]
            shell.mixer.trace.borrow_mut().add(began.elapsed());
            shell.frames_wanted(cx)
        })
    }

    fn frame(&self) -> &Geometry {
        &self.geometry
    }

    fn paint(&self, frame: &MeterGeometry, window: &mut Window, cx: &mut App) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            paint_mixer_meter_frame(frame, shell, window, cx, &self.readout_cache)
        });
    }
}

fn paint_mixer_meter_frame(
    frame: &MeterGeometry,
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
    readout_cache: &std::cell::RefCell<BTreeMap<NodeId, (String, Option<gpui_kit::ShapedLine>)>>,
) {
    #[cfg(feature = "gpui-test")]
    let began = std::time::Instant::now();
    #[cfg(feature = "gpui-test")]
    let mut levels_fingerprint = 0u64;
    for (&id, &(placement, channels, legacy_input)) in &frame.tracks {
        if !placement.decorations_visible() {
            continue;
        }
        let legacy = legacy_input.and_then(|input| {
            if input {
                shell.meters.display_input()
            } else {
                shell.meters.display_output()
            }
        });
        let levels = {
            let state = shell.state.read(cx);
            state.node_meters.levels(id).or(legacy)
        };
        #[cfg(feature = "gpui-test")]
        for level in levels.into_iter().flatten() {
            levels_fingerprint = levels_fingerprint.rotate_left(7)
                ^ u64::from(level.bar.to_bits())
                ^ (u64::from(level.rms.to_bits()) << 32)
                ^ u64::from(level.marker.to_bits());
        }
        let clips = shell.state.read(cx).node_meters.clips(id);
        window.with_content_mask(Some(placement.mask), |window| {
            crate::ui::widgets::meters::paint_channel_meter(
                placement.bounds,
                levels.as_ref().map(|levels| levels.as_slice()),
                channels,
                &clips,
                window,
                cx,
            );
        });
    }
    for (&id, &placement) in &frame.readouts {
        let peak = {
            let state = shell.state.read(cx);
            state
                .node_meters
                .raw_levels(id)
                .map(|levels| levels.iter().map(|level| level.peak).fold(0f32, f32::max))
        };
        let text = peak
            .map(|peak| {
                if peak == 0. {
                    "\u{2212}\u{221e}".to_owned()
                } else {
                    format!("{:.1}", 20. * peak.log10())
                }
            })
            .unwrap_or_else(|| "\u{2014}".to_owned());
        window.with_content_mask(Some(placement.mask), |window| {
            // Re-shape only when the text changed: the cache holds one entry
            // per track, so during peak hold every frame replays the shaped
            // line instead of re-running the shaper.
            let mut cache = readout_cache.borrow_mut();
            let entry = cache.entry(id).or_default();
            if entry.0 != text {
                entry.0 = text.clone();
                entry.1 = Some(window.text_system().shape_line(
                    text.into(),
                    px(8.),
                    &[TextRun {
                        len: entry.0.len(),
                        font: font(theme::FONT_MONO),
                        color: theme::TEXT_MUTED,
                        ..Default::default()
                    }],
                    None,
                ));
            }
            if let Some(line) = &entry.1 {
                let _ = line.paint(
                    placement.bounds.origin,
                    px(12.),
                    TextAlign::Center,
                    Some(placement.bounds.size.width),
                    window,
                    cx,
                );
            }
        });
    }
    #[cfg(feature = "gpui-test")]
    {
        let mut trace = shell.mixer.trace.borrow_mut();
        trace.levels(levels_fingerprint);
        trace.meter_paints += 1;
        trace.add(began.elapsed());
        trace.finish(shell.node_meter_points.borrow().len());
    }
}

#[cfg(test)]
mod tests {
    use super::Placement;
    use gpui_kit::{Bounds, ContentMask, point, px, size};

    #[test]
    fn vertical_culling_keeps_clip_and_channel_label_extents() {
        let placement = Placement {
            bounds: Bounds::new(point(px(0.), px(100.)), size(px(23.), px(160.))),
            mask: ContentMask {
                bounds: Bounds::new(point(px(0.), px(89.)), size(px(23.), px(5.))),
            },
        };
        assert!(
            placement.decorations_visible(),
            "clip lamp is visible above track"
        );
        assert!(
            Placement {
                mask: ContentMask {
                    bounds: Bounds::new(point(px(0.), px(266.)), size(px(23.), px(5.)))
                },
                ..placement
            }
            .decorations_visible(),
            "channel labels remain visible below track"
        );
        assert!(
            !Placement {
                mask: ContentMask {
                    bounds: Bounds::new(point(px(50.), px(100.)), size(px(23.), px(160.)))
                },
                ..placement
            }
            .decorations_visible()
        );
    }
}
