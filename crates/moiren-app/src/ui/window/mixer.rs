use super::*;
use crate::service::{GraphEdit, GraphNodeState, MixerState};
use controls::*;
use moiren_core::graph::{EdgeId, NodeId, NodeKind, SendParams, SendTap};

mod model;
pub(super) use model::MixerPresentation;
use model::{StripDrag, gain_db, reorder};

impl StudioShell {
    pub(super) fn update_node_meters(&mut self, visible: bool, cx: &App) -> bool {
        let mut nodes = if visible {
            self.node_meter_points.borrow().clone()
        } else {
            Default::default()
        };
        if visible
            && self.inspector_open
            && let Some(selected) = self.selected_node
            && self
                .state
                .read(cx)
                .snapshot
                .graph
                .nodes
                .iter()
                .any(|node| node.id == selected)
        {
            nodes.insert(selected);
        }
        self.node_meters
            .update(&self.state.read(cx).snapshot, &nodes)
    }
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let graph = &state.snapshot.graph;
    let mut channels = div()
        .id("mixer-strips")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .track_scroll(&shell.mixer_scroll)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    let mut dock = div().flex().flex_none().min_w_full().h_full().justify_end();
    let dock_container = div()
        .id("mixer-dock")
        .flex()
        .flex_none()
        .w(px(shell.mixer.dock_width))
        .max_w(relative(0.55))
        .min_h_0()
        .border_l_1()
        .border_color(theme::BORDER_STRONG)
        .overflow_x_scroll()
        .overflow_y_scroll()
        .on_scroll_wheel({
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |_, cx| cx.notify())
        });
    for id in &shell.mixer.order {
        let Ok(node) = graph.graph.get_node(*id) else {
            continue;
        };
        let entry = graph.node(*id).expect("service metadata");
        let strip = strip(shell, entry, node.kind(), node.channels(), cx);
        if matches!(node.kind(), NodeKind::Bus | NodeKind::Sink) {
            dock = dock.child(strip);
        } else {
            channels = channels.child(strip);
        }
    }
    div()
        .id("mixer-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .on_mouse_move({
            let this = cx.entity();
            move |event, _, cx| {
                this.update(cx, |shell, cx| {
                    if event.pressed_button == Some(MouseButton::Left)
                        && let Some((docked, origin, width)) = shell.mixer.resize
                    {
                        let delta = f32::from(event.position.x) - origin;
                        if docked {
                            shell.mixer.dock_width = (width - delta).clamp(160., 640.);
                        } else {
                            shell.mixer.width = (width + delta).clamp(136., 280.);
                        }
                        cx.notify();
                    }
                });
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let this = cx.entity();
            move |_, _, cx| this.update(cx, |shell, _| shell.mixer.resize = None)
        })
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(48.))
                .items_center()
                .justify_between()
                .px(px(16.))
                .child(heading("Mixer"))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(apply(shell, cx))
                        .child(inspector_toggle("toggle-details", cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(channels.gpui_test_observe())
                .child(resize_handle(shell, true, "mixer-dock-resize", cx))
                .child(dock_container.child(dock).gpui_test_observe())
                .when(shell.inspector_open, |view| {
                    view.child(graph::inspector(shell, cx))
                }),
        )
        .child(
            div()
                .flex_none()
                .px(px(16.))
                .py(px(8.))
                .border_t_1()
                .border_color(theme::BORDER)
                .child(label("Drag headers to reorder · drag dividers to resize")),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn resize_handle(
    shell: &StudioShell,
    docked: bool,
    id: impl Into<ElementId>,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let width = if docked {
        shell.mixer.dock_width
    } else {
        shell.mixer.width
    };
    let this = cx.entity();
    div()
        .id(id)
        .w(px(6.))
        .h_full()
        .flex_none()
        .bg(theme::SURFACE)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            this.update(cx, |shell, _| {
                shell.mixer.resize = Some((docked, f32::from(event.position.x), width))
            });
        })
        .gpui_test_observe()
}

fn strip(
    shell: &StudioShell,
    entry: &GraphNodeState,
    kind: NodeKind,
    channels: usize,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let id = entry.id;
    let selected = shell.selected_node == Some(id);
    let color = match kind {
        NodeKind::Source => theme::NODE_SOURCE,
        NodeKind::Sink => theme::NODE_SINK,
        NodeKind::Bus => theme::NODE_UTILITY,
        _ => theme::NODE_DSP,
    };
    let title = div()
        .id(SharedString::from(format!("strip-header-{id:?}")))
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .cursor_pointer()
        .child(heading(entry.label.clone()))
        .child(label(format!("{kind:?} · {channels} ch")))
        .on_click({
            let this = cx.entity();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.selected_node = Some(id);
                    shell.inspector_open = true;
                    cx.notify();
                })
            }
        })
        .on_drag(id, move |_, _, _, cx| cx.new(|_| StripDrag))
        .gpui_test_observe();
    div()
        .id(SharedString::from(format!("strip-{id:?}")))
        .flex()
        .flex_none()
        .w(px(shell.mixer.width))
        .min_h(px(720.))
        .bg(if selected {
            theme::SURFACE_3
        } else {
            theme::SURFACE
        })
        .border_r_1()
        .border_color(theme::BORDER)
        .on_drop({
            let this = cx.entity();
            move |dragged: &NodeId, _, cx| {
                this.update(cx, |shell, cx| {
                    let graph = &shell.state.read(cx).snapshot.graph.graph;
                    let docked = |id| {
                        graph
                            .get_node(id)
                            .is_ok_and(|node| matches!(node.kind(), NodeKind::Bus | NodeKind::Sink))
                    };
                    if docked(*dragged) == docked(id) {
                        reorder(&mut shell.mixer.order, *dragged, id);
                    }
                    cx.notify();
                })
            }
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().h(px(3.)).bg(color.opacity(0.55)))
                .child(title)
                .child(sends(shell, id, cx))
                .child(
                    div()
                        .flex_none()
                        .px(px(10.))
                        .py(px(8.))
                        .child(node_controls(entry, channels, cx)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("strip-levels-{id:?}")))
                        .flex()
                        .flex_none()
                        .justify_center()
                        .gap(px(4.))
                        .py(px(12.))
                        .child(scale())
                        .child(fader(&shell.mixer.faders[&id], entry.mixer.gain))
                        .child(meter(shell, entry, kind, cx))
                        .gpui_test_observe(),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("strip-readout-{id:?}")))
                        .flex_none()
                        .px(px(12.))
                        .pb(px(12.))
                        .child(heading(db(entry.mixer.gain)))
                        .gpui_test_observe(),
                ),
        )
        .child(resize_handle(
            shell,
            false,
            SharedString::from(format!("strip-resize-{id:?}")),
            cx,
        ))
        .gpui_test_observe()
}

pub(super) fn node_controls(
    entry: &GraphNodeState,
    channels: usize,
    cx: &Context<StudioShell>,
) -> Div {
    let id = entry.id;
    let requested = entry.mixer;
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    button(SharedString::from(format!("strip-mute-{id:?}")), "M")
                        .variant(if requested.muted {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(cx.entity(), id, |state| {
                            state.muted = !state.muted
                        })),
                )
                .child(
                    button(SharedString::from(format!("strip-solo-{id:?}")), "S")
                        .variant(if requested.solo {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(mixer_edit(cx.entity(), id, |state| {
                            state.solo = !state.solo
                        })),
                ),
        )
        .child(
            button(SharedString::from(format!("strip-dim-{id:?}")), "DIM −12")
                .variant(if requested.dimmed {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(mixer_edit(cx.entity(), id, |state| {
                    state.dimmed = !state.dimmed
                })),
        )
        .child(
            button(
                SharedString::from(format!("strip-mode-{id:?}")),
                requested.mode_label(channels),
            )
            .on_click(mixer_edit(cx.entity(), id, move |state| {
                *state = state.next_mode(channels)
            })),
        )
}

fn mixer_edit(
    this: Entity<StudioShell>,
    id: NodeId,
    change: impl Fn(&mut MixerState) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                if let Some(node) = state.snapshot.graph.node(id) {
                    let mut requested = node.mixer;
                    change(&mut requested);
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetMixer {
                        node: id,
                        state: requested,
                    }));
                }
            });
            cx.notify();
        })
    }
}

fn sends(shell: &StudioShell, id: NodeId, cx: &Context<StudioShell>) -> impl IntoElement {
    let graph = &shell.state.read(cx).snapshot.graph;
    let mut rows = div()
        .id(SharedString::from(format!("sends-{id:?}")))
        .flex()
        .flex_col()
        .gap(px(6.))
        .h(px(130.))
        .flex_none()
        .px(px(10.))
        .py(px(8.))
        .border_y_1()
        .border_color(theme::BORDER)
        .overflow_y_scroll()
        .child(label("SENDS"));
    let mut count = 0;
    for edge in graph.graph.edges().iter().filter(|edge| edge.src() == id) {
        count += 1;
        let edge_id = edge.id();
        let pre = edge.params().tap == SendTap::PreFader;
        rows = rows.child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(label(
                            graph
                                .node(edge.dst())
                                .map(|node| node.label.clone())
                                .unwrap_or_default(),
                        ))
                        .child(
                            button(
                                SharedString::from(format!("send-pre-{edge_id:?}")),
                                if pre { "PRE" } else { "POST" },
                            )
                            .variant(if pre {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click({
                                let this = cx.entity();
                                move |_, _, cx| {
                                    this.update(cx, |shell, cx| {
                                        shell.state.update(cx, |state, _| {
                                            if let Ok(edge) =
                                                state.snapshot.graph.graph.get_edge(edge_id)
                                            {
                                                let params = SendParams {
                                                    tap: if edge.params().tap == SendTap::PreFader {
                                                        SendTap::PostFader
                                                    } else {
                                                        SendTap::PreFader
                                                    },
                                                    ..*edge.params()
                                                };
                                                state.dispatch(AppRequest::EditGraph(
                                                    GraphEdit::SetSend {
                                                        edge: edge_id,
                                                        params,
                                                    },
                                                ));
                                            }
                                        });
                                        cx.notify();
                                    })
                                }
                            }),
                        ),
                )
                .child(label(db(edge.params().gain)))
                .child(component::slider::Slider::new(&shell.mixer.sends[&edge_id])),
        );
    }
    if count == 0 {
        rows = rows.child(label("No sends · connect in Graph"));
    }
    rows.gpui_test_observe()
}

fn meter(
    shell: &StudioShell,
    entry: &GraphNodeState,
    kind: NodeKind,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let state = shell.state.read(cx);
    let input = kind == NodeKind::Source;
    let eligible = matches!(kind, NodeKind::Source | NodeKind::Sink)
        && state.snapshot.graph.active_revision == Some(state.snapshot.graph.candidate_revision)
        && shell
            .observation_nodes
            .is_some_and(|(generation, source, sink)| {
                generation == state.snapshot.generation
                    && entry.id == if input { source } else { sink }
            })
        && state.snapshot.desired.as_ref().is_some_and(|active| {
            if input {
                entry.source.as_ref() == Some(&active.source)
            } else {
                entry.output_endpoint_id.as_ref() == Some(&active.output_endpoint_id)
            }
        });
    let levels = eligible
        .then_some({
            if input {
                shell.meters.input
            } else {
                shell.meters.output
            }
        })
        .flatten();
    let levels = (state.snapshot.graph.active_revision
        == Some(state.snapshot.graph.candidate_revision))
    .then(|| shell.node_meters.levels(entry.id))
    .flatten()
    .or(levels);
    let clips = shell.node_meters.clips(entry.id);
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&entry.id)
        && state.snapshot.graph.active_revision == Some(state.snapshot.graph.candidate_revision);
    div()
        .id(SharedString::from(format!("meter-reset-{:?}", entry.id)))
        .on_mouse_down(MouseButton::Left, {
            let this = cx.entity();
            let id = entry.id;
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.node_meters.reset_clip(id);
                    cx.notify();
                })
            }
        })
        .relative()
        .when(managed, |view| {
            let points = shell.node_meter_points.clone();
            let id = entry.id;
            view.child(
                canvas(
                    move |bounds, window, _| {
                        let intersection = bounds.intersect(&window.content_mask().bounds);
                        if intersection.size.width > px(0.) && intersection.size.height > px(0.) {
                            points.borrow_mut().insert(id);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
        })
        .when(eligible, |view| {
            view.child(meter_probe(shell.meter_points.clone(), input))
        })
        .child(crate::ui::widgets::meters::stereo_meter_with_clip(
            levels, clips,
        ))
        .gpui_test_observe()
}

fn scale() -> Div {
    let mut view = div()
        .relative()
        .flex_none()
        .w(px(18.))
        .h(px(230.))
        .font_family(theme::FONT_MONO)
        .text_size(px(9.))
        .text_color(theme::TEXT_FAINT);
    for db in [24, 12, 0, -6, -12, -24, -48, -60] {
        view = view.child(
            div()
                .absolute()
                .top(px((24. - db as f32) / 84. * 216.))
                .child(if db == -60 {
                    "−∞".into()
                } else {
                    db.to_string()
                }),
        );
    }
    view
}

fn fader(state: &Entity<base::slider::SliderState>, gain: f64) -> impl IntoElement {
    let normalized = (gain_db(gain) + 60.) / 84.;
    base::slider::Slider::new(state)
        .vertical()
        .relative()
        .flex_none()
        .w(px(26.))
        .h(px(230.))
        .child(
            base::slider::SliderTrack::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(12.))
                .top(px(7.))
                .w(px(3.))
                .h(px(216.))
                .bg(theme::BORDER_STRONG),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(1.))
                .top(px((1. - normalized) * 216.))
                .w(px(24.))
                .h(px(14.))
                .rounded(px(3.))
                .bg(theme::TEXT_MUTED)
                .child(div().mt(px(6.)).w_full().h(px(2.)).bg(theme::SURFACE)),
        )
}

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
                flags.0 = visible;
            } else {
                flags.1 = visible;
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
