//! One strip per real graph node. Only source/sink strips own observed meters.
use super::*;
use controls::*;
use moiren_core::graph::NodeKind;

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let graph = &state.snapshot.graph;
    let first_gain = graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Gain)
        .map(|n| n.id());
    let first_pan = graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Pan)
        .map(|n| n.id());
    let mut strips = div()
        .id("mixer-strips")
        .flex()
        .flex_1()
        .min_h_0()
        .track_scroll(&shell.mixer_scroll)
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        })
        .overflow_x_scroll()
        .overflow_y_scroll();
    for node in graph.graph.nodes() {
        let entry = graph.node(node.id()).expect("service metadata");
        let key = format!("{:?}", node.id());
        let selected = shell.selected_node.as_ref() == Some(&key);
        let color = match node.kind() {
            NodeKind::Source => theme::SIGNAL_DEVICE,
            NodeKind::Sink => theme::SIGNAL_RECORDER,
            _ => theme::PRIMARY,
        };
        let this = cx.entity();
        let mut strip = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(if shell.density == Density::Compact {
                150.
            } else {
                170.
            }))
            .min_h(px(550.))
            .bg(if selected {
                theme::SURFACE_3
            } else {
                theme::SURFACE
            })
            .border_r_1()
            .border_color(theme::BORDER)
            .child(div().h(px(3.)).bg(color))
            .child(
                div()
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        button(
                            SharedString::from(format!("select-{key}")),
                            entry.label.clone(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            this.update(cx, |s, cx| {
                                s.selected_node = Some(key.clone());
                                s.inspector_open = true;
                                cx.notify();
                            })
                        }),
                    )
                    .child(label(format!("{:?} · {} ch", node.kind(), node.channels())))
                    .child(label(format!(
                        "{} in / {} out",
                        node.inputs().len(),
                        node.outputs().len()
                    ))),
            );
        if Some(node.id()) == first_pan {
            strip = strip.child(
                div()
                    .px(px(12.))
                    .h(px(66.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(detail("Balance", format!("{:+.2}", state.pan)))
                    .child(horizontal_slider(&shell.pan_slider, state.pan as f32)),
            );
        } else {
            strip = strip.child(div().px(px(12.)).h(px(66.)).child(label(match node.kind() {
                NodeKind::Compressor => "Linked peak · default settings",
                NodeKind::Bus => "Sum connected inputs",
                _ => "Direct graph stream",
            })));
        }
        let body = if Some(node.id()) == first_gain {
            div()
                .h(px(250.))
                .flex()
                .justify_center()
                .gap(px(8.))
                .child(scale())
                .child(fader(&shell.gain_slider, state.gain))
                .into_any_element()
        } else if node.kind() == NodeKind::Source || node.kind() == NodeKind::Sink {
            let input = node.kind() == NodeKind::Source;
            let identity_matches =
                shell
                    .observation_nodes
                    .is_some_and(|(generation, source, sink)| {
                        generation == state.snapshot.generation
                            && node.id() == if input { source } else { sink }
                    });
            let binding_matches = state.snapshot.desired.as_ref().is_some_and(|active| {
                if input {
                    entry.source.as_ref() == Some(&active.source)
                } else {
                    entry.output_endpoint_id.as_ref() == Some(&active.output_endpoint_id)
                }
            });
            let eligible = identity_matches && binding_matches;
            let (levels, status) = if !eligible {
                (None, "Candidate · no active observation".to_string())
            } else if input {
                (
                    shell.meters.input,
                    shell.meters.input_status.label().to_string(),
                )
            } else {
                (
                    shell.meters.output,
                    shell.meters.output_status.label().to_string(),
                )
            };
            div()
                .relative()
                .h(px(250.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .when(eligible, |v| {
                    v.child(meter_probe(shell.meter_points.clone(), input))
                })
                .child(crate::ui::widgets::meters::stereo_meter(levels))
                .child(label(status))
                .into_any_element()
        } else {
            div()
                .h(px(250.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(label("No meter at this node"))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_color(theme::TEXT_FAINT)
                        .child("—"),
                )
                .when(node.kind() == NodeKind::Gain, |v| {
                    v.child(label(db(entry.gain)))
                })
                .into_any_element()
        };
        strip = strip.child(body).child(
            div()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(if Some(node.id()) == first_gain {
                    heading(db(state.effective_gain()))
                } else {
                    heading(entry.label.clone())
                })
                .child(badge(
                    if graph.active_revision == Some(graph.candidate_revision) {
                        "Active"
                    } else {
                        "Candidate"
                    },
                    if graph.active_revision == Some(graph.candidate_revision) {
                        theme::SUCCESS
                    } else {
                        theme::WARNING
                    },
                )),
        );
        if Some(node.id()) == first_gain {
            strip = strip.child(
                div()
                    .px(px(12.))
                    .flex()
                    .gap(px(6.))
                    .child(
                        button("mix-mute", "Mute")
                            .variant(if state.muted {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click({
                                let this = cx.entity();
                                move |_, _, cx| {
                                    this.update(cx, |s, cx| {
                                        s.state.update(cx, |state, _| {
                                            state.muted = !state.muted;
                                            state.request_parameters();
                                        });
                                        cx.notify();
                                    })
                                }
                            }),
                    )
                    .child(
                        button("mix-dim", "Dim")
                            .variant(if state.dimmed {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click({
                                let this = cx.entity();
                                move |_, _, cx| {
                                    this.update(cx, |s, cx| {
                                        s.state.update(cx, |state, _| {
                                            state.dimmed = !state.dimmed;
                                            state.request_parameters();
                                        });
                                        cx.notify();
                                    })
                                }
                            }),
                    ),
            );
        }
        if Some(node.id()) == first_pan {
            strip = strip.child(
                div()
                    .px(px(12.))
                    .pt(px(8.))
                    .child(stereo_controls(entry, cx)),
            );
        }
        strips = strips.child(strip);
    }
    div()
        .id("mixer-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(48.))
                .items_center()
                .justify_between()
                .px(px(16.))
                .child(heading("Mixer"))
                .child(graph_summary(state))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(apply(shell, cx))
                        .child(inspector_toggle(cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(strips)
                .when(shell.inspector_open, |v| {
                    v.child(graph::inspector(shell, cx))
                }),
        )
        .child(
            div()
                .flex_none()
                .px(px(16.))
                .py(px(8.))
                .child(label(state.control_feedback())),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn scale() -> Div {
    let mut view = div()
        .relative()
        .w(px(30.))
        .h(px(230.))
        .font_family(theme::FONT_MONO)
        .text_size(px(10.))
        .text_color(theme::TEXT_FAINT);
    for db in [0, -6, -12, -24, -48, -60] {
        view = view.child(div().absolute().top(px(((-db as f32) / 60.) * 216.)).child(
            if db == -60 {
                "−∞".into()
            } else {
                db.to_string()
            },
        ));
    }
    view
}

fn fader(state: &Entity<base::slider::SliderState>, gain: f64) -> impl IntoElement {
    let normalized = if gain <= 0. {
        0.
    } else {
        ((20. * gain.log10() + 60.) / 60.).clamp(0., 1.) as f32
    };
    base::slider::Slider::new(state)
        .vertical()
        .relative()
        .w(px(45.))
        .h(px(230.))
        .child(
            base::slider::SliderTrack::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(21.))
                .top(px(7.))
                .w(px(3.))
                .h(px(216.))
                .bg(theme::BORDER_STRONG),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(6.))
                .top(px((1. - normalized) * 216.))
                .w(px(31.))
                .h(px(14.))
                .rounded(px(3.))
                .bg(theme::TEXT_MUTED)
                .child(div().mt(px(6.)).w_full().h(px(2.)).bg(theme::SURFACE)),
        )
}

fn horizontal_slider(state: &Entity<base::slider::SliderState>, pan: f32) -> impl IntoElement {
    base::slider::Slider::new(state)
        .relative()
        .w_full()
        .h(px(24.))
        .child(
            base::slider::SliderTrack::new(state)
                .absolute()
                .top(px(11.))
                .w_full()
                .h(px(3.))
                .bg(theme::BORDER_STRONG),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .absolute()
                .left(relative((pan + 1.) / 2.))
                .ml(px(-6.))
                .top(px(5.))
                .size(px(12.))
                .rounded_full()
                .bg(theme::PRIMARY),
        )
}

/// Prepaint sees the real clipping mask of horizontal/vertical scroll ancestors.
/// The UI clock consumes these flags after layout, so no hidden strip owns demand.
fn meter_probe(
    points: std::rc::Rc<std::cell::Cell<(bool, bool)>>,
    input: bool,
) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let intersection = bounds.intersect(&window.content_mask().bounds);
            let visible = intersection.size.width > px(0.) && intersection.size.height > px(0.);
            let mut flags = points.get();
            if input {
                flags.0 = visible
            } else {
                flags.1 = visible
            }
            points.set(flags);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
