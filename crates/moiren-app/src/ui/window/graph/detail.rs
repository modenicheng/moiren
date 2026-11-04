//! The per-node detail window (design 18 B4): an in-window overlay over
//! the graph page carrying the node's headline visualization — the EQ's
//! draggable control point, the dynamics family's transfer curve — plus
//! the identity controls every node shares: tint palette and a pointer to
//! the inspector for the long tail of parameters. Mounted at the shell
//! root next to the settings overlay, outside the cached canvas subtree,
//! so editing never re-projects the graph.

use super::super::controls;
use super::*;
use crate::service::{AppRequest, GraphEdit, NODE_TINT_COUNT};
use crate::ui::theme;
use gpui_kit::{
    AnyElement, Bounds, MouseButton, Pixels, ScrollWheelEvent, Window, canvas, fill, point, px,
    size,
};
use moiren_core::graph::NodeId;

/// EQ editor plot spans: log frequency axis and symmetric gain axis.
const EQ_FREQ_MIN: f64 = 20.;
const EQ_FREQ_MAX: f64 = 20_000.;
const EQ_GAIN_SPAN: f64 = 24.;

pub(in crate::ui::window) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let close = this.clone();
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let Some(id) = shell.detail_node else {
        return div().into_any_element();
    };
    let Some(logical) = snapshot.graph.graph.get_node(id).ok() else {
        return div().into_any_element();
    };
    let Some(entry) = snapshot.graph.node(id) else {
        return div().into_any_element();
    };
    let kind = logical.kind();
    let name = crate::ui::presentation::display_label(snapshot, entry, kind);
    let channels = if logical.channels() == 2 {
        "L/R".to_string()
    } else {
        format!("{} ch", logical.channels())
    };
    let detail = pages::graph_canvas::detail_string(kind, entry, &channels);

    let height = (f32::from(window.viewport_size().height) - 40.).max(260.);
    let _ = height;
    let viewport = window.viewport_size();
    let dialog_width = 480f32.min(f32::from(viewport.width) * 0.95);
    let dialog_height = (f32::from(viewport.height) - 40.).clamp(260., 640.);
    let left = (f32::from(viewport.width) - dialog_width) * 0.5;
    let top = (f32::from(viewport.height) - dialog_height) * 0.5;

    let body: AnyElement = match kind {
        moiren_core::graph::NodeKind::Eq => eq_section(shell, id, entry, cx),
        moiren_core::graph::NodeKind::Compressor
        | moiren_core::graph::NodeKind::Gate
        | moiren_core::graph::NodeKind::Limiter => dynamics_section(kind, entry),
        _ => div()
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .text_size(px(theme::TEXT_MD))
            .text_color(theme::TEXT_MUTED)
            .child(detail)
            .into_any_element(),
    };

    let tint_row = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(theme::SPACE_3))
        .pb(px(theme::SPACE_2))
        .child(
            div()
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_FAINT)
                .child(t!("Color")),
        )
        .child(swatches(this.clone(), id, entry.tint));
    let _ = close;

    div()
        .id("node-detail-page")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_key_down(move |event, _window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                close.update(cx, |shell, cx| {
                    shell.detail_node = None;
                    cx.notify();
                });
                cx.stop_propagation();
            }
        })
        .on_any_mouse_down({
            let this = this.clone();
            move |event, _window, cx| {
                cx.stop_propagation();
                let inside = event.position.x >= px(left)
                    && event.position.x <= px(left + dialog_width)
                    && event.position.y >= px(top)
                    && event.position.y <= px(top + dialog_height);
                if !inside {
                    this.update(cx, |shell, cx| {
                        shell.detail_node = None;
                        cx.notify();
                    });
                }
            }
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("node-detail-backdrop")
                .absolute()
                .inset_0()
                .bg(theme::OVERLAY_SCRIM)
                .gpui_test_observe(),
        )
        .child(
            div()
                .id("node-detail-dialog")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(dialog_width))
                .h(px(dialog_height))
                .flex()
                .flex_col()
                .rounded(px(theme::RADIUS_LG))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(header(this.clone(), &name))
                .child(tint_row)
                .child(
                    div()
                        .id("node-detail-body")
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .flex_col()
                        .child(body),
                )
                .gpui_test_observe(),
        )
        .into_any_element()
}

fn header(this: Entity<StudioShell>, name: &str) -> impl IntoElement {
    use controls::*;
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_2))
        .border_b_1()
        .border_color(theme::HAIRLINE)
        .child(
            div()
                .text_size(px(theme::TEXT_LG))
                .text_color(theme::TEXT)
                .child(name.to_owned()),
        )
        .child(
            button("node-detail-close", t!("Close")).on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.detail_node = None;
                    cx.notify();
                })
            }),
        )
}

/// The tint palette plus the automatic (category) swatch.
fn swatches(this: Entity<StudioShell>, node: NodeId, current: Option<u8>) -> impl IntoElement {
    let mut row = div().flex().items_center().gap(px(4.));
    let auto = div()
        .id("node-tint-auto")
        .size(px(16.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(if current.is_none() {
            theme::TEXT
        } else {
            theme::BORDER_STRONG
        })
        .when(current.is_none(), |v| {
            v.bg(theme::SURFACE_4).child(
                div()
                    .m(px(3.))
                    .size(px(8.))
                    .rounded(px(2.))
                    .bg(theme::PRIMARY.opacity(0.5)),
            )
        })
        .on_click({
            let this = this.clone();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::EditGraph(GraphEdit::SetNodeTint {
                            node,
                            tint: None,
                        }))
                    });
                    cx.notify();
                })
            }
        });
    row = row.child(auto);
    for index in 0..NODE_TINT_COUNT {
        let tint = theme::node_tint(index).expect("palette covers the count");
        let selected = current == Some(index);
        row = row.child(
            div()
                .id(format!("node-tint-{index}"))
                .size(px(16.))
                .rounded(px(theme::RADIUS_SM))
                .bg(tint)
                .when(selected, |v| v.border_1().border_color(theme::TEXT))
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(GraphEdit::SetNodeTint {
                                    node,
                                    tint: Some(index),
                                }))
                            });
                            cx.notify();
                        })
                    }
                }),
        );
    }
    row
}

fn eq_section(
    shell: &StudioShell,
    node: NodeId,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let settings = entry.builtin.eq().cloned().unwrap_or_default();
    let this = cx.entity();
    let editor = eq_editor(this, node, settings);
    let q = entry.builtin.eq().map(|s| s.q).unwrap_or(1.);
    let q_row = div()
        .id("eq-q-row")
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_2))
        .child(
            div()
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT_MUTED)
                .child(t!("Q")),
        )
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT)
                .child(format!("{q:.1}")),
        );
    let _ = shell;
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .id("eq-editor-host")
                .relative()
                .flex_1()
                .min_h_0()
                .mx(px(theme::SPACE_3))
                .my(px(theme::SPACE_2))
                .child(editor),
        )
        .child(q_row)
        .into_any_element()
}

/// The EQ curve editor: the frequency response with one draggable control
/// point. Dragging moves frequency (log x) and gain (linear y); the wheel
/// over the plot adjusts Q.
fn eq_editor(
    this: Entity<StudioShell>,
    node: NodeId,
    settings: moiren_engine::processor::EqSettings,
) -> impl IntoElement {
    let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let plot_bounds = bounds.clone();
    let plot_paint_settings = settings;
    let plot = canvas(
        move |bounds, _, _| {
            plot_bounds.set(bounds);
            bounds
        },
        move |bounds, _, window, _| {
            paint_eq_plot(bounds, &plot_paint_settings, window);
        },
    )
    .size_full();
    let apply = {
        let this = this.clone();
        let bounds = bounds.clone();
        move |position: gpui_kit::Point<Pixels>, window: &mut Window, cx: &mut gpui_kit::App| {
            let bounds = bounds.get();
            let width = f32::from(bounds.size.width).max(1.);
            let height = f32::from(bounds.size.height).max(1.);
            let x = (f32::from(position.x) - f32::from(bounds.origin.x)).clamp(0., width) / width;
            let y = (f32::from(position.y) - f32::from(bounds.origin.y)).clamp(0., height) / height;
            let frequency = EQ_FREQ_MIN * (EQ_FREQ_MAX / EQ_FREQ_MIN).powf(x as f64);
            let gain = EQ_GAIN_SPAN - y as f64 * 2. * EQ_GAIN_SPAN;
            this.update(cx, |shell, cx| {
                // Preserve Q from the freshest state the shell sees; the
                // editor snapshot may lag a frame behind.
                let q = shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(node)
                    .and_then(|entry| entry.builtin.eq().map(|s| s.q))
                    .unwrap_or(settings.q);
                shell.state.update(cx, |state, _| {
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                        node,
                        state: crate::service::BuiltinState::Eq(
                            moiren_engine::processor::EqSettings {
                                frequency_hz: frequency,
                                gain_db: gain,
                                q,
                            },
                        ),
                    }))
                });
                cx.notify();
            });
            let _ = window;
        }
    };
    let down = apply.clone();
    let move_apply = apply.clone();
    div()
        .id("eq-editor")
        .relative()
        .size_full()
        .rounded(px(theme::RADIUS_MD))
        .overflow_hidden()
        .bg(theme::WINDOW_BG)
        .child(plot)
        .child(
            div()
                .id("eq-editor-input")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    down(event.position, window, cx);
                    cx.stop_propagation();
                })
                .on_mouse_move(move |event, window, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        move_apply(event.position, window, cx);
                    }
                })
                .on_scroll_wheel({
                    let this = this.clone();
                    move |event: &ScrollWheelEvent, _, cx| {
                        let notches = match event.delta {
                            gpui_kit::ScrollDelta::Lines(lines) => lines.y,
                            gpui_kit::ScrollDelta::Pixels(delta) => f32::from(delta.y) / 60.,
                        };
                        this.update(cx, |shell, cx| {
                            let current = shell
                                .state
                                .read(cx)
                                .snapshot
                                .graph
                                .node(node)
                                .and_then(|entry| entry.builtin.eq().cloned())
                                .unwrap_or(settings);
                            let q = (current.q + notches as f64 * 0.1).clamp(0.1, 20.);
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                                    node,
                                    state: crate::service::BuiltinState::Eq(
                                        moiren_engine::processor::EqSettings { q, ..current },
                                    ),
                                }))
                            });
                            cx.notify();
                        });
                        cx.stop_propagation();
                    }
                })
                .gpui_test_observe(),
        )
}

/// Grid, response curve, and the control point at (frequency, gain).
fn paint_eq_plot(
    bounds: Bounds<Pixels>,
    settings: &moiren_engine::processor::EqSettings,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let x_of =
        |freq: f64| ((freq / EQ_FREQ_MIN).ln() / (EQ_FREQ_MAX / EQ_FREQ_MIN).ln()) as f32 * width;
    let y_of = |gain: f64| height / 2. - (gain as f32 / EQ_GAIN_SPAN as f32) * height / 2.;
    // Grid: 0 dB line, ±12 dB, and decade frequencies 100..10k.
    let mut line = |from: (f32, f32), to: (f32, f32), color| {
        let mut builder = gpui_kit::PathBuilder::stroke(px(1.));
        builder.move_to(point(
            px(f32::from(bounds.origin.x) + from.0),
            px(f32::from(bounds.origin.y) + from.1),
        ));
        builder.line_to(point(
            px(f32::from(bounds.origin.x) + to.0),
            px(f32::from(bounds.origin.y) + to.1),
        ));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    };
    for gain_db in [-12., 0., 12.] {
        let y = y_of(gain_db);
        line(
            (0., y),
            (width, y),
            if gain_db == 0. {
                theme::BORDER_STRONG
            } else {
                theme::HAIRLINE
            },
        );
    }
    for freq in [100., 1_000., 10_000.] {
        let x = x_of(freq);
        line((x, 0.), (x, height), theme::HAIRLINE);
    }
    // The response curve (curves.rs magnitude response).
    let mut builder = gpui_kit::PathBuilder::stroke(px(2.));
    let mut first = true;
    for step in 0..=96 {
        let fraction = step as f64 / 96.;
        let freq = EQ_FREQ_MIN * (EQ_FREQ_MAX / EQ_FREQ_MIN).powf(fraction);
        let response = super::super::super::pages::graph_canvas::curves::eq_magnitude_db(
            settings, 48_000., freq,
        );
        let p = point(
            px(f32::from(bounds.origin.x) + x_of(freq)),
            px(f32::from(bounds.origin.y) + y_of(response)),
        );
        if first {
            builder.move_to(p);
            first = false;
        } else {
            builder.line_to(p);
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, theme::TRACK_FILL);
    }
    // The control point.
    let px_x = x_of(settings.frequency_hz);
    let px_y = y_of(settings.gain_db);
    window.paint_quad(fill(
        Bounds {
            origin: point(
                bounds.origin.x + px(px_x - 6.),
                bounds.origin.y + px(px_y - 6.),
            ),
            size: size(px(12.), px(12.)),
        },
        theme::PRIMARY.opacity(0.3),
    ));
    window.paint_quad(fill(
        Bounds {
            origin: point(
                bounds.origin.x + px(px_x - 3.),
                bounds.origin.y + px(px_y - 3.),
            ),
            size: size(px(6.), px(6.)),
        },
        theme::TEXT,
    ));
}

fn dynamics_section(
    kind: moiren_core::graph::NodeKind,
    entry: &crate::service::GraphNodeState,
) -> AnyElement {
    let curve = match kind {
        moiren_core::graph::NodeKind::Gate => entry
            .builtin
            .gate()
            .map(pages::graph_canvas::curves::DynamicsCurve::gate),
        moiren_core::graph::NodeKind::Limiter => entry
            .builtin
            .limiter()
            .map(pages::graph_canvas::curves::DynamicsCurve::limiter),
        _ => Some(pages::graph_canvas::curves::DynamicsCurve::compressor(
            &entry.compressor,
        )),
    }
    .unwrap_or_else(|| pages::graph_canvas::curves::DynamicsCurve::compressor(&Default::default()));
    let note = t!("Full parameters live in the inspector");
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .id("dynamics-curve")
                .flex_1()
                .min_h_0()
                .m(px(theme::SPACE_3))
                .rounded(px(theme::RADIUS_MD))
                .bg(theme::WINDOW_BG)
                .child(
                    canvas(
                        move |bounds, _, _| bounds,
                        move |bounds, _, window, _| {
                            paint_dynamics_plot(bounds, curve, window);
                        },
                    )
                    .size_full(),
                ),
        )
        .child(
            div()
                .px(px(theme::SPACE_3))
                .pb(px(theme::SPACE_2))
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_FAINT)
                .child(note),
        )
        .into_any_element()
}

/// The static transfer curve at detail scale: unity reference, the
/// curve, and a threshold tick.
fn paint_dynamics_plot(
    bounds: Bounds<Pixels>,
    curve: pages::graph_canvas::curves::DynamicsCurve,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let span = 60.;
    let to_x = |db: f64| ((db + span) / (2. * span)) as f32 * width;
    let to_y = |db: f64| height - ((db + span) / (2. * span)) as f32 * height;
    let mut line = |from: (f64, f64), to: (f64, f64), color| {
        let mut builder = gpui_kit::PathBuilder::stroke(px(1.));
        builder.move_to(point(
            px(f32::from(bounds.origin.x) + to_x(from.0)),
            px(f32::from(bounds.origin.y) + to_y(from.1)),
        ));
        builder.line_to(point(
            px(f32::from(bounds.origin.x) + to_x(to.0)),
            px(f32::from(bounds.origin.y) + to_y(to.1)),
        ));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    };
    line((-span, -span), (span, span), theme::HAIRLINE);
    line(
        (curve.threshold_db, -span),
        (curve.threshold_db, span),
        theme::BORDER_STRONG,
    );
    let mut builder = gpui_kit::PathBuilder::stroke(px(2.));
    let mut first = true;
    for (input, output) in pages::graph_canvas::curves::sample_curve_linear_x(
        |db| curve.transfer_db(db),
        -span,
        span,
        96,
    ) {
        let p = point(
            px(f32::from(bounds.origin.x) + to_x(input)),
            px(f32::from(bounds.origin.y) + to_y(output)),
        );
        if first {
            builder.move_to(p);
            first = false;
        } else {
            builder.line_to(p);
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, theme::TRACK_FILL);
    }
}
