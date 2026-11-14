use super::*;
use crate::service::{CompressorParameter, CompressorState, GraphEdit, GraphSnapshot};
use crate::ui::editors::{
    SliderDispatch, SliderEdit, SliderSlot, commit_closure, sync_slider_range,
};
use crate::ui::widgets::fader::SliderBehavior;
use moiren_core::graph::{NodeId, NodeKind};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Normalized position step shared by slider creation and the wheel gesture.
const SLIDER_STEP: f32 = 0.001;

#[derive(Default)]
pub(crate) struct CompressorControls {
    pub inputs: BTreeMap<(NodeId, CompressorParameter), Entity<component::input::InputState>>,
    pub sliders: BTreeMap<(NodeId, CompressorParameter), SliderSlot>,
}

/// Wheel travel on a compressor parameter: one normalized slider step per
/// notch, clamped to the unit position range.
fn compressor_step(value: f32, notches: f32) -> f32 {
    (value + notches * SLIDER_STEP).clamp(0., 1.)
}

/// Wheel + double-click behavior for one compressor parameter slider: one
/// notch is the slider's normalized step, and a double click restores the
/// engine default taken from the same [`CompressorState::default`] the
/// engine boots with. The commit is the shared mixer path: write the slider
/// value and dispatch the parameter edit immediately, bypassing drag
/// coalescing.
fn slider_behavior(
    dispatch: &Rc<SliderDispatch>,
    slider: Entity<base::slider::SliderState>,
    parameter: CompressorParameter,
) -> SliderBehavior {
    SliderBehavior {
        step: Rc::new(compressor_step),
        reset: default_position(parameter),
        commit: commit_closure(dispatch, &slider),
    }
}

/// Normalized slider position of the parameter's engine default.
fn default_position(parameter: CompressorParameter) -> f32 {
    slider_position(parameter, CompressorState::default().value(parameter))
}

impl StudioShell {
    pub(crate) fn sync_compressor_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.state.read(cx).snapshot.graph.clone();
        self.inspector
            .compressor
            .inputs
            .retain(|(id, _), _| snapshot.node(*id).is_some());
        self.inspector
            .compressor
            .sliders
            .retain(|(id, _), _| snapshot.node(*id).is_some());
        let Some(node) = snapshot.graph.nodes().iter().find(|node| {
            node.kind() == NodeKind::Compressor && self.selection.node == Some(node.id())
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
                self.inspector
                    .compressor
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
        }
        // Sliders ride the shared mixer sync: drag Changes dispatch at most
        // once per frame, the release's final value lands, the snapshot
        // write-back stands down while the slider leads the snapshot, and
        // the slot carries the wheel/double-click behavior.
        let rows = CompressorParameter::ALL
            .iter()
            .copied()
            .map(|parameter| {
                let position = slider_position(parameter, state.value(parameter));
                let edit = Box::new(move |position: f32, graph: &GraphSnapshot| {
                    graph.node(id).map(|_| {
                        AppRequest::EditGraph(GraphEdit::SetCompressor {
                            node: id,
                            parameter,
                            value: slider_value(parameter, position),
                        })
                    })
                }) as SliderEdit;
                ((id, parameter), position, edit)
            })
            .collect();
        sync_slider_range(
            window,
            cx,
            &self.state,
            &mut self.inspector.compressor.sliders,
            rows,
            [0., 1., SLIDER_STEP],
            |key, dispatch, slider| slider_behavior(dispatch, slider, key.1),
        );
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

    #[::core::prelude::v1::test]
    fn compressor_slider_step_moves_one_normalized_step_and_clamps() {
        assert!((compressor_step(0.5, 1.) - 0.501).abs() < 1e-6);
        assert!((compressor_step(0.5, -1.) - 0.499).abs() < 1e-6);
        // A fractional notch moves its share of the step.
        assert!((compressor_step(0.5, 0.5) - 0.5005).abs() < 1e-6);
        // The unit range clamps; zero travel is a no-op.
        assert_eq!(compressor_step(0., -1.), 0.);
        assert_eq!(compressor_step(1., 1.), 1.);
        assert_eq!(compressor_step(0.5, 0.), 0.5);
    }

    #[::core::prelude::v1::test]
    fn compressor_slider_reset_restores_engine_default_positions() {
        for parameter in CompressorParameter::ALL {
            let (min, max) = parameter.range();
            let default = CompressorState::default().value(parameter);
            let restored = slider_value(parameter, default_position(parameter));
            assert!(
                (restored - default).abs() < (max - min) * 0.00001,
                "{parameter:?}: reset restored {restored}, expected {default}"
            );
        }
    }
}
