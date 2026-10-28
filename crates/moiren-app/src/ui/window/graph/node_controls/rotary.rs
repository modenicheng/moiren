//! Relative vertical gestures for continuous DSP parameters, not angle chasing.
use super::*;
use crate::service::RequestId;
use crate::ui::widgets::fader::SliderBehavior;
use std::cell::Cell;
use std::time::{Duration, Instant};

type Key = (NodeId, Parameter);

/// Only admitted intent owns the dial until its receipt is published. Keeping
/// the value too lets a rejected later gesture fall back to the last admission.
pub(super) struct Pending {
    request: RequestId,
    position: f32,
}

/// Call only after a validated numeric edit was admitted. This changes future
/// gesture presentation, not the exact submitted value or the editor's text.
pub(super) fn remember_input_admission(
    shell: &mut StudioShell,
    id: NodeId,
    parameter: Parameter,
    value: f64,
    request: RequestId,
    cx: &Context<StudioShell>,
) {
    let key = (id, parameter);
    if shell.pending_project.is_some()
        || !shell.node_controls.sliders.contains_key(&key)
        || (!value.is_finite() && !(parameter == Parameter::Level && value == f64::NEG_INFINITY))
        || shell
            .node_controls
            .rotary_pending
            .get(&key)
            .is_some_and(|pending| pending.request >= request)
    {
        return;
    }
    let position = {
        let state = shell.state.read(cx);
        if request <= state.snapshot.consumed_request_id {
            return;
        }
        let graph = &state.snapshot.graph;
        let Some(node) = graph.node(id) else { return };
        let Some(kind) = graph.kind_of(id) else {
            return;
        };
        if !Parameter::primary(kind, node).contains(&parameter) {
            return;
        }
        let (min, max) = parameter.range(node);
        let offset = value.clamp(min, max) - min;
        // Mirror Parameter::position without cloning GraphNodeState: a Level
        // field can belong to a Mapper carrying a large routing matrix.
        if matches!(
            parameter,
            Parameter::Frequency
                | Parameter::EqFrequency
                | Parameter::EqQ
                | Parameter::GateAttack
                | Parameter::GateHold
                | Parameter::GateRelease
                | Parameter::LimitRelease
                | Parameter::DelayTime
                | Parameter::Compressor(
                    crate::service::CompressorParameter::Attack
                        | crate::service::CompressorParameter::Release
                        | crate::service::CompressorParameter::Ratio
                )
        ) {
            (offset.ln_1p() / (max - min).ln_1p()) as f32
        } else {
            (offset / (max - min).max(f64::EPSILON)) as f32
        }
    };
    if position.is_finite() {
        shell
            .node_controls
            .rotary_pending
            .insert(key, Pending { request, position });
    }
}

pub(super) fn sync(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) {
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let consumed = shell.state.read(cx).snapshot.consumed_request_id;
    let controls = &mut shell.node_controls;
    controls
        .rotary_pending
        .retain(|key, pending| pending.request > consumed && controls.sliders.contains_key(key));
    for node in &graph.nodes {
        let Some(kind) = graph.kind_of(node.id) else {
            continue;
        };
        for parameter in Parameter::primary(kind, node) {
            let key = (node.id, parameter);
            let position = parameter.position(kind, node);
            let slot = controls.sliders.entry(key).or_insert_with(|| {
                new_slot(key, position, parameter.default_position(kind, node), cx)
            });
            // Delay's prepared capacity can change while this entity survives.
            if parameter == Parameter::DelayTime {
                slot.behavior.reset = parameter.default_position(kind, node);
            }
            if !slot.dragging.get() {
                let position = controls
                    .rotary_pending
                    .get(&key)
                    .map_or(position, |pending| pending.position);
                if slot.state.read(cx).value().end() != position {
                    slot.state
                        .update(cx, |state, cx| state.set_value(position, window, cx));
                }
            }
        }
    }
}

fn new_slot(key: Key, position: f32, reset: f32, cx: &mut Context<StudioShell>) -> SliderSlot {
    let slider = cx.new(|_| {
        base::slider::SliderState::new()
            .min(0.)
            .max(1.)
            .step(0.001)
            .default_value(position)
    });
    let identity = slider.entity_id();
    let dragging = Rc::new(Cell::new(false));
    let drag_flag = dragging.clone();
    let mut last_sent: Option<Instant> = None;
    cx.subscribe(
        &slider,
        move |shell, slider, event: &base::slider::SliderEvent, cx| {
            if !is_current(shell, key, identity) {
                drag_flag.set(false);
                last_sent = None;
                return;
            }
            match event {
                base::slider::SliderEvent::Change(value) => {
                    drag_flag.set(true);
                    let now = Instant::now();
                    // Match the existing 33 ms drag budget; release always
                    // submits the latest thumb, without a deferred timer.
                    if last_sent
                        .is_none_or(|sent| now.duration_since(sent) >= Duration::from_millis(33))
                        && dispatch(shell, key, identity, value.end(), cx)
                    {
                        last_sent = Some(now);
                    }
                }
                base::slider::SliderEvent::Release(value) => {
                    drag_flag.set(false);
                    last_sent = None;
                    // A wheel/reset may have superseded a queued release.
                    if slider.read(cx).value().end() == value.end() {
                        dispatch(shell, key, identity, value.end(), cx);
                    }
                }
            }
            cx.notify();
        },
    )
    .detach();
    // The shell owns this callback via SliderSlot. A weak owner and an opaque
    // entity identity avoid a shell -> callback -> shell ownership cycle.
    let owner = cx.entity().downgrade();
    SliderSlot {
        state: slider,
        dragging,
        behavior: SliderBehavior {
            step: Rc::new(|value, direction| (value + direction as f32 * 0.001).clamp(0., 1.)),
            reset,
            commit: Rc::new(move |position, window, cx| {
                let _ = owner.update(cx, |shell, cx| {
                    if dispatch(shell, key, identity, position, cx) {
                        shell.node_controls.sliders[&key]
                            .state
                            .update(cx, |state, cx| state.set_value(position, window, cx));
                    }
                });
            }),
        },
    }
}

fn is_current(shell: &StudioShell, key: Key, identity: EntityId) -> bool {
    shell.pending_project.is_none()
        && shell
            .node_controls
            .sliders
            .get(&key)
            .is_some_and(|slot| slot.state.entity_id() == identity)
}

fn dispatch(
    shell: &mut StudioShell,
    key: Key,
    identity: EntityId,
    position: f32,
    cx: &mut Context<StudioShell>,
) -> bool {
    if !position.is_finite() || !is_current(shell, key, identity) {
        return false;
    }
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let Some(node) = graph.node(key.0) else {
        return false;
    };
    let Some(kind) = graph.kind_of(key.0) else {
        return false;
    };
    if !Parameter::primary(kind, node).contains(&key.1) {
        return false;
    }
    let Some(edit) = key
        .1
        .edit(key.1.value_at_position(position, node), kind, node)
    else {
        return false;
    };
    let admitted = shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(edit))
    });
    let accepted = if let Ok(request) = admitted {
        shell
            .node_controls
            .rotary_pending
            .insert(key, Pending { request, position });
        true
    } else {
        false
    };
    cx.notify();
    accepted
}

#[derive(Clone)]
struct KnobDrag {
    key: String,
    slider: EntityId,
    cursor: Rc<Cell<f32>>,
    position: Rc<Cell<f32>>,
}

impl Render for KnobDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(super) fn knob(
    shell: &StudioShell,
    node: &GraphNodeState,
    parameter: Parameter,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let key = (node.id, parameter);
    let Some(slot) = shell.node_controls.sliders.get(&key) else {
        return div().into_any_element();
    };
    keyed_knob(format!("card-knob-{:?}-{parameter:?}", node.id), slot, cx)
}

/// Identity includes the editing surface and target. A Rack card and its
/// Inspector can therefore reuse the dial without sharing drag ownership.
pub(super) fn keyed_knob(key: String, slot: &SliderSlot, cx: &Context<StudioShell>) -> AnyElement {
    let value = slot.state.read(cx).value().end();
    let behavior = slot.behavior.clone();
    let wheel_commit = behavior.commit.clone();
    let slider = slot.state.clone();
    let drag_slider = slider.clone();
    let release_slider = slider.clone();
    let dragging = slot.dragging.clone();
    let outside_slider = slider.clone();
    let outside_dragging = slot.dragging.clone();
    let drag = KnobDrag {
        key: key.clone(),
        slider: slider.entity_id(),
        cursor: Rc::new(Cell::new(0.)),
        position: Rc::new(Cell::new(value)),
    };
    div()
        .id(key.clone())
        .w(px(54.))
        .h(px(54.))
        .flex_none()
        .relative()
        .cursor_pointer()
        .on_drag(drag, |drag, _, window, cx| {
            let drag = drag.clone();
            drag.cursor.set(f32::from(window.mouse_position().y));
            cx.new(|_| drag)
        })
        .on_drag_move(move |event: &DragMoveEvent<KnobDrag>, window, cx| {
            let drag = event.drag(cx);
            if drag.key != key || drag.slider != drag_slider.entity_id() {
                return;
            }
            let y = f32::from(event.event.position.y);
            let value = dragged_position(
                drag.position.get(),
                drag.cursor.replace(y) - y,
                event.event.modifiers.shift,
            );
            drag.position.set(value);
            // Reuse the slider's throttled dispatch and drag ownership, so a
            // stale service snapshot cannot pull the dial off the pointer.
            drag_slider.update(cx, |state, cx| {
                state.set_value(value, window, cx);
                cx.emit(base::slider::SliderEvent::Change(value.into()));
            });
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            if dragging.replace(false) {
                release_slider.update(cx, |state, cx| {
                    cx.emit(base::slider::SliderEvent::Release(state.value()))
                });
            }
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
            if outside_dragging.replace(false) {
                outside_slider.update(cx, |state, cx| {
                    cx.emit(base::slider::SliderEvent::Release(state.value()))
                });
            }
        })
        .on_scroll_wheel(move |event, window, cx| {
            cx.stop_propagation();
            let delta = match event.delta {
                ScrollDelta::Pixels(p) => f32::from(p.y),
                ScrollDelta::Lines(p) => p.y,
            };
            if delta != 0. {
                let step = if event.modifiers.shift { 0.001 } else { 0.01 };
                wheel_commit(
                    (slider.read(cx).value().end() + delta.signum() * step).clamp(0., 1.),
                    window,
                    cx,
                );
            }
        })
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                (behavior.commit)(behavior.reset, window, cx);
            }
        })
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(
                t!("Drag vertically · Shift for fine adjustment · double-click to reset")
                    .to_string(),
            )
            .build(window, cx)
        })
        .child(
            canvas(
                move |_, _, _| value,
                |bounds, value, window, _| {
                    let center = bounds.center();
                    let radius = f32::from(bounds.size.width) * 0.38;
                    let start = std::f32::consts::PI * 0.75;
                    let span = std::f32::consts::PI * 1.5;
                    let point_at = |t: f32, r: f32| {
                        center
                            + point(
                                px((start + t * span).cos() * r),
                                px((start + t * span).sin() * r),
                            )
                    };
                    for (end, color, width) in [
                        (1., theme::BORDER_STRONG, 3.),
                        (value, theme::SIGNAL_DEVICE, 3.),
                    ] {
                        let mut path = PathBuilder::stroke(px(width));
                        for i in 0..=48 {
                            let p = point_at(i as f32 / 48. * end, radius);
                            if i == 0 {
                                path.move_to(p)
                            } else {
                                path.line_to(p)
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                    let mut pointer = PathBuilder::stroke(px(2.));
                    pointer.move_to(point_at(value, radius * 0.35));
                    pointer.line_to(point_at(value, radius * 0.78));
                    if let Ok(path) = pointer.build() {
                        window.paint_path(path, theme::TEXT);
                    }
                },
            )
            .size_full(),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn dragged_position(position: f32, delta: f32, fine: bool) -> f32 {
    (position + delta / if fine { 1800. } else { 180. }).clamp(0., 1.)
}

#[cfg(test)]
mod tests {
    use super::dragged_position;
    #[::core::prelude::v1::test]
    fn rotary_drag_is_relative_fine_and_bounded() {
        assert!((dragged_position(0.5, 18., false) - 0.6).abs() < 1e-6);
        assert!((dragged_position(0.5, 18., true) - 0.51).abs() < 1e-6);
        assert_eq!(dragged_position(0.5, 1000., false), 1.);
    }
}
