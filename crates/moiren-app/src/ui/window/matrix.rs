//! Direct port connections of the same LogicalGraph used by Graph and Mixer.
use super::*;
use crate::service::GraphEdit;
use controls::*;
use moiren_core::graph::{PortId, SendParams};

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot.graph;
    let graph = &snapshot.graph;
    let outputs: Vec<_> = graph
        .nodes()
        .iter()
        .flat_map(|n| n.outputs().iter().enumerate().map(move |(i, p)| (n, i, p)))
        .collect();
    let inputs: Vec<_> = graph
        .nodes()
        .iter()
        .flat_map(|n| n.inputs().iter().enumerate().map(move |(i, p)| (n, i, p)))
        .collect();
    let row_height = if shell.density == Density::Compact {
        36.
    } else {
        48.
    };
    let width = inputs.len() as f32 * 112.;
    let height = outputs.len() as f32 * row_height;
    let offset = shell.matrix_scroll.offset();
    let name = |id| {
        snapshot
            .node(id)
            .map(|n| n.label.clone())
            .unwrap_or_default()
    };
    let mut column_header = div()
        .absolute()
        .left(offset.x)
        .flex()
        .w(px(width))
        .h(px(row_height));
    for (node, i, _) in &inputs {
        column_header = column_header.child(
            div()
                .flex_none()
                .w(px(112.))
                .h_full()
                .p(px(8.))
                .bg(theme::SURFACE_3)
                .child(label(format!("{} / in {}", name(node.id()), i + 1))),
        );
    }
    let mut row_header = div()
        .absolute()
        .top(offset.y)
        .flex()
        .flex_col()
        .w(px(156.))
        .h(px(height));
    for (node, i, _) in &outputs {
        row_header = row_header.child(
            div()
                .flex_none()
                .h(px(row_height))
                .p(px(8.))
                .bg(theme::SURFACE_3)
                .child(label(format!("{} / out {}", name(node.id()), i + 1))),
        );
    }
    let mut grid = div().flex().flex_col().w(px(width)).h(px(height));
    for (ri, (source, _, out)) in outputs.iter().enumerate() {
        let mut row = div().flex().flex_none().h(px(row_height));
        for (ci, (destination, _, input)) in inputs.iter().enumerate() {
            let src = out.id();
            let dst = input.id();
            let edge = graph
                .edges()
                .iter()
                .find(|e| e.src_port() == src && e.dst_port() == dst);
            let selected = shell.selected_route == Some((src, dst));
            let forbidden = source.id() == destination.id();
            let value = edge
                .map(|e| {
                    if e.params().mute {
                        "Muted".into()
                    } else {
                        db(e.params().gain)
                    }
                })
                .unwrap_or_else(|| if forbidden { "×".into() } else { "—".into() });
            let this = cx.entity();
            row = row.child(
                base::Button::new(SharedString::from(format!("route-{ri}-{ci}")))
                    .accessibility_label(format!(
                        "{} to {}: {value}",
                        name(source.id()),
                        name(destination.id())
                    ))
                    .w(px(112.))
                    .h(px(row_height))
                    .rounded_none()
                    .border_0()
                    .bg(if selected {
                        theme::soft(theme::PRIMARY)
                    } else if edge.is_some() {
                        theme::SURFACE_3
                    } else if ri % 2 == 0 {
                        theme::SURFACE
                    } else {
                        theme::WINDOW_BG
                    })
                    .text_size(px(12.))
                    .text_color(if edge.is_some() {
                        theme::PRIMARY
                    } else {
                        theme::TEXT_FAINT
                    })
                    .child(value)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |s, cx| {
                            s.selected_route = Some((src, dst));
                            s.inspector_open = true;
                            cx.notify();
                        })
                    }),
            );
        }
        grid = grid.child(row);
    }
    let this = cx.entity();
    div()
        .id("matrix-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(row_height))
                .items_center()
                .justify_between()
                .px(px(16.))
                .child(heading("Routing matrix"))
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
                .gap(px(12.))
                .p(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .rounded(px(theme::RADIUS_MD))
                        .border_1()
                        .border_color(theme::BORDER)
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .h(px(row_height))
                                .child(
                                    div()
                                        .w(px(156.))
                                        .flex_none()
                                        .p(px(8.))
                                        .bg(theme::SURFACE_3)
                                        .child(label("Outputs → Inputs")),
                                )
                                .child(
                                    div()
                                        .id("matrix-column-header")
                                        .relative()
                                        .flex_1()
                                        .overflow_hidden()
                                        .child(column_header)
                                        .gpui_test_observe(),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("matrix-row-header")
                                        .relative()
                                        .w(px(156.))
                                        .flex_none()
                                        .overflow_hidden()
                                        .child(row_header)
                                        .gpui_test_observe(),
                                )
                                .child(
                                    div()
                                        .id("matrix-grid")
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_scroll()
                                        .track_scroll(&shell.matrix_scroll)
                                        .on_scroll_wheel(move |_, _, cx| {
                                            this.update(cx, |_, cx| cx.notify())
                                        })
                                        .child(grid)
                                        .gpui_test_observe(),
                                ),
                        )
                        .child(div().flex_none().p(px(12.)).child(label(format!(
                            "{} output ports · {} input ports · {} direct connections",
                            outputs.len(),
                            inputs.len(),
                            graph.edges().len()
                        )))),
                )
                .when(shell.inspector_open, |v| v.child(inspector(shell, cx))),
        )
        .gpui_test_observe()
        .into_any_element()
}

pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot.graph;
    let mut view = panel("matrix-inspector").child(heading("Connection"));
    if let Some((source, destination)) = shell.selected_route {
        if let Some((src, dst)) = find_nodes(snapshot, source, destination) {
            view = view.child(detail("From", src)).child(detail("To", dst));
            if let Some(edge) = snapshot
                .graph
                .edges()
                .iter()
                .find(|e| e.src_port() == source && e.dst_port() == destination)
            {
                let id = edge.id();
                let params = *edge.params();
                view = view
                    .child(badge("Connected", theme::PRIMARY))
                    .child(detail("Route gain", db(params.gain)))
                    .child(detail("Route balance", format!("{:+.2}", params.pan)))
                    .child(
                        button("disconnect-route", "Disconnect")
                            .variant(ButtonVariant::Danger)
                            .on_click(edit(cx.entity(), GraphEdit::Disconnect(id))),
                    )
                    .child(
                        button(
                            "mute-route",
                            if params.mute {
                                "Unmute route"
                            } else {
                                "Mute route"
                            },
                        )
                        .on_click(edit(
                            cx.entity(),
                            GraphEdit::SetSend {
                                edge: id,
                                params: SendParams {
                                    mute: !params.mute,
                                    ..params
                                },
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(button("route-down", "−1 dB").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetSend {
                                    edge: id,
                                    params: SendParams {
                                        gain: params.gain * 10f64.powf(-1. / 20.),
                                        ..params
                                    },
                                },
                            )))
                            .child(button("route-up", "+1 dB").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetSend {
                                    edge: id,
                                    params: SendParams {
                                        gain: params.gain * 10f64.powf(1. / 20.),
                                        ..params
                                    },
                                },
                            ))),
                    );
            } else {
                view = view.child(badge("Not connected", theme::TEXT_FAINT)).child(
                    button("connect-route", "Connect")
                        .variant(ButtonVariant::Primary)
                        .on_click(edit(
                            cx.entity(),
                            GraphEdit::Connect {
                                source,
                                destination,
                            },
                        )),
                );
            }
            let this = cx.entity();
            let node = snapshot
                .graph
                .nodes()
                .iter()
                .find(|n| n.outputs().iter().any(|p| p.id() == source))
                .map(|n| format!("{:?}", n.id()));
            view = view
                .child(
                    button("show-in-graph", "Show in Graph").on_click(move |_, _, cx| {
                        this.update(cx, |s, cx| {
                            s.selected_node = node.clone();
                            s.set_page(Page::Graph, cx);
                        })
                    }),
                )
                .child(label(
                    "Route gain belongs to this connection. It is independent of a node's fader.",
                ));
        } else {
            view = view.child(label("This connection no longer exists. Select a cell."));
        }
    } else {
        view = view.child(label(
            "Select an intersection to inspect or change a direct connection.",
        ));
    }
    view.child(label("Apply graph to use the new routing."))
}

fn find_nodes(
    snapshot: &crate::service::GraphSnapshot,
    source: PortId,
    destination: PortId,
) -> Option<(String, String)> {
    let src = snapshot
        .graph
        .nodes()
        .iter()
        .find(|n| n.outputs().iter().any(|p| p.id() == source))?;
    let dst = snapshot
        .graph
        .nodes()
        .iter()
        .find(|n| n.inputs().iter().any(|p| p.id() == destination))?;
    Some((
        snapshot.node(src.id())?.label.clone(),
        snapshot.node(dst.id())?.label.clone(),
    ))
}
