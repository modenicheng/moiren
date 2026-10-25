use super::*;
use crate::service::GraphSnapshot;
use rust_i18n::t;
use std::collections::BTreeMap;

pub(in crate::ui::window) struct MixerPresentation {
    pub order: Vec<NodeId>,
    pub width: f32,
    pub dock_width: f32,
    pub resize: Option<(bool, f32, f32)>,
    pub faders: BTreeMap<NodeId, Entity<base::slider::SliderState>>,
    pub sends: BTreeMap<EdgeId, Entity<base::slider::SliderState>>,
}

impl Default for MixerPresentation {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            width: 176.,
            dock_width: 360.,
            resize: None,
            faders: BTreeMap::new(),
            sends: BTreeMap::new(),
        }
    }
}

pub(in crate::ui::window) fn gain_db(gain: f64) -> f32 {
    if gain <= 0. {
        -60.
    } else {
        (20. * gain.log10()).clamp(-60., 24.) as f32
    }
}

pub(in crate::ui::window) fn linear_gain(value: f32) -> f64 {
    if value <= -60. {
        0.
    } else {
        10f64.powf(f64::from(value) / 20.)
    }
}

/// Builds the real-time edit for a slider's new value from the current
/// snapshot; None when the target vanished since the slider was created.
pub(in crate::ui::window) type SliderEdit = Box<dyn Fn(f32, &GraphSnapshot) -> Option<AppRequest>>;

/// Slider construction and value sync shared by the fader (node) and send
/// (edge) columns; only the current-value lookup and edit construction
/// differ. With `release_applies` a slider release additionally applies the
/// candidate graph, so quick edits (tray flyout) take effect immediately.
pub(in crate::ui::window) fn sync_sliders<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, Entity<base::slider::SliderState>>,
    rows: Vec<(K, f32, SliderEdit)>,
    release_applies: bool,
) {
    for (id, value, edit) in rows {
        let slider = sliders.entry(id).or_insert_with(|| {
            let slider = cx.new(|_| {
                base::slider::SliderState::new()
                    .min(-60.)
                    .max(24.)
                    .step(0.1)
                    .default_value(value)
            });
            let state = state.clone();
            cx.subscribe(
                &slider,
                move |_, _, event: &base::slider::SliderEvent, cx| {
                    match event {
                        base::slider::SliderEvent::Change(value) => {
                            state.update(cx, |state, _| {
                                if let Some(edit) = edit(value.end(), &state.snapshot.graph) {
                                    state.dispatch(edit);
                                }
                            });
                            cx.notify();
                        }
                        base::slider::SliderEvent::Release(_) if release_applies => {
                            // The local snapshot may not have polled the edits
                            // yet, so this mirrors the Apply button: fire
                            // whenever no apply is already in flight.
                            state.update(cx, |state, _| {
                                if state.snapshot.graph.pending_revision.is_none()
                                    && !state.pending_request
                                {
                                    state.dispatch(AppRequest::ApplyGraph);
                                }
                            });
                            cx.notify();
                        }
                        _ => {}
                    }
                },
            )
            .detach();
            slider
        });
        if slider.read(cx).value().end() != value {
            slider.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_mixer_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.mixer.order.retain(|id| graph.node(*id).is_some());
        self.mixer.faders.retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .sends
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        for node in &graph.nodes {
            if !self.mixer.order.contains(&node.id) {
                self.mixer.order.push(node.id);
            }
        }
        let faders = graph
            .nodes
            .iter()
            .map(|node| {
                let id = node.id;
                (
                    id,
                    gain_db(node.mixer.gain),
                    Box::new(move |value: f32, graph: &GraphSnapshot| {
                        graph.node(id).map(|node| {
                            AppRequest::EditGraph(GraphEdit::SetMixer {
                                node: id,
                                state: MixerState {
                                    gain: linear_gain(value),
                                    ..node.mixer
                                },
                            })
                        })
                    }) as SliderEdit,
                )
            })
            .collect();
        sync_sliders(
            window,
            cx,
            &self.state,
            &mut self.mixer.faders,
            faders,
            false,
        );
        let sends = graph
            .graph
            .edges()
            .iter()
            .map(|edge| {
                let id = edge.id();
                (
                    id,
                    gain_db(edge.params().gain),
                    Box::new(move |value: f32, graph: &GraphSnapshot| {
                        graph.graph.get_edge(id).ok().map(|edge| {
                            AppRequest::EditGraph(GraphEdit::SetSend {
                                edge: id,
                                params: SendParams {
                                    gain: linear_gain(value),
                                    ..*edge.params()
                                },
                            })
                        })
                    }) as SliderEdit,
                )
            })
            .collect();
        sync_sliders(window, cx, &self.state, &mut self.mixer.sends, sends, false);
    }
}

pub(super) fn reorder(order: &mut Vec<NodeId>, dragged: NodeId, target: NodeId) {
    if dragged == target || !order.contains(&dragged) || !order.contains(&target) {
        return;
    }
    order.retain(|id| *id != dragged);
    let index = order
        .iter()
        .position(|id| *id == target)
        .expect("existing reorder target");
    order.insert(index, dragged);
}

pub(super) struct StripDrag;
impl Render for StripDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(12.))
            .bg(theme::SURFACE_3)
            .text_color(theme::TEXT)
            .child(t!("Move channel"))
    }
}
