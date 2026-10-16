use super::*;
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

pub(super) fn gain_db(gain: f64) -> f32 {
    if gain <= 0. {
        -60.
    } else {
        (20. * gain.log10()).clamp(-60., 24.) as f32
    }
}

pub(super) fn linear_gain(value: f32) -> f64 {
    if value <= -60. {
        0.
    } else {
        10f64.powf(f64::from(value) / 20.)
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
            let id = node.id;
            let value = gain_db(node.mixer.gain);
            let slider = self.mixer.faders.entry(id).or_insert_with(|| {
                let slider = cx.new(|_| {
                    base::slider::SliderState::new()
                        .min(-60.)
                        .max(24.)
                        .step(0.1)
                        .default_value(value)
                });
                cx.subscribe(
                    &slider,
                    move |shell, _, event: &base::slider::SliderEvent, cx| {
                        if let base::slider::SliderEvent::Change(value) = event {
                            shell.state.update(cx, |state, _| {
                                if let Some(node) = state.snapshot.graph.node(id) {
                                    let mixer = MixerState {
                                        gain: linear_gain(value.end()),
                                        ..node.mixer
                                    };
                                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetMixer {
                                        node: id,
                                        state: mixer,
                                    }));
                                }
                            });
                            cx.notify();
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
        for edge in graph.graph.edges() {
            let id = edge.id();
            let value = gain_db(edge.params().gain);
            let slider = self.mixer.sends.entry(id).or_insert_with(|| {
                let slider = cx.new(|_| {
                    base::slider::SliderState::new()
                        .min(-60.)
                        .max(24.)
                        .step(0.1)
                        .default_value(value)
                });
                cx.subscribe(
                    &slider,
                    move |shell, _, event: &base::slider::SliderEvent, cx| {
                        if let base::slider::SliderEvent::Change(value) = event {
                            shell.state.update(cx, |state, _| {
                                if let Ok(edge) = state.snapshot.graph.graph.get_edge(id) {
                                    let params = SendParams {
                                        gain: linear_gain(value.end()),
                                        ..*edge.params()
                                    };
                                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                                        edge: id,
                                        params,
                                    }));
                                }
                            });
                            cx.notify();
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
            .child("Move channel")
    }
}
