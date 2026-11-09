//! Routing rows retain graph edge/port identity, including parallel connections.
use super::*;
use crate::service::GraphEdit;
use crate::ui::widgets::fader::hslider;
use crate::ui::widgets::overlay_scrollbar::{notify_owner, scroll_region};
use gpui_kit::base::ScrollbarAxis;

fn section(
    id: String,
    title: &'static str,
    height: f32,
    collapsed: bool,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let this = cx.entity();
    div()
        .id(SharedString::from(id))
        .flex()
        .flex_col()
        .flex_none()
        .h(px(if collapsed {
            theme::SECTION_HEADER_HEIGHT
        } else {
            height
        }))
        .border_b_1()
        .border_color(theme::BORDER)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .h(px(theme::SECTION_HEADER_HEIGHT))
                .flex_none()
                .child(
                    button(SharedString::from(format!("toggle-{title}")), t!(title))
                        .full_width()
                        .justify_start()
                        .square()
                        .trailing(if collapsed { "▸" } else { "▾" })
                        .density(Density::Compact)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            this.update(cx, |shell, cx| {
                                if title == "INPUTS" {
                                    shell.mixer.presentation.inputs_collapsed =
                                        !shell.mixer.presentation.inputs_collapsed;
                                } else {
                                    shell.mixer.presentation.sends_collapsed =
                                        !shell.mixer.presentation.sends_collapsed;
                                }
                                cx.notify();
                            })
                        }),
                ),
        )
}

fn select_route(
    this: Entity<StudioShell>,
    ports: (moiren_core::graph::PortId, moiren_core::graph::PortId),
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            if shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .edges()
                .iter()
                .any(|edge| (edge.src_port(), edge.dst_port()) == ports)
            {
                shell.selection.node = None;
                shell.selection.route = Some(ports);
                shell.chrome.inspector_open = true;
                cx.notify();
            }
        })
    }
}

pub(super) fn inputs(
    shell: &StudioShell,
    entry: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let graph = &snapshot.graph;
    let mut view = section(
        format!("inputs-{:?}", entry.id),
        "INPUTS",
        theme::compact(shell.chrome.compact, 64., 88.),
        shell.mixer.presentation.inputs_collapsed,
        cx,
    );
    if !shell.mixer.presentation.inputs_collapsed {
        let scrolls = &shell.mixer.presentation.row_scrolls[&entry.id];
        let this = cx.entity();
        let mut rows = div()
            .id(SharedString::from(format!("input-rows-{:?}", entry.id)))
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .px(px(8.))
            .track_scroll(&scrolls.input)
            .restrict_scroll_to_axis()
            .overflow_y_scroll()
            .on_scroll_wheel(notify_owner(this));
        let mut count = 0;
        for edge in graph
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.dst() == entry.id)
        {
            count += 1;
            let name = graph
                .node(edge.src())
                .map(crate::ui::presentation::display_label)
                .unwrap_or_else(|| t!("Unknown").into_owned());
            let port = graph
                .graph
                .get_node(entry.id)
                .ok()
                .and_then(|node| {
                    node.inputs()
                        .iter()
                        .position(|port| port.id() == edge.dst_port())
                })
                .unwrap_or(0)
                + 1;
            let source_port = graph
                .graph
                .get_node(edge.src())
                .ok()
                .and_then(|node| {
                    node.outputs()
                        .iter()
                        .position(|port| port.id() == edge.src_port())
                })
                .unwrap_or(0)
                + 1;
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("input-{:?}", edge.id())))
                    .flex_none()
                    .py(px(3.))
                    .text_size(px(10.))
                    .text_color(theme::TEXT_MUTED)
                    .overflow_hidden()
                    .text_ellipsis()
                    .cursor_pointer()
                    .child(t!("%{name} → in %{port}", name = name, port = port))
                    .tooltip({
                        let text = t!(
                            "%{name} · out %{source_port} → %{target} · in %{port}",
                            name = name,
                            source_port = source_port,
                            target = entry.label,
                            port = port
                        )
                        .into_owned();
                        move |window, cx| {
                            component::tooltip::Tooltip::new(text.clone()).build(window, cx)
                        }
                    })
                    .on_click(select_route(
                        cx.entity(),
                        (edge.src_port(), edge.dst_port()),
                    ))
                    .gpui_test_observe(),
            );
        }
        if count == 0 {
            let resource = entry.source.as_ref().map(|source| {
                t!(
                    "Resource · %{name}",
                    name = crate::ui::presentation::source_name(snapshot, source)
                )
                .into_owned()
            });
            rows = rows.child(
                label(resource.unwrap_or_else(|| t!("No inputs").into_owned())).text_size(px(10.)),
            );
        }
        view = view.child(
            scroll_region(
                SharedString::from(format!("scrollbar-input-rows-{:?}", entry.id)),
                &scrolls.input,
                ScrollbarAxis::Vertical,
                rows,
            )
            .flex_1()
            .min_h_0(),
        );
    }
    view.gpui_test_observe()
}

pub(super) fn send_edit(
    this: Entity<StudioShell>,
    id: EdgeId,
    change: impl Fn(&mut SendParams) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                if let Ok(edge) = state.snapshot.graph.graph.get_edge(id) {
                    let mut params = *edge.params();
                    change(&mut params);
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                        edge: id,
                        params,
                    }));
                }
            });
            cx.notify();
        })
    }
}

pub(super) fn sends(
    shell: &StudioShell,
    id: NodeId,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let graph = &snapshot.graph;
    let mut view = section(
        format!("sends-{id:?}"),
        "SENDS",
        theme::compact(shell.chrome.compact, 80., 116.),
        shell.mixer.presentation.sends_collapsed,
        cx,
    );
    if !shell.mixer.presentation.sends_collapsed {
        let scrolls = &shell.mixer.presentation.row_scrolls[&id];
        let this = cx.entity();
        let mut rows = div()
            .id(SharedString::from(format!("send-rows-{id:?}")))
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .h_full()
            .px(px(8.))
            .track_scroll(&scrolls.send)
            .restrict_scroll_to_axis()
            .overflow_y_scroll()
            .on_scroll_wheel(notify_owner(this));
        let mut count = 0;
        for edge in graph.graph.edges().iter().filter(|edge| edge.src() == id) {
            count += 1;
            let edge_id = edge.id();
            let pre = edge.params().tap == SendTap::PreFader;
            let value = crate::ui::presentation::db_text_bare(edge.params().gain);
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    button(
                        SharedString::from(format!("send-mute-{edge_id:?}")),
                        if edge.params().mute { "×" } else { "●" },
                    )
                    .density(Density::Compact)
                    .square()
                    .tooltip(t!("Enable send; turning it off preserves its gain"))
                    .variant(if edge.params().mute {
                        ButtonVariant::Ghost
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(send_edit(cx.entity(), edge_id, |params| {
                        params.mute = !params.mute
                    })),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("send-target-{edge_id:?}")))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(10.))
                        .child(
                            graph
                                .node(edge.dst())
                                .map(crate::ui::presentation::display_label)
                                .unwrap_or_default(),
                        )
                        .on_click(select_route(
                            cx.entity(),
                            (edge.src_port(), edge.dst_port()),
                        )),
                );
            if shell.mixer.presentation.editing_send == Some(edge_id) {
                // The editor takes the label's box (32px, right-aligned mono
                // 9px text) with the shared underline-only chrome, so the row
                // neither grows nor shifts the text when it opens.
                row = row.child(
                    div()
                        .id(SharedString::from(format!("send-value-{edge_id:?}")))
                        .flex_none()
                        .w(px(32.))
                        .font_family(theme::FONT_MONO)
                        .child(underline_editor(
                            &shell.mixer.presentation.send_inputs[&edge_id],
                            px(9.),
                            TextAlign::Right,
                        )),
                );
            } else {
                row = row.child(
                    div()
                        .id(SharedString::from(format!("send-value-{edge_id:?}")))
                        .flex_none()
                        .w(px(32.))
                        .text_right()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(9.))
                        .child(value)
                        .cursor_pointer()
                        .on_click({
                            let this = cx.entity();
                            move |event, window, cx| {
                                if event.click_count() == 2 {
                                    this.update(cx, |shell, cx| {
                                        if let Some(input) =
                                            shell.mixer.presentation.send_inputs.get(&edge_id)
                                        {
                                            input.read(cx).focus_handle(cx).focus(window, cx);
                                            shell.mixer.presentation.editing_send = Some(edge_id);
                                            cx.notify();
                                        }
                                    });
                                }
                            }
                        })
                        .gpui_test_observe(),
                );
            }
            row = row.child(div().id(SharedString::from(format!("send-tap-{edge_id:?}"))).flex_none()
                .child(button(SharedString::from(format!("send-pre-{edge_id:?}")), if pre { "PRE" } else { "POST" })
                    .tooltip(t!("PRE: after channel mode, balance and mute; before gain and Dim. POST: after gain and Dim."))
                    .density(Density::Compact)
                    .square()
                    .variant(if pre { ButtonVariant::Secondary } else { ButtonVariant::Ghost })
                    .on_click(send_edit(cx.entity(), edge_id, |params| {
                        params.tap = if params.tap == SendTap::PreFader { SendTap::PostFader } else { SendTap::PreFader };
                    }))).gpui_test_observe());
            rows = rows.child(
                div().flex().flex_col().flex_none().child(row).child(
                    div()
                        .id(SharedString::from(format!("send-track-{edge_id:?}")))
                        .child(hslider(
                            &shell.mixer.presentation.sends[&edge_id].state,
                            &shell.mixer.presentation.sends[&edge_id].behavior,
                            cx,
                        )),
                ),
            );
        }
        if count == 0 {
            rows = rows.child(label(t!("No sends")).text_size(px(10.)));
        }
        view = view.child(
            scroll_region(
                SharedString::from(format!("scrollbar-send-rows-{:?}", id)),
                &scrolls.send,
                ScrollbarAxis::Vertical,
                rows,
            )
            .flex_1()
            .min_h_0(),
        );
    }
    view.gpui_test_observe()
}
