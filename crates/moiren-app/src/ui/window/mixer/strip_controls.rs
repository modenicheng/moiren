//! Compact channel actions and the strip-specific balance control.

use super::*;

fn control(button: ActionButton) -> Div {
    div().flex_1().min_w_0().child(button.full_width().square())
}

pub(in crate::ui::window) fn node_controls(
    entry: &GraphNodeState,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    let requested = entry.mixer;
    div()
        .flex()
        .flex_col()
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .child(
            div()
                .flex()
                .w_full()
                .child(control(
                    button(SharedString::from(format!("strip-mute-{id:?}")), "M")
                        .tooltip(t!("Mute this channel"))
                        .variant(if requested.muted {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(cx.entity(), id, |state| {
                            state.muted = !state.muted
                        })),
                ))
                .child(control(
                    button(SharedString::from(format!("strip-solo-{id:?}")), "S")
                        .tooltip(t!("Solo this channel and its connected signal path"))
                        .variant(if requested.solo {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(cx.entity(), id, |state| {
                            state.solo = !state.solo
                        })),
                )),
        )
        .child(
            div()
                .flex()
                .w_full()
                .child(control(
                    button(
                        SharedString::from(format!("strip-mode-{id:?}")),
                        requested.mode_label(channels),
                    )
                    .variant(if requested.channel_mode != 0 {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .tooltip(if channels == 2 {
                        t!("Stereo -> Mono average -> Left to both -> Right to both -> Swap")
                    } else {
                        t!("Cycle normal, mono average, individual channels, and reverse channel order")
                    })
                    .on_click(mixer_edit(cx.entity(), id, move |state| {
                        *state = state.next_mode(channels)
                    })),
                ))
                .child(control(
                    button(SharedString::from(format!("strip-dim-{id:?}")), "DIM")
                        .tooltip(t!("Dim attenuates the post-fader signal by 12 dB"))
                        .variant(if requested.dimmed {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(cx.entity(), id, |state| {
                            state.dimmed = !state.dimmed
                        })),
                )),
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
/// short track, while its label reports the requested left/right offset.
pub(in crate::ui::window) fn balance(
    _shell: &StudioShell,
    entry: &GraphNodeState,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    if channels != 2 {
        return div()
            .id(SharedString::from(format!("balance-{id:?}")))
            .flex_none()
            .h(px(40.));
    }
    let value = entry.mixer.balance.clamp(-1., 1.);
    let position = ((value as f32) + 1.) * 0.5;
    let value_label = if value.abs() < 0.005 {
        "C".to_string()
    } else {
        format!(
            "{} {:.0}",
            if value < 0. { "L" } else { "R" },
            value.abs() * 100.
        )
    };
    let track_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
    let this = cx.entity();
    let drag_this = this.clone();
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
        .child(
            div()
                .id(track_id)
                .relative()
                .w_full()
                .h(px(17.))
                .bg(theme::WINDOW_BG)
                .border_1()
                .border_color(theme::BORDER)
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
                .gpui_test_observe(),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(17.))
                .text_size(px(10.))
                .font_family(theme::FONT_MONO)
                .text_color(theme::TEXT_MUTED)
                .child(value_label),
        )
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
