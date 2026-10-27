//! Static transfer functions, never synthetic realtime observations.
use super::*;
use std::cell::Cell;

#[derive(Clone)]
struct PlotDrag {
    id: NodeId,
    kind: NodeKind,
    cursor: Rc<Cell<Point<Pixels>>>,
    values: Rc<Cell<(f64, f64)>>,
}
impl Render for PlotDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(super) fn response(
    kind: NodeKind,
    node: &GraphNodeState,
    sample_rate: f64,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let values: Vec<f64> = match kind {
        NodeKind::Eq => {
            let settings = *node.builtin.eq()?;
            (0..128)
                .map(|i| eq_db(settings, 20. * 1000f64.powf(i as f64 / 127.), sample_rate))
                .collect()
        }
        NodeKind::Compressor => (0..128)
            .map(|i| {
                let input = -120. + i as f64 / 127. * 144.;
                compressor_db(node.compressor, input)
            })
            .collect(),
        _ => return None,
    };
    let eq = kind == NodeKind::Eq;
    let node_compressor = node.compressor;
    let id = node.id;
    let initial = if eq {
        let s = node.builtin.eq()?;
        (
            (s.frequency_hz / 20.).ln() / 1000f64.ln(),
            (24. - s.gain_db) / 48.,
        )
    } else {
        (
            (node.compressor.threshold_db + 120.) / 144.,
            node.compressor.ratio.ln() / 100f64.ln(),
        )
    };
    let measured = Rc::new(Cell::new(Bounds::default()));
    let drag_bounds = measured.clone();
    let drag = PlotDrag {
        id,
        kind,
        cursor: Rc::new(Cell::new(point(px(0.), px(0.)))),
        values: Rc::new(Cell::new(initial)),
    };
    let this = cx.entity();
    let wheel_this = this.clone();
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(note(if eq {
                "EQ response · 20 Hz — 20 kHz"
            } else {
                "Static transfer · input / output dB"
            }))
            .child(
                div()
                    .id(format!("card-response-{id:?}"))
                    .w_full()
                    .h(px(112.))
                    .relative()
                    .cursor_pointer()
                    .on_drag(drag, |drag, _, window, cx| {
                        let drag = drag.clone();
                        drag.cursor.set(window.mouse_position());
                        cx.new(|_| drag)
                    })
                    .on_drag_move(move |event: &DragMoveEvent<PlotDrag>, _, cx| {
                        let drag = event.drag(cx);
                        if drag.id != id {
                            return;
                        }
                        let bounds = drag_bounds.get();
                        let previous = drag.cursor.replace(event.event.position);
                        let scale = if event.event.modifiers.shift { 0.1 } else { 1. };
                        let dx = f64::from(
                            f32::from(event.event.position.x - previous.x)
                                / f32::from(bounds.size.width).max(1.),
                        ) * scale;
                        let dy = f64::from(
                            f32::from(event.event.position.y - previous.y)
                                / f32::from(bounds.size.height).max(1.),
                        ) * scale;
                        let (x, y) = drag.values.get();
                        let next = (
                            (x + dx).clamp(0., 1.),
                            (y + if eq { dy } else { -dy }).clamp(0., 1.),
                        );
                        drag.values.set(next);
                        edit_plot(&this, id, drag.kind, next, cx);
                    })
                    .on_scroll_wheel(move |event, _, cx| {
                        if !eq {
                            return;
                        }
                        cx.stop_propagation();
                        let delta = match event.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(p) => p.y,
                        };
                        if delta == 0. {
                            return;
                        }
                        wheel_this.update(cx, |shell, cx| {
                            let graph = &shell.state.read(cx).snapshot.graph;
                            if let Some(s) = graph.node(id).and_then(|n| n.builtin.eq()) {
                                let step = if event.modifiers.shift { 0.01 } else { 0.1 };
                                let mut settings = *s;
                                settings.q = (settings.q
                                    * f64::exp(f64::from(delta.signum()) * step))
                                .clamp(0.1, 20.);
                                shell.state.update(cx, |state, _| {
                                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                                        node: id,
                                        state: crate::service::BuiltinState::Eq(settings),
                                    }))
                                });
                            }
                            cx.notify();
                        });
                    })
                    .tooltip(move |window, cx| {
                        component::tooltip::Tooltip::new(
                            t!(if eq {
                                "Drag band: frequency / gain · wheel: Q · Shift: fine"
                            } else {
                                "Drag curve: threshold / ratio · Shift: fine"
                            })
                            .to_string(),
                        )
                        .build(window, cx)
                    })
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                measured.set(bounds);
                                values
                            },
                            move |bounds, values, window, _| {
                                window.paint_quad(fill(bounds, theme::WINDOW_BG));
                                for i in 1..4 {
                                    let mut line = bounds;
                                    line.origin.y += bounds.size.height * i as f32 / 4.;
                                    line.size.height = px(1.);
                                    window.paint_quad(fill(line, theme::BORDER));
                                }
                                for i in 1..6 {
                                    let mut line = bounds;
                                    line.origin.x += bounds.size.width * i as f32 / 6.;
                                    line.size.width = px(1.);
                                    window.paint_quad(fill(line, theme::BORDER.opacity(0.5)));
                                }
                                let mut path = PathBuilder::stroke(px(1.7));
                                for (i, value) in values.iter().enumerate() {
                                    let x = i as f32 / (values.len() - 1) as f32;
                                    let y = if eq {
                                        ((24. - value) / 48.) as f32
                                    } else {
                                        ((24. - value) / 144.) as f32
                                    };
                                    let p = bounds.origin
                                        + point(
                                            bounds.size.width * x,
                                            bounds.size.height * y.clamp(0., 1.),
                                        );
                                    if i == 0 {
                                        path.move_to(p)
                                    } else {
                                        path.line_to(p)
                                    }
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, theme::SIGNAL_DEVICE);
                                }
                                let x = initial.0.clamp(0., 1.) as f32;
                                let y = if eq {
                                    initial.1 as f32
                                } else {
                                    ((24.
                                        - compressor_db(
                                            node_compressor,
                                            -120. + f64::from(x) * 144.,
                                        ))
                                        / 144.) as f32
                                };
                                let marker = bounds.origin
                                    + point(bounds.size.width * x, bounds.size.height * y);
                                window.paint_quad(PaintQuad {
                                    bounds: Bounds::new(
                                        marker - point(px(4.), px(4.)),
                                        size(px(8.), px(8.)),
                                    ),
                                    corner_radii: px(4.).into(),
                                    background: theme::SIGNAL_DEVICE.into(),
                                    border_widths: px(1.).into(),
                                    border_color: theme::TEXT,
                                    border_style: BorderStyle::default(),
                                });
                            },
                        )
                        .size_full(),
                    )
                    .gpui_test_observe(),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(9.))
                    .text_color(theme::TEXT_FAINT)
                    .children(if eq {
                        vec!["20 Hz", "632 Hz", "20 kHz"]
                    } else {
                        vec!["-120 dB", "-48 dB", "+24 dB"]
                    }),
            )
            .into_any_element(),
    )
}

fn edit_plot(
    this: &Entity<StudioShell>,
    id: NodeId,
    kind: NodeKind,
    value: (f64, f64),
    cx: &mut App,
) {
    this.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let Some(node) = graph.node(id) else { return };
        let edits = if kind == NodeKind::Eq {
            let Some(mut settings) = node.builtin.eq().copied() else {
                return;
            };
            settings.frequency_hz = 20. * 1000f64.powf(value.0);
            settings.gain_db = 24. - value.1 * 48.;
            vec![GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::Eq(settings),
            }]
        } else {
            vec![
                GraphEdit::SetCompressor {
                    node: id,
                    parameter: crate::service::CompressorParameter::Threshold,
                    value: -120. + value.0 * 144.,
                },
                GraphEdit::SetCompressor {
                    node: id,
                    parameter: crate::service::CompressorParameter::Ratio,
                    value: 100f64.powf(value.1),
                },
            ]
        };
        for edit in edits {
            shell
                .state
                .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
        }
        cx.notify();
    });
}

/// Steady-state gain follows the engine's amplitude-domain dry/wet mix.
/// Envelope timing and instantaneous gain reduction are deliberately not shown.
fn compressor_db(s: crate::service::CompressorState, input: f64) -> f64 {
    let over = input + s.input_gain_db - s.threshold_db;
    let reduction = if s.knee_db > 0. && over.abs() <= s.knee_db / 2. {
        (1. / s.ratio - 1.) * (over + s.knee_db / 2.).powi(2) / (2. * s.knee_db)
    } else if over > 0. {
        (1. / s.ratio - 1.) * over
    } else {
        0.
    };
    let wet = 10f64.powf((s.input_gain_db + reduction + s.makeup_gain_db) / 20.);
    input + 20. * ((1. - s.mix) + s.mix * wet).log10() + s.output_gain_db
}

/// RBJ peaking response uses the same gain/Q convention as the engine biquad.
fn eq_db(s: moiren_engine::processor::EqSettings, frequency: f64, sample_rate: f64) -> f64 {
    let rate = sample_rate.max(1.);
    let w0 = std::f64::consts::TAU * s.frequency_hz.min(rate * 0.49) / rate;
    let a = 10f64.powf(s.gain_db / 40.);
    let alpha = w0.sin() / (2. * s.q);
    let b = [1. + alpha * a, -2. * w0.cos(), 1. - alpha * a];
    let d = [1. + alpha / a, -2. * w0.cos(), 1. - alpha / a];
    let w = std::f64::consts::TAU * frequency.min(rate * 0.49) / rate;
    let magnitude = |c: [f64; 3]| {
        let re = c[0] + c[1] * w.cos() + c[2] * (2. * w).cos();
        let im = -c[1] * w.sin() - c[2] * (2. * w).sin();
        re * re + im * im
    };
    10. * (magnitude(b) / magnitude(d).max(f64::MIN_POSITIVE))
        .max(f64::MIN_POSITIVE)
        .log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn eq_plot_is_unity_at_zero_gain_and_matches_center_gain() {
        let mut s = moiren_engine::processor::EqSettings::default();
        for f in [20., 1000., 20000.] {
            assert!(eq_db(s, f, 48000.).abs() < 1e-8);
        }
        s.gain_db = 12.;
        assert!((eq_db(s, 1000., 48000.) - 12.).abs() < 1e-8);
    }

    #[::core::prelude::v1::test]
    fn compressor_plot_tracks_dry_mix_input_makeup_and_output_gains() {
        let mut s = crate::service::CompressorState {
            mix: 0.,
            output_gain_db: 3.,
            ..Default::default()
        };
        assert!((compressor_db(s, -6.) + 3.).abs() < 1e-8);
        s.mix = 1.;
        s.ratio = 1.;
        s.input_gain_db = 2.;
        s.makeup_gain_db = 4.;
        assert!((compressor_db(s, -20.) + 11.).abs() < 1e-8);
    }
}
