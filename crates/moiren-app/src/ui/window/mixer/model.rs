use super::*;
use crate::service::GraphSnapshot;
use crate::ui::widgets::fader::FaderScale;
use rust_i18n::t;
use std::collections::BTreeMap;

#[derive(Default)]
pub(in crate::ui::window) struct MixerPresentation {
    pub order: Vec<NodeId>,
    pub faders: BTreeMap<NodeId, Entity<base::slider::SliderState>>,
    pub sends: BTreeMap<EdgeId, Entity<base::slider::SliderState>>,
    pub send_inputs: BTreeMap<EdgeId, Entity<component::input::InputState>>,
    pub editing_send: Option<EdgeId>,
    pub inputs_collapsed: bool,
    pub sends_collapsed: bool,
    synced_revision: Option<u64>,
    #[cfg(feature = "gpui-test")]
    pub trace: std::cell::RefCell<super::performance::MeterTrace>,
}

impl MixerPresentation {
    /// Fixed strip and dock widths; strips are not user-resizable.
    pub(in crate::ui::window) const STRIP_WIDTH: f32 = 160.;
    pub(in crate::ui::window) const DOCK_WIDTH: f32 = 360.;
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
/// differ. Every edit applies itself; releases carry no extra behavior.
pub(in crate::ui::window) fn sync_sliders<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, Entity<base::slider::SliderState>>,
    rows: Vec<(K, f32, SliderEdit)>,
) {
    sync_slider_range(window, cx, state, sliders, rows, [-60., 24., 0.1]);
}

fn sync_slider_range<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, Entity<base::slider::SliderState>>,
    rows: Vec<(K, f32, SliderEdit)>,
    range: [f32; 3],
) {
    for (id, value, edit) in rows {
        let slider = sliders.entry(id).or_insert_with(|| {
            let slider = cx.new(|_| {
                base::slider::SliderState::new()
                    .min(range[0])
                    .max(range[1])
                    .step(range[2])
                    .default_value(value)
            });
            let state = state.clone();
            cx.subscribe(
                &slider,
                move |_, _, event: &base::slider::SliderEvent, cx| {
                    if let base::slider::SliderEvent::Change(value) = event {
                        state.update(cx, |state, _| {
                            if let Some(edit) = edit(value.end(), &state.snapshot.graph) {
                                state.dispatch(edit);
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

impl StudioShell {
    pub(in crate::ui::window) fn sync_mixer_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = self.state.read(cx).snapshot.graph.candidate_revision;
        if self.mixer.synced_revision == Some(revision) {
            return;
        }
        self.mixer.synced_revision = Some(revision);
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.mixer.order.retain(|id| graph.node(*id).is_some());
        self.mixer.faders.retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .sends
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        self.mixer
            .send_inputs
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        if self
            .mixer
            .editing_send
            .is_some_and(|id| graph.graph.get_edge(id).is_err())
        {
            self.mixer.editing_send = None;
        }
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
                    1. - FaderScale::position(node.mixer.gain),
                    Box::new(move |value: f32, graph: &GraphSnapshot| {
                        graph.node(id).map(|node| {
                            AppRequest::EditGraph(GraphEdit::SetMixer {
                                node: id,
                                state: MixerState {
                                    gain: FaderScale::gain(1. - value),
                                    ..node.mixer
                                },
                            })
                        })
                    }) as SliderEdit,
                )
            })
            .collect();
        sync_slider_range(
            window,
            cx,
            &self.state,
            &mut self.mixer.faders,
            faders,
            [0., 1., 0.0001],
        );
        for edge in graph.graph.edges() {
            let id = edge.id();
            let input = self.mixer.send_inputs.entry(id).or_insert_with(|| {
                let input = cx.new(|cx| component::input::InputState::new(window, cx));
                cx.subscribe(
                    &input,
                    move |shell, input, event: &component::input::InputEvent, cx| {
                        if !matches!(
                            event,
                            component::input::InputEvent::PressEnter { .. }
                                | component::input::InputEvent::Blur
                        ) || shell.mixer.editing_send != Some(id)
                        {
                            return;
                        }
                        shell.mixer.editing_send = None;
                        let text = input.read(cx).value();
                        let gain = if matches!(text.trim(), "-inf" | "−∞" | "-∞") {
                            Some(0.)
                        } else {
                            text.parse::<f64>()
                                .ok()
                                .filter(|db| db.is_finite() && (-120. ..=24.).contains(db))
                                .map(|db| 10f64.powf(db / 20.))
                        };
                        if let Some(gain) = gain {
                            shell.state.update(cx, |state, _| {
                                if let Ok(edge) = state.snapshot.graph.graph.get_edge(id) {
                                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                                        edge: id,
                                        params: SendParams {
                                            gain,
                                            ..*edge.params()
                                        },
                                    }));
                                }
                            });
                        }
                        cx.notify();
                    },
                )
                .detach();
                input
            });
            if self.mixer.editing_send != Some(id) {
                let text = if edge.params().gain <= 0. {
                    "-inf".into()
                } else {
                    format!("{:.1}", 20. * edge.params().gain.log10())
                };
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
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
        sync_sliders(window, cx, &self.state, &mut self.mixer.sends, sends);
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
