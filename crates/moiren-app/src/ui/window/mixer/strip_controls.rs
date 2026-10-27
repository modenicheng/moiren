//! Compact channel actions and the strip-specific balance control.

use super::*;

fn control(button: impl IntoElement) -> Div {
    div().flex_1().min_w_0().child(button)
}

/// One flat cell of the strip's M/S group: the same unstyled
/// [`base::Button`] the routing matrix builds on, painted with the
/// [`ActionButton`] Primary/Secondary contract (toggled fills, hover,
/// pressed, focus ring) but without a per-button box. The group container
/// owns the single rounded outline, so the cell stays square and border-free
/// apart from the interior separators its slot draws.
fn group_cell(
    id: SharedString,
    label: impl Into<SharedString>,
    toggled: bool,
    tooltip: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> base::Button {
    let label = label.into();
    let tooltip = tooltip.into();
    let (bg, fg, hover_bg, pressed_bg) = if toggled {
        (
            theme::controls::PRIMARY,
            theme::controls::ON_ACCENT,
            theme::controls::PRIMARY_HOVER,
            theme::controls::PRIMARY_PRESSED,
        )
    } else {
        (
            theme::controls::RAISED,
            theme::controls::TEXT,
            theme::controls::RAISED,
            theme::controls::CANVAS,
        )
    };
    // Keep the ActionButton sizing contract by resolving the same metrics.
    let metrics = ControlSize::Small.metrics(Density::default());
    base::Button::new(id)
        .accessibility_label(label.clone())
        .flex_none()
        .w_full()
        .h_full()
        .rounded_none()
        .px(px(metrics.padding))
        .font_family(theme::controls::FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(metrics.font))
        .line_height(px(metrics.line_height))
        .bg(bg)
        .text_color(fg)
        .hover(move |s| s.bg(hover_bg).text_color(fg))
        .active(move |s| s.bg(pressed_bg).text_color(fg))
        .focus_visible(move |s| {
            s.bg(bg)
                .text_color(fg)
                .border_2()
                .border_color(theme::controls::FOCUS)
        })
        .child(label)
        .tooltip(move |window, cx| {
            component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .on_click(on_click)
}

pub(in crate::ui::window) fn node_controls(
    entry: &GraphNodeState,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    let requested = entry.mixer;
    let metrics = ControlSize::Small.metrics(Density::default());
    div()
        .flex()
        .flex_col()
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .rounded(px(theme::RADIUS_SM))
                .border_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .w_full()
                        .h(px(metrics.height))
                        .child(control(group_cell(
                            SharedString::from(format!("strip-mute-{id:?}")),
                            "M",
                            requested.muted,
                            t!("Mute this channel"),
                            mixer_edit(cx.entity(), id, |state| state.muted = !state.muted),
                        )))
                        .child(control(group_cell(
                            SharedString::from(format!("strip-solo-{id:?}")),
                            "S",
                            requested.solo,
                            t!("Solo this channel and its connected signal path"),
                            mixer_edit(cx.entity(), id, |state| state.solo = !state.solo),
                        ))
                        .border_l_1()
                        .border_color(theme::BORDER)),
                )
                .child(
                    div()
                        .flex()
                        .w_full()
                        .h(px(metrics.height))
                        .child(control(group_cell(
                            SharedString::from(format!("strip-mode-{id:?}")),
                            requested.mode_label(channels),
                            requested.channel_mode != 0,
                            if channels == 2 {
                                t!("Stereo -> Mono average -> Left to both -> Right to both -> Swap")
                            } else {
                                t!("Cycle normal, mono average, individual channels, and reverse channel order")
                            },
                            mixer_edit(cx.entity(), id, move |state| {
                                *state = state.next_mode(channels)
                            }),
                        ))
                        .border_t_1()
                        .border_color(theme::BORDER))
                        .child(control(group_cell(
                            SharedString::from(format!("strip-dim-{id:?}")),
                            "DIM",
                            requested.dimmed,
                            t!("Dim attenuates the post-fader signal by 12 dB"),
                            mixer_edit(cx.entity(), id, |state| state.dimmed = !state.dimmed),
                        ))
                        .border_l_1()
                        .border_t_1()
                        .border_color(theme::BORDER)),
                ),
        )
}

#[derive(Clone)]
struct BalanceDrag(NodeId);

impl Render for BalanceDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn balance_value(position: Point<Pixels>, bounds: Bounds<Pixels>) -> f64 {
    if bounds.size.width <= px(0.) {
        return 0.;
    }
    (f32::from(position.x - bounds.left()) / f32::from(bounds.size.width))
        .clamp(0., 1.)
        .mul_add(2., -1.) as f64
}

/// Wheel direction as a step count: wheel up (or tilt right) is +1. Mirrors
/// the fader's private helper: gpui carries touchpad pixel deltas and
/// discrete line notches with the same sign — positive scrolls the content
/// down, i.e. the wheel moved up. For the horizontal balance the vertical
/// axis is the plain wheel gesture and the secondary axis covers shift+wheel
/// and tilt wheels.
fn wheel_direction(delta: &ScrollDelta) -> i32 {
    let (x, y) = match delta {
        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
        ScrollDelta::Lines(delta) => (delta.x, delta.y),
    };
    let (x, y) = (x.signum(), y.signum());
    if y != 0. { y as i32 } else { x as i32 }
}

fn set_balance(this: &Entity<StudioShell>, id: NodeId, balance: f64, cx: &mut App) {
    this.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            if let Some(node) = state.snapshot.graph.node(id) {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetMixer {
                    node: id,
                    state: MixerState {
                        balance: balance.clamp(-1., 1.),
                        ..node.mixer
                    },
                }));
            }
        });
        cx.notify();
    });
}

/// Balance is a center-biased monitor control rather than a value slider. It
/// keeps a fixed center reference and moves one flat position marker across a
/// short track, while its label reports the requested left/right offset. The
/// click/drag handlers live on the track itself: the label under it is a
/// click-to-type editor whose clicks must not also set a balance position.
pub(in crate::ui::window) fn balance(
    shell: &StudioShell,
    entry: &GraphNodeState,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    if channels != 2 {
        return div()
            .id(SharedString::from(format!("balance-{id:?}")))
            .flex_none()
            .h(px(40.))
            .into_any_element();
    }
    let value = entry.mixer.balance.clamp(-1., 1.);
    let position = ((value as f32) + 1.) * 0.5;
    let editing = shell.mixer.editing_balance == Some(id);
    let track_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
    let this = cx.entity();
    let drag_this = this.clone();
    let wheel_this = this.clone();
    let click_bounds = track_bounds.clone();
    let drag_bounds = track_bounds.clone();
    let track_id = SharedString::from(format!("balance-track-{id:?}"));

    div()
        .id(SharedString::from(format!("balance-{id:?}")))
        .flex()
        .flex_col()
        .flex_none()
        .h(px(40.))
        .px(px(8.))
        .gap(px(2.))
        .child(
            div()
                .id(track_id)
                .relative()
                .w_full()
                .h(px(17.))
                .bg(theme::WINDOW_BG)
                .border_1()
                .border_color(theme::BORDER)
                .cursor_pointer()
                .on_click(move |event, _, cx| {
                    if event.click_count() == 2 {
                        set_balance(&this, id, 0., cx);
                    } else if let Some(position) = event.mouse_position() {
                        set_balance(&this, id, balance_value(position, click_bounds.get()), cx);
                    }
                })
                .on_drag(BalanceDrag(id), |drag, _, _, cx| cx.new(|_| drag.clone()))
                .on_drag_move(move |event: &DragMoveEvent<BalanceDrag>, _, cx| {
                    // Drag-move listeners also see movement outside their own bounds.
                    // Only the originating strip owns the drag's coordinate mapping.
                    if event.drag(cx).0 != id {
                        return;
                    }
                    let value = balance_value(event.event.position, drag_bounds.get());
                    set_balance(&drag_this, id, value, cx);
                })
                .on_scroll_wheel({
                    let this = wheel_this;
                    move |event: &ScrollWheelEvent, _, cx| {
                        // A wheel step rides the same dispatch as the drag and
                        // click edits; propagation stops here so adjusting the
                        // balance never scrolls the strip page or the
                        // inspector panel underneath.
                        cx.stop_propagation();
                        let direction = wheel_direction(&event.delta);
                        if direction == 0 {
                            return;
                        }
                        // One notch is 0.1 in the Left/Right buttons' sign
                        // convention: left is negative and a wheel-up notch
                        // moves right. The drag path has no center snap, so
                        // stepping keeps the value it lands on.
                        let step = f64::from(direction) * 0.1;
                        set_balance(&this, id, (value + step).clamp(-1., 1.), cx);
                    }
                })
                .child(
                    canvas(
                        move |bounds, _, _| track_bounds.set(bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(relative(0.5))
                        .top(px(2.))
                        .ml(px(-0.5))
                        .w(px(1.))
                        .h(px(11.))
                        .bg(theme::SIGNAL_DEVICE.opacity(0.45)),
                )
                .child(
                    div()
                        .absolute()
                        .left(relative(position))
                        .top(px(1.))
                        .ml(px(-2.))
                        .w(px(4.))
                        .h(px(14.))
                        .bg(theme::SIGNAL_DEVICE),
                )
                .tooltip(|window, cx| {
                    component::tooltip::Tooltip::new(
                        t!("Scroll to adjust · double-click to reset").into_owned(),
                    )
                    .build(window, cx)
                })
                .gpui_test_observe(),
        )
        .child(
            editable_value(
                SharedString::from(format!("balance-label-{id:?}")),
                balance_text(value),
                editing,
                &shell.mixer.balance_inputs[&id],
                start_value_edit(
                    cx.entity(),
                    EditableValueFamily::Balance,
                    id,
                    move |graph| {
                        graph
                            .node(id)
                            .map(|node| balance_text(node.mixer.balance))
                            .unwrap_or_default()
                    },
                ),
                px(10.),
                TextAlign::Center,
            )
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .h(px(17.))
            .font_family(theme::FONT_MONO)
            .text_color(theme::TEXT_MUTED)
            .gpui_test_observe(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::balance_value;
    use gpui_kit::{Bounds, Point, point, px, size};

    #[test]
    fn balance_track_maps_edges_and_center() {
        let bounds = Bounds::new(point(px(10.), px(0.)), size(px(100.), px(16.)));
        assert_eq!(balance_value(Point::new(px(10.), px(8.)), bounds), -1.);
        assert_eq!(balance_value(Point::new(px(60.), px(8.)), bounds), 0.);
        assert_eq!(balance_value(Point::new(px(110.), px(8.)), bounds), 1.);
        assert_eq!(balance_value(Point::new(px(0.), px(8.)), bounds), -1.);
    }
}
