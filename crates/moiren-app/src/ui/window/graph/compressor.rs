use super::*;
use crate::service::{CompressorParameter, GraphEdit};
use moiren_core::graph::{NodeId, NodeKind};
use std::collections::BTreeMap;

#[derive(Default)]
pub(in crate::ui::window) struct CompressorControls {
    pub inputs: BTreeMap<(NodeId, CompressorParameter), Entity<component::input::InputState>>,
    pub sliders: BTreeMap<(NodeId, CompressorParameter), Entity<base::slider::SliderState>>,
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_compressor_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.state.read(cx).snapshot.graph.clone();
        self.compressor
            .inputs
            .retain(|(id, _), _| snapshot.node(*id).is_some());
        self.compressor
            .sliders
            .retain(|(id, _), _| snapshot.node(*id).is_some());
        let Some(node) = snapshot.graph.nodes().iter().find(|node| {
            node.kind() == NodeKind::Compressor && self.selected_node == Some(node.id())
        }) else {
            return;
        };
        let id = node.id();
        let state = snapshot
            .node(id)
            .expect("selected node metadata")
            .compressor;
        for parameter in CompressorParameter::ALL {
            let value = state.value(parameter);
            let input =
                self.compressor
                    .inputs
                    .entry((id, parameter))
                    .or_insert_with(|| {
                        let input = cx.new(|cx| {
                            component::input::InputState::new(window, cx)
                                .default_value(value.to_string())
                        });
                        cx.subscribe(
                            &input,
                            move |shell, input, event: &component::input::InputEvent, cx| {
                                if matches!(
                                    event,
                                    component::input::InputEvent::PressEnter { .. }
                                        | component::input::InputEvent::Blur
                                ) {
                                    let value =
                                        input.read(cx).value().parse::<f64>().unwrap_or(f64::NAN);
                                    if shell.state.read(cx).snapshot.graph.node(id).is_some_and(
                                        |node| node.compressor.value(parameter) == value,
                                    ) {
                                        return;
                                    }
                                    shell.state.update(cx, |state, _| {
                                        state.dispatch(AppRequest::EditGraph(
                                            GraphEdit::SetCompressor {
                                                node: id,
                                                parameter,
                                                value,
                                            },
                                        ))
                                    });
                                    cx.notify();
                                }
                            },
                        )
                        .detach();
                        input
                    });
            if !input.read(cx).focus_handle(cx).is_focused(window)
                && input.read(cx).value().parse::<f64>().ok() != Some(value)
            {
                input.update(cx, |state, cx| {
                    state.set_value(value.to_string(), window, cx)
                });
            }
            let position = slider_position(parameter, value);
            let slider = self
                .compressor
                .sliders
                .entry((id, parameter))
                .or_insert_with(|| {
                    let slider = cx.new(|_| {
                        base::slider::SliderState::new()
                            .min(0.)
                            .max(1.)
                            .step(0.001)
                            .default_value(position)
                    });
                    cx.subscribe(
                        &slider,
                        move |shell, _, event: &base::slider::SliderEvent, cx| {
                            if let base::slider::SliderEvent::Change(position) = event {
                                shell.state.update(cx, |state, _| {
                                    state.dispatch(AppRequest::EditGraph(
                                        GraphEdit::SetCompressor {
                                            node: id,
                                            parameter,
                                            value: slider_value(parameter, position.end()),
                                        },
                                    ))
                                });
                                cx.notify();
                            }
                        },
                    )
                    .detach();
                    slider
                });
            if slider.read(cx).value().end() != position {
                slider.update(cx, |state, cx| state.set_value(position, window, cx));
            }
        }
    }
}

fn logarithmic(parameter: CompressorParameter) -> bool {
    matches!(
        parameter,
        CompressorParameter::Attack
            | CompressorParameter::Release
            | CompressorParameter::Hold
            | CompressorParameter::Ratio
    )
}

fn slider_position(parameter: CompressorParameter, value: f64) -> f32 {
    let (min, max) = parameter.range();
    let span = max - min;
    if logarithmic(parameter) {
        ((value - min).ln_1p() / span.ln_1p()) as f32
    } else {
        ((value - min) / span) as f32
    }
}

fn slider_value(parameter: CompressorParameter, position: f32) -> f64 {
    let (min, max) = parameter.range();
    let span = max - min;
    let position = f64::from(position.clamp(0., 1.));
    if logarithmic(parameter) {
        (min + (position * span.ln_1p()).exp_m1()).clamp(min, max)
    } else {
        min + position * span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn compressor_slider_maps_engine_domains_without_losing_zero_times() {
        for parameter in CompressorParameter::ALL {
            let (min, max) = parameter.range();
            for value in [min, (min + max) * 0.5, max] {
                let restored = slider_value(parameter, slider_position(parameter, value));
                assert!((restored - value).abs() < (max - min) * 0.00001);
            }
            assert_eq!(slider_value(parameter, 0.), min);
        }
    }
}
