//! Cache controls separately from the display-rate meter canvas. The canvas
//! is a sibling, not a descendant: GPUI invalidates ancestors of an animated
//! view, which would otherwise invalidate the controls' prepaint/paint cache.
use super::*;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

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

#[derive(Default)]
struct MeterGeometry {
    tracks: BTreeMap<NodeId, (Placement, usize, Option<bool>)>,
    readouts: BTreeMap<NodeId, Placement>,
}

#[derive(Clone, Default)]
pub(in crate::ui::window) struct Geometry(Rc<RefCell<MeterGeometry>>);

#[cfg(all(test, feature = "gpui-test"))]
impl Geometry {
    pub(super) fn track_bounds(&self, id: NodeId) -> Option<Bounds<Pixels>> {
        self.0
            .borrow()
            .tracks
            .get(&id)
            .map(|(placement, _, _)| placement.bounds)
    }
}

pub(in crate::ui::window) struct MixerControls {
    shell: WeakEntity<StudioShell>,
    geometry: Geometry,
    key: Option<ControlsKey>,
    slider_subscriptions: std::collections::HashMap<EntityId, Subscription>,
    _subscription: Subscription,
}

/// Only data consumed by the control tree belongs in this key. In particular,
/// elapsed time and audio observations must not turn the 33 ms service poll
/// into another full-strip render loop. Child input/hover/scroll notifications
/// still invalidate their actual GPUI ancestor normally.
#[derive(PartialEq)]
struct ControlsKey {
    graph: std::sync::Arc<crate::service::GraphSnapshot>,
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
            graph: snapshot.graph.clone(),
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

impl MixerControls {
    pub fn new(shell: Entity<StudioShell>, geometry: Geometry, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&shell, |controls, shell, cx| {
            let key = ControlsKey::new(shell.read(cx), cx);
            if controls.key.as_ref() != Some(&key) {
                controls.key = Some(key);
                cx.notify();
            }
        });
        Self {
            shell: shell.downgrade(),
            geometry,
            key: None,
            slider_subscriptions: Default::default(),
            _subscription: subscription,
        }
    }
}

impl Render for MixerControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(shell) = self.shell.upgrade() {
            self.key = Some(ControlsKey::new(shell.read(cx), cx));
        }
        *self.geometry.0.borrow_mut() = MeterGeometry::default();
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
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
        self.slider_subscriptions
            .retain(|id, _| sliders.iter().any(|slider| slider.entity_id() == *id));
        for slider in sliders {
            self.slider_subscriptions
                .entry(slider.entity_id())
                .or_insert_with(|| cx.observe(&slider, |_, _, cx| cx.notify()));
        }
        shell.update(cx, |shell, cx| {
            shell.meter_points.set((false, false));
            shell.node_meter_points.borrow_mut().clear();
            super::page(shell, cx)
        })
    }
}

pub(in crate::ui::window) fn page(shell: &StudioShell) -> AnyElement {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(
            shell
                .mixer_controls
                .clone()
                .cached(StyleRefinement::default().size_full()),
        )
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
                    geometry.0.borrow_mut().tracks.insert(
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
            geometry.0.borrow_mut().readouts.insert(
                id,
                Placement {
                    bounds,
                    mask: window.content_mask(),
                },
            );
        },
        |_, _, _, _| {},
    )
    .size_full()
}

pub(in crate::ui::window) struct MixerMeterFrame {
    shell: WeakEntity<StudioShell>,
    geometry: Geometry,
}

impl MixerMeterFrame {
    pub fn new(shell: WeakEntity<StudioShell>, geometry: Geometry) -> Self {
        Self { shell, geometry }
    }
}

impl Render for MixerMeterFrame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let shell = self.shell.clone();
        let geometry = self.geometry.clone();
        // No hitbox: faders, clip reset, tooltips and context menus continue
        // to use the unchanged controls underneath this painting-only layer.
        let pump_shell = self.shell.clone();
        canvas(
            move |_, window, cx| {
                let _ = pump_shell.update(cx, |shell, cx| {
                    #[cfg(feature = "gpui-test")]
                    shell.mixer.trace.borrow_mut().begin(true);
                    #[cfg(feature = "gpui-test")]
                    let began = std::time::Instant::now();
                    shell.pump_meters(cx);
                    // Motion, not visibility, sustains the loop: at rest the
                    // control-side poll keeps the strips current and re-arms
                    // the frames once fresh data moves a layer.
                    if shell.meters_frame_active(cx) && shell.meters_in_motion() {
                        window.request_animation_frame();
                    }
                    #[cfg(feature = "gpui-test")]
                    shell.mixer.trace.borrow_mut().add(began.elapsed());
                });
            },
            move |_, _, window, cx| {
                let Some(shell) = shell.upgrade() else { return };
                shell.update(cx, |shell, cx| {
                    #[cfg(feature = "gpui-test")]
                    let began = std::time::Instant::now();
                    #[cfg(feature = "gpui-test")]
                    let mut levels_fingerprint = 0u64;
                    let geometry = geometry.0.borrow();
                    for (&id, &(placement, channels, legacy_input)) in &geometry.tracks {
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
                        let levels = shell
                            .node_meters
                            .channel_levels(id)
                            .or_else(|| legacy.as_ref().map(|levels| levels.as_slice()));
                        #[cfg(feature = "gpui-test")]
                        for level in levels.into_iter().flatten() {
                            levels_fingerprint = levels_fingerprint.rotate_left(7)
                                ^ u64::from(level.bar.to_bits())
                                ^ (u64::from(level.rms.to_bits()) << 32)
                                ^ u64::from(level.marker.to_bits());
                        }
                        window.with_content_mask(Some(placement.mask), |window| {
                            crate::ui::widgets::meters::paint_channel_meter(
                                placement.bounds,
                                levels,
                                channels,
                                shell.node_meters.channel_clips(id),
                                window,
                                cx,
                            );
                        });
                    }
                    for (&id, &placement) in &geometry.readouts {
                        let peak = shell.node_meters.raw_levels(id).map(|levels| {
                            levels.iter().map(|level| level.peak).fold(0f32, f32::max)
                        });
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
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
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
