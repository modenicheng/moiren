//! Relative vertical gestures for continuous DSP parameters, not angle chasing.
use super::*;
use std::cell::Cell;

#[derive(Clone)]
struct KnobDrag {
    node: NodeId,
    parameter: Parameter,
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
        node: node.id,
        parameter,
        cursor: Rc::new(Cell::new(0.)),
        position: Rc::new(Cell::new(value)),
    };
    let id = node.id;
    div()
        .id(format!("card-knob-{id:?}-{parameter:?}"))
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
            if (drag.node, drag.parameter) != key {
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
