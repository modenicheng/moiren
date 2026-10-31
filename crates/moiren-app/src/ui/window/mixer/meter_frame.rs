//! Cached controls behind a [`PanelHost`] and the display-rate meter layer
//! behind a [`HotLayerView`]: the meter canvas is a sibling, never a
//! descendant of the cached subtree — gpui-fast replays a cached panel's
//! paint without re-running its closures, so live bars must be painted by
//! the hot layer at the frame the panel's prepaint registered geometry for.
use super::*;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::window::hot::{HotLayerView, HotSurface};
use crate::ui::window::panel::{ArcSnapshot, CachedPanel, PanelHost, cached_or_live};
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

/// The Mixer page panel behind [`PanelHost`]: the strip tree, rebuilt only
/// when its semantic key changes.
pub(in crate::ui::window) struct MixerPanel;

/// Only data consumed by the control tree belongs in this key. In particular,
/// elapsed time and audio observations must not turn the 33 ms service poll
/// into another full-strip render loop. Child input/hover/scroll notifications
/// still invalidate their actual GPUI ancestor normally.
#[derive(PartialEq)]
pub(in crate::ui::window) struct ControlsKey {
    graph: ArcSnapshot<crate::service::GraphSnapshot>,
    catalog: std::sync::Arc<crate::service::DeviceCatalog>,
    backend: crate::service::AudioBackend,
    generation: crate::service::SessionGeneration,
    phase: SessionPhase,
    desired: Option<crate::service::SessionSpec>,
    topics: Vec<(NodeId, moiren_signal::TopicId)>,
    preferences: crate::project::AppPreferences,
    order: Vec<NodeId>,
    selected: Option<NodeId>,
    editing_value: Option<NodeId>,
    editing_balance: Option<NodeId>,
    editing_node_gain: Option<NodeId>,
    editing_node_pan: Option<NodeId>,
    editing_send: Option<EdgeId>,
    collapsed: (bool, bool),
    scroll: Point<Pixels>,
    compact: bool,
    page: Page,
}

impl ControlsKey {
    fn new(shell: &StudioShell, cx: &App) -> Self {
        let snapshot = &shell.state.read(cx).snapshot;
        Self {
            graph: ArcSnapshot::new(snapshot.graph.clone()),
            catalog: snapshot.catalog.clone(),
            backend: snapshot.audio_settings.backend,
            generation: snapshot.generation,
            phase: snapshot.phase,
            desired: snapshot.desired.clone(),
            topics: snapshot
                .signal_topics
                .node_levels
                .iter()
                .map(|(id, topic)| (*id, topic.id()))
                .collect(),
            preferences: shell.preferences.clone(),
            order: shell.mixer.order.clone(),
            selected: shell.selected_node,
            editing_value: shell.mixer.editing_value,
            editing_balance: shell.mixer.editing_balance,
            editing_node_gain: shell.mixer.editing_node_gain,
            editing_node_pan: shell.mixer.editing_node_pan,
            editing_send: shell.mixer.editing_send,
            collapsed: (shell.mixer.inputs_collapsed, shell.mixer.sends_collapsed),
            scroll: shell.mixer_scroll.offset(),
            compact: shell.compact,
            page: shell.presentation.page,
        }
    }
}

pub(in crate::ui::window) type MixerPanelHost = PanelHost<MixerPanel>;

impl CachedPanel for MixerPanel {
    type Owner = StudioShell;
    type Key = ControlsKey;

    fn key(shell: &StudioShell, cx: &App) -> Self::Key {
        ControlsKey::new(shell, cx)
    }

    fn render(
        shell: &mut StudioShell,
        _window: &mut Window,
        cx: &mut Context<StudioShell>,
    ) -> AnyElement {
        super::page(shell, cx)
    }

    fn on_rebuild(shell: &mut StudioShell) {
        // Geometry is registered by the strip canvases during this same
        // layout pass; stale placements must not survive a rebuild.
        shell.mixer_geometry.reset();
        // Fresh visibility demand: the strip canvases re-register the meters
        // their content mask shows during this same layout pass.
        shell.meter_points.set((false, false));
        shell.node_meter_points.borrow_mut().clear();
    }

    fn observe_nested(
        shell: &Entity<StudioShell>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // SliderState is a model entity, not a Render view in the dispatch
        // tree. Its notifications cannot invalidate this cached ancestor by
        // themselves. Observe both faders and sends, including drag updates
        // that intentionally precede the service's confirmed snapshot.
        let sliders = shell.read_with(cx, |shell, _| {
            shell
                .mixer
                .faders
                .values()
                .chain(shell.mixer.sends.values())
                .map(|slot| slot.state.clone())
                .collect::<Vec<_>>()
        });
        sliders
            .into_iter()
            .map(|slider| cx.observe(&slider, |host, _, cx| host.notify_nested(cx)))
            .collect()
    }
}

pub(in crate::ui::window) fn page(shell: &StudioShell, window: &Window) -> AnyElement {
    let controls = cached_or_live(
        shell.mixer_controls.clone(),
        window,
        |entity| {
            entity
                .cached(StyleRefinement::default().size_full())
                .into_any_element()
        },
        |entity| div().size_full().child(entity).into_any_element(),
    );
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(controls)
        .child(shell.mixer_frame.clone())
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
}

impl MixerMeters {
    pub(in crate::ui::window) fn new(shell: WeakEntity<StudioShell>, geometry: Geometry) -> Self {
        Self { shell, geometry }
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
            paint_mixer_meter_frame(frame, shell, window, cx)
        });
    }
}

fn paint_mixer_meter_frame(
    frame: &MeterGeometry,
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
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
            let line = window.text_system().shape_line(
                text.clone().into(),
                px(8.),
                &[TextRun {
                    len: text.len(),
                    font: font(theme::FONT_MONO),
                    color: theme::TEXT_MUTED,
                    ..Default::default()
                }],
                None,
            );
            let _ = line.paint(
                placement.bounds.origin,
                px(12.),
                TextAlign::Center,
                Some(placement.bounds.size.width),
                window,
                cx,
            );
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
