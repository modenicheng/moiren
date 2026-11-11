//! The Mixer page assembly: strip panels with their key projection, the
//! track/readout meters, and the display-rate meter layer. Cached controls
//! sit behind a [`PanelHost`] and the meter layer behind a
//! [`HotLayerView`]: the meter canvas is a sibling, never a descendant of
//! the cached subtree — gpui-fast replays a cached panel's paint without
//! re-running its closures, so live bars must be painted by the hot layer
//! at the frame the panel's prepaint registered geometry for.
use super::*;
use crate::ui::monitoring::DisplayLevel;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::shell::hot::{HotLayerView, HotSurface};
use crate::ui::shell::panel::{CachedPanel, PanelHost};
use crate::ui::widgets::overlay_scrollbar::{notify_owner, scroll_region};
use gpui_kit::base::ScrollbarAxis;
use std::collections::{BTreeMap, HashMap};

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
pub(crate) struct MeterGeometry {
    tracks: BTreeMap<NodeId, (Placement, usize, Option<bool>)>,
    readouts: BTreeMap<NodeId, Placement>,
    /// The strip rect the hovered fault banner hangs from. No clip rides
    /// along: the banner may reach below the scroll viewport's edge, so it
    /// clamps against the hot layer's own bounds instead.
    banners: BTreeMap<NodeId, Bounds<Pixels>>,
}

/// Where the strip's meter tracks and readouts sit, registered by the
/// cached strip's canvases during a rebuild and read by the hot layer at
/// paint time. Stale by design between rebuilds.
pub(crate) type Geometry = SharedFrame<MeterGeometry>;

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
pub(crate) struct StripPanel {
    pub(crate) id: NodeId,
}

/// Only the cached body's inputs, projected: derived booleans over the
/// snapshot (`managed`, `probe`) instead of the snapshot fields behind them,
/// so a key capture stays allocation-free apart from the graph Arc. The
/// routing rows render live outside this cache, so edges, the collapse
/// flags, the send editor and the strip selection (which styles the live
/// column) are not key inputs here. Elapsed time and audio observations must
/// not appear either.
pub(crate) struct StripKey {
    id: NodeId,
    graph: std::sync::Arc<crate::service::GraphSnapshot>,
    catalog: std::sync::Arc<crate::service::DeviceCatalog>,
    /// The node's level topic exists: the managed-meter visibility canvas is
    /// mounted and the hot layer expects a track.
    managed: bool,
    /// The strip is the session's observed source/sink: the legacy probe
    /// canvas claims one of the shared `meter_points` flags.
    probe: bool,
    compact: bool,
    position: usize,
    editing_value: bool,
    editing_balance: bool,
    /// Registered meter geometry goes stale under scroll just like under any
    /// other layout change, so the offset of the strip's own container rides
    /// in the key — the main strip row or the dock, per kind.
    scroll: Point<Pixels>,
    meters: crate::project::MeterDisplayPreferences,
    /// The shared `GraphSnapshot` projection says this strip's fault group
    /// is non-empty: the strip wears the danger frame, the wash and the "!"
    /// badge. Projected as one allocation-free bool so a fault landing or
    /// clearing rebuilds only this strip.
    faulted: bool,
}

impl PartialEq for StripKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.managed == other.managed
            && self.probe == other.probe
            && self.compact == other.compact
            && self.position == other.position
            && self.editing_value == other.editing_value
            && self.editing_balance == other.editing_balance
            && self.scroll == other.scroll
            && self.meters == other.meters
            && self.faulted == other.faulted
            && self.catalog == other.catalog
            && strip_content_eq(&self.graph, &other.graph, self.id)
    }
}

impl StripPanel {
    fn key_for(id: NodeId, shell: &StudioShell, cx: &App) -> StripKey {
        let state = shell.state.read(cx);
        let snapshot = &state.snapshot;
        let graph = &snapshot.graph;
        let node = graph.graph.get_node(id);
        let probe = node
            .ok()
            .zip(graph.node(id))
            .is_some_and(|(node, entry)| strip_probe_candidate(snapshot, entry, node.kind()));
        // Docked strips scroll with the dock pane, channel strips with the
        // main row: a stale container's offset would freeze the strip's
        // cached paint (scroll-under-cache) on the other container's scroll.
        let docked = node.is_ok_and(|node| matches!(node.kind(), NodeKind::Bus | NodeKind::Sink));
        StripKey {
            id,
            graph: snapshot.graph.clone(),
            catalog: snapshot.catalog.clone(),
            managed: snapshot.signal_topics.node_levels.contains_key(&id),
            probe,
            compact: shell.chrome.compact,
            position: shell
                .mixer
                .presentation
                .order
                .iter()
                .position(|node| *node == id)
                .unwrap_or(0),
            editing_value: shell.mixer.presentation.editing_value == Some(id),
            editing_balance: shell.mixer.presentation.editing_balance == Some(id),
            scroll: if docked {
                shell.mixer.dock_scroll.offset()
            } else {
                shell.mixer.scroll.offset()
            },
            meters: shell.preferences.values.meters,
            faulted: graph.node_faulted(id),
        }
    }
}

/// Whether a strip's cached body is identical between two snapshots: its own
/// node state, kind, channels, and the export intents that can name a
/// rendered node. The routing rows moved out of the cached body (they own
/// live scroll containers), so edges no longer render inside the cache and
/// an edge edit rebuilds no strip at all. Deliberately ignores session
/// bookkeeping — revisions, faults, undo flags — that no strip renders: a
/// whole-snapshot compare would drag every sibling strip into a rebuild on
/// every edit publish.
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
    // The kind and channel count style the body (color bar, meter tracks)
    // but live on the logical node, not the session state.
    match (a.graph.get_node(id), b.graph.get_node(id)) {
        (Ok(x), Ok(y)) => {
            if x.kind() != y.kind() || x.channels() != y.channels() {
                return false;
            }
        }
        _ => return false,
    }
    a.exports == b.exports
}

pub(crate) type StripPanelHost = PanelHost<StripPanel>;

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
        super::strip_body(shell, entry, node.kind(), node.channels(), cx).into_any_element()
    }

    fn on_rebuild(&self, shell: &mut StudioShell) {
        // The accumulating geometry frame must exist before this strip's
        // canvases extend it at prepaint — `update` is a deliberate no-op on
        // an empty slot. Only create it when missing: a whole-table reset
        // here would wipe the registrations of siblings that are not
        // rebuilding. This strip's own stale placements are removed so the
        // canvases re-register fresh during this same layout pass.
        if shell.mixer.geometry.read().is_none() {
            shell.mixer.geometry.reset();
        }
        shell.mixer.geometry.update(|geometry| {
            geometry.tracks.remove(&self.id);
            geometry.readouts.remove(&self.id);
            geometry.banners.remove(&self.id);
        });
        shell
            .metering
            .node_meter_points
            .borrow_mut()
            .remove(&self.id);
    }

    fn observe_nested(
        &self,
        shell: &Entity<StudioShell>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // The model entities nested in THIS strip's cached body: its fader
        // and its readout/balance editors. The send sliders and editors of
        // its out-edges render in the live routing sections, outside this
        // cache — their entities notify the shell directly, which re-renders
        // the sections without a rebuild here.
        let (sliders, inputs) = shell.read_with(cx, |shell, _| {
            let mut sliders = Vec::new();
            let mut inputs = Vec::new();
            if let Some(slot) = shell.mixer.presentation.faders.get(&self.id) {
                sliders.push(slot.state.clone());
            }
            if let Some(input) = shell.mixer.presentation.value_inputs.get(&self.id) {
                inputs.push(input.clone());
            }
            if let Some(input) = shell.mixer.presentation.balance_inputs.get(&self.id) {
                inputs.push(input.clone());
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
pub(crate) fn meter_probe_eligible(
    shell: &StudioShell,
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    kind: NodeKind,
) -> bool {
    let input = kind == NodeKind::Source;
    matches!(kind, NodeKind::Source | NodeKind::Sink)
        && shell
            .metering
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

/// How far the dock divider's invisible grab handle reaches to either side
/// of the 1px border it straddles.
const DOCK_RESIZE_REACH: f32 = 3.;

/// One strip's live column: the scrollable routing sections render above the
/// cached control stack, *outside* it. The rows are a handful of labels and
/// they own the sections' scroll containers, whose paint and hitboxes must
/// stay live — a scroll container inside a cached panel freezes its paint on
/// cache hits (the cache replays baked coordinates), and a cached scrollbar
/// thumb loses its drag target with them. The cached body replays beneath
/// the sections; the selection styles this live column, not the cache.
fn strip_column(
    shell: &StudioShell,
    id: NodeId,
    entry: &crate::service::GraphNodeState,
    kind: NodeKind,
    host: &Entity<StripPanelHost>,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let selected = shell.selection.node == Some(id);
    // The shared fault projection: the live column wears the danger frame,
    // the faint red wash and the banner anchor while the strip holds a
    // fault; the "!" badge rides the cached title row off the same
    // projection via `StripKey::faulted`.
    let faulted = shell.state.read(cx).snapshot.graph.node_faulted(id);
    let cached_body = {
        // The cached wrapper carries its load-bearing sizing: it takes the
        // column height the sections leave over (cached views lay out by
        // their wrapper). The wrapper only reserves that space — the body's
        // own root must claim it with definite extents, because a cached
        // view lays its content out as a layout root where `flex_1` has no
        // parent to grow against (see `strip_body`).
        host.clone()
            .cached(StyleRefinement::default().flex_1().min_h_0())
            .into_any_element()
    };
    let this = cx.entity();
    div()
        .id(SharedString::from(format!("strip-{id:?}")))
        .flex()
        .flex_none()
        .w(px(MixerPresentation::STRIP_WIDTH))
        .h_full()
        .min_h_0()
        .bg(if selected {
            theme::controls::RAISED
        } else {
            theme::SURFACE
        })
        .relative()
        .rounded_none()
        .border_l_1()
        .border_t_1()
        .border_b_1()
        .border_r_1()
        .border_color(if faulted {
            theme::DANGER
        } else {
            theme::BORDER
        })
        .on_hover({
            let this = this.clone();
            move |hovered, _, cx| {
                this.update(cx, |shell, cx| {
                    // Hover feeds the meter frame's fault banner alone, the
                    // graph's `hovered_node` twin: one notified frame per
                    // change, never a panel key.
                    let next = hovered.then_some(id);
                    if shell.mixer.hovered_strip != next {
                        shell.mixer.hovered_strip = next;
                        cx.notify();
                    }
                })
            }
        })
        .on_drop(move |dragged: &NodeId, _, cx| {
            this.update(cx, |shell, cx| {
                let graph = &shell.state.read(cx).snapshot.graph.graph;
                let docked = |id| {
                    graph
                        .get_node(id)
                        .is_ok_and(|node| matches!(node.kind(), NodeKind::Bus | NodeKind::Sink))
                };
                if docked(*dragged) == docked(id) {
                    model::reorder(&mut shell.mixer.presentation.order, *dragged, id);
                }
                cx.notify();
            })
        })
        .child({
            // The fault banner's anchor: the whole strip's window rect,
            // registered for the meter frame the same way meter tracks are.
            let geometry = shell.mixer.geometry.clone();
            canvas(
                move |bounds, _window, _| {
                    geometry.update(|geometry| {
                        geometry.banners.insert(id, bounds);
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
        })
        .when(faulted, |strip| {
            // The faint red wash over the strip's own surface: painted
            // above the background and below every control that follows.
            strip.child(
                div()
                    .id(SharedString::from(format!("strip-wash-{id:?}")))
                    .absolute()
                    .inset_0()
                    .bg(theme::DANGER_WASH)
                    .gpui_test_observe(),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .pt(px(theme::SPACE_1))
                .child(routing::io(shell, entry, cx))
                .child(cached_body),
        )
        .child(
            // The kind color rides the live column: an observed element
            // inside the cached body would report stale bounds on cache-hit
            // frames. Dimmed at ALPHA_DIM — the same strength the tray
            // strip's bar already used (design 21 §1.3 unifies the former
            // 0.8 here).
            div()
                .id(SharedString::from(format!("strip-color-{id:?}")))
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(4.))
                .bg(theme::dim(node_kind_color(kind)))
                .gpui_test_observe(),
        )
        .gpui_test_observe()
}

/// The dock divider's live resize gesture. Renders nothing: the drag exists
/// only so its `on_drag_move` keeps receiving pointer moves after the press
/// leaves the divider's hitbox.
struct DockResize;
impl Render for DockResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// The Mixer page: a light container (rebuilt per shell render) splicing the
/// per-strip cached panels, with the meter hot layer as a sibling above them.
pub(crate) fn page(shell: &StudioShell, window: &Window, cx: &Context<StudioShell>) -> AnyElement {
    let graph = &shell.state.read(cx).snapshot.graph;
    // The strip rows' scrollbar overlays are siblings of their scroll
    // containers, never children (see `scroll_region`); the containers are
    // built here and wrapped below. The outer sizing lives on the relative
    // wrapper.
    let mut channels_area = div()
        .id("mixer-strips")
        .w_full()
        .h_full()
        .flex()
        .track_scroll(&shell.mixer.scroll)
        .restrict_scroll_to_axis()
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel(notify_owner(cx.entity()));
    let mut dock = div().flex().flex_none().min_w_full().h_full().justify_end();
    let dock_container = div()
        .id("mixer-dock")
        .w_full()
        .h_full()
        .flex()
        .border_l_1()
        .border_color(theme::BORDER_STRONG)
        .track_scroll(&shell.mixer.dock_scroll)
        .restrict_scroll_to_axis()
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel(notify_owner(cx.entity()));
    for id in &shell.mixer.presentation.order {
        let Ok(node) = graph.graph.get_node(*id) else {
            continue;
        };
        let Some(entry) = graph.node(*id) else {
            continue;
        };
        let Some(host) = shell.mixer.presentation.strips.get(id) else {
            continue;
        };
        let column = strip_column(shell, *id, entry, node.kind(), host, cx);
        if matches!(node.kind(), NodeKind::Bus | NodeKind::Sink) {
            dock = dock.child(column);
        } else {
            channels_area = channels_area.child(column);
        }
    }
    let channels = scroll_region(
        "scrollbar-mixer-strips",
        &shell.mixer.scroll,
        ScrollbarAxis::Both,
        channels_area.gpui_test_observe(),
    )
    .flex_1()
    .min_w_0()
    .min_h_0();
    let dock_host = scroll_region(
        "scrollbar-mixer-dock",
        &shell.mixer.dock_scroll,
        ScrollbarAxis::Horizontal,
        dock_container.child(dock).gpui_test_observe(),
    )
    .size_full();
    // The dock pane owns the user-resizable width (and its 55% cap) so the
    // divider border stays the pane's left edge and the grab handle can
    // straddle it without the scroll container clipping the reach; the
    // container fills the pane, keeping its bounds — and every strip layout
    // inside — exactly where the fixed width put them.
    let row_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let measure_row = row_bounds.clone();
    let resize_shell = cx.entity();
    let dock_pane = div()
        .relative()
        .flex_none()
        .min_h_0()
        .w(px(shell.mixer.presentation.dock_width))
        .max_w(relative(MixerPresentation::DOCK_MAX_FRACTION))
        .child(dock_host)
        .child(
            div()
                .id("mixer-dock-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-DOCK_RESIZE_REACH))
                .w(px(DOCK_RESIZE_REACH * 2. + 1.))
                .cursor_ew_resize()
                .on_drag(DockResize, |_, _, _, cx| cx.new(|_| DockResize))
                .on_drag_move(move |event: &DragMoveEvent<DockResize>, _, cx| {
                    let row = row_bounds.get();
                    if row.size.width <= px(0.) {
                        return;
                    }
                    // The pane hugs the strip row's right edge, so the new
                    // width is that edge minus the pointer. The clamp mirrors
                    // the pane's max_w (and a one-strip floor) so the drawn
                    // border tracks the cursor instead of lagging the cap.
                    let width = f32::from(row.right() - event.event.position.x).clamp(
                        MixerPresentation::STRIP_WIDTH,
                        f32::from(row.size.width) * MixerPresentation::DOCK_MAX_FRACTION,
                    );
                    resize_shell.update(cx, |shell, cx| {
                        shell.mixer.presentation.dock_width = width;
                        cx.notify();
                    });
                })
                .gpui_test_observe(),
        );
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
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(
                    canvas(move |bounds, _, _| measure_row.set(bounds), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
                .child(channels)
                .child(dock_pane),
        )
        .child(shell.mixer.frame.clone())
        // The input dropdown hangs over the whole page (an occluding page
        // under it dismisses on outside press), painted above the hot layer.
        .when(shell.mixer.presentation.input_popover.is_some(), |page| {
            page.child(routing::input_popover_overlay(shell, window, cx))
        })
        .gpui_test_observe()
        .into_any_element()
}

pub(crate) fn track(
    shell: &StudioShell,
    id: NodeId,
    channels: usize,
    legacy_input: Option<bool>,
) -> Div {
    let geometry = shell.mixer.geometry.clone();
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

// --- The meter readout's reserved block (design 21 §1.5) --------------------
// The dB readout hanging under each strip's meter track is the UI's one
// layout-grade absolute positioning (design 21 §2), and its metrics live in
// this one group because two files consume them: the reserved block itself
// is styled in `super::meter` (pages/mixer.rs) while the hot layer below
// (`ReadoutShapes`, `paint_mixer_meter_frame`) shapes and paints the live
// text at the same size and line height — one group so the block and its
// painter cannot drift apart.

/// Left edge of the block this far left of the meter's left edge, so the
/// block's width centers the dB text under the meter track.
pub(crate) const READOUT_OFFSET_X: f32 = -10.;
/// Bottom edge of the block this far below the meter's bottom edge: the
/// number hangs under the track, inside the levels row's bottom margin
/// (see `strip_body` in pages/mixer.rs).
pub(crate) const READOUT_OFFSET_Y: f32 = -28.;
/// Reserved width for the widest dB text at the mono size.
pub(crate) const READOUT_WIDTH: f32 = 48.;
/// The block's height: exactly one line.
pub(crate) const READOUT_HEIGHT: f32 = 12.;
/// The single line's line height, shared by the block style and the hot
/// layer's `ShapedLine::paint`.
pub(crate) const READOUT_LINE_HEIGHT: f32 = 12.;
/// The mono text size the block style and the readout shaper share.
pub(crate) const READOUT_TEXT_SIZE: f32 = 8.;

pub(crate) fn readout(shell: &StudioShell, id: NodeId) -> impl IntoElement {
    let geometry = shell.mixer.geometry.clone();
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
pub(crate) struct MixerMeters {
    shell: WeakEntity<StudioShell>,
    geometry: Geometry,
    /// Shared, bounded shaping cache for every strip's readout, keyed by
    /// the exact text (see [`ReadoutShapes`]).
    readouts: std::cell::RefCell<ReadoutShapes>,
}

impl MixerMeters {
    pub(crate) fn new(shell: WeakEntity<StudioShell>, geometry: Geometry) -> Self {
        Self {
            shell,
            geometry,
            readouts: std::cell::RefCell::new(ReadoutShapes::default()),
        }
    }

    /// How many shaper runs the readout cache has needed, and how many
    /// distinct texts it holds. Every run inserts exactly one entry, so the
    /// pair pins "one shape per distinct text, everything else is a lookup".
    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn readout_shape_stats(&self) -> (usize, usize) {
        let shapes = self.readouts.borrow();
        (shapes.census.shapes, shapes.lines.len())
    }

    /// The text a strip's readout last painted; `None` before its first paint.
    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn last_readout(&self, id: NodeId) -> Option<String> {
        self.readouts.borrow().census.painted.get(&id).cloned()
    }
}

/// The strip readouts' shaping cache, shared by every track and keyed by the
/// exact text. Marker semantics make the readout's text change every frame
/// while a peak falls, so the third machine round's per-track cache (one
/// line per track, reshaped when its text changes) would have reshaped every
/// track every frame during a fall. Keying by text instead turns the fall
/// into one shape per distinct dB string and then lookups forever: at the
/// readout's 0.1 dB display precision the honest texts number in the
/// hundreds, and no per-node entry survives a deleted node, so the old
/// per-track map's unbounded growth is gone with it. The key is the text
/// alone because every shaping parameter at the only call site (mono font,
/// [`READOUT_TEXT_SIZE`], muted color) is a constant; if the readout ever
/// becomes styleable, those parameters must join the key.
#[derive(Default)]
struct ReadoutShapes {
    lines: HashMap<String, gpui_kit::ShapedLine>,
    /// Test census: real shaper runs and the text each readout last painted.
    #[cfg(feature = "gpui-test")]
    census: ReadoutCensus,
}

#[cfg(feature = "gpui-test")]
#[derive(Default)]
struct ReadoutCensus {
    shapes: usize,
    painted: BTreeMap<NodeId, String>,
}

/// Far above the honest text population (~0.1 dB steps over the meter
/// range); a pathological feed that exceeds it clears the cache and pays
/// reshapes again instead of growing without bound.
const READOUT_SHAPE_CACHE_CAP: usize = 2048;

impl ReadoutShapes {
    /// The shaped line for `text`, running the shaper only on a cache miss.
    fn line(&mut self, text: &str, window: &mut Window) -> Option<&gpui_kit::ShapedLine> {
        if !self.lines.contains_key(text) {
            if self.lines.len() >= READOUT_SHAPE_CACHE_CAP {
                self.lines.clear();
            }
            let line = window.text_system().shape_line(
                text.into(),
                px(READOUT_TEXT_SIZE),
                &[TextRun {
                    len: text.len(),
                    font: font(theme::FONT_MONO),
                    color: theme::TEXT_MUTED,
                    ..Default::default()
                }],
                None,
            );
            self.lines.insert(text.to_owned(), line);
            #[cfg(feature = "gpui-test")]
            {
                self.census.shapes += 1;
            }
        }
        self.lines.get(text)
    }
}

pub(crate) type MixerMeterLayer = HotLayerView<MixerMeters>;

impl HotSurface for MixerMeters {
    type Frame = MeterGeometry;

    fn pump(&mut self, _window: &mut Window, cx: &mut App) -> bool {
        let Some(shell) = self.shell.upgrade() else {
            return false;
        };
        shell.update(cx, |shell, cx| {
            #[cfg(feature = "gpui-test")]
            shell.trace.borrow_mut().begin(true);
            #[cfg(feature = "gpui-test")]
            let began = std::time::Instant::now();
            shell.pump_meters(cx);
            #[cfg(feature = "gpui-test")]
            shell.trace.borrow_mut().add(began.elapsed());
            shell.frames_wanted(cx)
        })
    }

    fn frame(&self) -> &Geometry {
        &self.geometry
    }

    fn paint(
        &self,
        frame: &MeterGeometry,
        layer: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            paint_mixer_meter_frame(frame, shell, layer, window, cx, &self.readouts)
        });
    }
}

/// One strip readout's text for a frame of display levels: the loudest
/// channel's held marker, spelled by the same
/// [`crate::ui::presentation::dbfs_text`] the inspector's "Peak" row uses,
/// so both surfaces read the same number at the same instant. The marker
/// falls after its hold, so the readout decays with the bar instead of
/// latching the retained raw peak, which only ever ratchets up and pins a
/// transient's maximum for the whole subscription. `None` — no observation
/// at all — stays an em dash.
fn readout_text(levels: Option<&[DisplayLevel]>) -> String {
    match levels {
        Some(levels) => crate::ui::presentation::dbfs_text(
            levels.iter().fold(0f32, |max, level| max.max(level.marker)),
        ),
        None => "\u{2014}".to_owned(),
    }
}

fn paint_mixer_meter_frame(
    frame: &MeterGeometry,
    shell: &mut StudioShell,
    layer: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
    readouts: &std::cell::RefCell<ReadoutShapes>,
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
                shell.metering.meters.display_input()
            } else {
                shell.metering.meters.display_output()
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
        // The readout rides the same levels its bars paint — the managed
        // node observation first, the legacy capture-tap fallback for probe
        // strips without a node topic — so the number can never disagree
        // with the marker drawn directly above it.
        let legacy = frame.tracks.get(&id).and_then(|&(_, _, legacy_input)| {
            legacy_input.and_then(|input| {
                if input {
                    shell.metering.meters.display_input()
                } else {
                    shell.metering.meters.display_output()
                }
            })
        });
        let levels = {
            let state = shell.state.read(cx);
            state.node_meters.levels(id).or(legacy)
        };
        let text = readout_text(levels.as_ref().map(|levels| levels.as_slice()));
        window.with_content_mask(Some(placement.mask), |window| {
            // Shape only when this exact text has never been shaped: during
            // a peak hold nothing here runs, and while markers fall the
            // shared text-keyed cache turns per-track reshapes into one
            // lookup per track (see [`ReadoutShapes`]).
            let mut shapes = readouts.borrow_mut();
            #[cfg(feature = "gpui-test")]
            shapes.census.painted.insert(id, text.clone());
            let Some(line) = shapes.line(&text, window) else {
                return;
            };
            let _ = line.paint(
                placement.bounds.origin,
                px(READOUT_LINE_HEIGHT),
                TextAlign::Center,
                Some(placement.bounds.size.width),
                window,
                cx,
            );
        });
    }
    #[cfg(feature = "gpui-test")]
    {
        let mut trace = shell.trace.borrow_mut();
        trace.levels(levels_fingerprint);
        trace.meter_paints += 1;
        trace.add(began.elapsed());
        trace.finish(shell.metering.node_meter_points.borrow().len());
    }
    // The hovered faulted strip's banner, hung under the strip and clamped
    // against the hot layer's own bounds. Like the graph page's tooltip it
    // lives on the shell outside every panel key: one notified frame per
    // hover change paints it, and it stands until the next change repaints.
    if let Some(id) = shell.mixer.hovered_strip
        && let Some(bounds) = frame.banners.get(&id)
    {
        let faults: Vec<crate::ui::pages::graph::graph_canvas::CardFault> = {
            let state = shell.state.read(cx);
            state
                .snapshot
                .graph
                .faults_for_node(id)
                .into_iter()
                .map(|fault| crate::ui::pages::graph::graph_canvas::CardFault {
                    cause: fault.cause,
                    hint: fault.hint,
                })
                .collect()
        };
        let mut shapes = shell.mixer.shapes.borrow_mut();
        crate::ui::pages::graph::graph_canvas_view::paint_fault_banner(
            &faults,
            *bounds,
            layer,
            crate::ui::pages::graph::graph_canvas_view::Paint {
                shapes: &mut shapes,
                window,
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Placement, readout_text};
    use crate::ui::monitoring::DisplayLevel;
    use crate::ui::presentation::dbfs_text;
    use gpui_kit::{Bounds, ContentMask, point, px, size};

    fn level(marker: f32) -> DisplayLevel {
        DisplayLevel {
            bar: 0.,
            marker,
            rms: 0.,
        }
    }

    #[test]
    fn strip_readout_text_follows_the_marker_the_inspector_shows() {
        assert_eq!(readout_text(None), "\u{2014}");
        // Silence and a full-scale transient are spelled exactly like the
        // inspector's "Peak" row at the same instant.
        assert_eq!(readout_text(Some(&[level(0.)])), dbfs_text(0.));
        assert_eq!(readout_text(Some(&[level(1.)])), dbfs_text(1.));
        // The loudest channel wins, at that shared formatting.
        assert_eq!(
            readout_text(Some(&[level(0.25), level(0.75)])),
            dbfs_text(0.75),
        );
        // A transient followed by silence walks the text back down with the
        // marker instead of latching the all-time maximum.
        let mut previous = readout_text(Some(&[level(1.)]));
        for marker in [0.5f32, 0.1] {
            let text = readout_text(Some(&[level(marker)]));
            assert_ne!(text, previous, "the readout must follow the marker down");
            assert_eq!(text, dbfs_text(marker));
            previous = text;
        }
    }

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
