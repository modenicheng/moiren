//! Direct port connections of the same LogicalGraph used by Graph and Mixer.
use rust_i18n::t;

use super::*;
use crate::service::GraphEdit;
use crate::ui::presentation::{db_text, step_db};
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
                .child(label(t!(
                    "%{name} / in %{index}",
                    name = name(node.id()),
                    index = i + 1
                ))),
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
                .child(label(t!(
                    "%{name} / out %{index}",
                    name = name(node.id()),
                    index = i + 1
                ))),
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
                        t!("Muted").into()
                    } else {
                        db_text(e.params().gain)
                    }
                })
                .unwrap_or_else(|| if forbidden { "×".into() } else { "—".into() });
            let this = cx.entity();
            row = row.child(
                base::Button::new(SharedString::from(format!("route-{ri}-{ci}")))
                    .accessibility_label(
                        t!(
                            "%{source} to %{destination}: %{value}",
                            source = name(source.id()),
                            destination = name(destination.id()),
                            value = value
                        )
                        .to_string(),
                    )
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
                            // Single click selects the cell; a double click
                            // also opens the inspector.
                            if s.element_double_clicked(StudioShell::element_key(
                                "route",
                                (src, dst),
                            )) {
                                s.open_inspector();
                            }
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
                .flex_1()
                .min_h_0()
                .min_w_0()
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
                                        .child(label(t!("Outputs → Inputs"))),
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
                        .child(div().flex_none().p(px(12.)).child(label(t!(
                            "%{outputs} output ports · %{inputs} input ports · %{connections} direct connections",
                            outputs = outputs.len(),
                            inputs = inputs.len(),
                            connections = graph.edges().len()
                        )))),
                ),
        )
        .gpui_test_observe()
        .into_any_element()
}

pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot.graph;
    let mut view = panel("matrix-inspector").child(heading(t!("Connection")));
    if let Some((source, destination)) = shell.selected_route {
        if let Some((src, dst, channels)) = find_nodes(snapshot, source, destination) {
            view = view
                .child(detail(&t!("From"), src))
                .child(detail(&t!("To"), dst))
                .child(detail(&t!("Channels"), channels.to_string()));
            if let Some(edge) = snapshot
                .graph
                .edges()
                .iter()
                .find(|e| e.src_port() == source && e.dst_port() == destination)
            {
                let id = edge.id();
                let params = *edge.params();
                view = view
                    .child(badge(t!("Connected"), theme::PRIMARY))
                    .child(detail(&t!("Route gain"), db_text(params.gain)))
                    .child(detail(&t!("Route balance"), format!("{:+.2}", params.pan)))
                    .child(detail(
                        &t!("Tap"),
                        match params.tap {
                            moiren_core::graph::SendTap::PreFader => t!("Pre-fader"),
                            moiren_core::graph::SendTap::PostFader => t!("Post-fader"),
                        },
                    ))
                    .child(
                        button("disconnect-route", t!("Disconnect"))
                            .variant(ButtonVariant::Danger)
                            .on_click(edit(cx.entity(), GraphEdit::Disconnect(id))),
                    )
                    .child(
                        button(
                            "mute-route",
                            if params.mute {
                                t!("Unmute route")
                            } else {
                                t!("Mute route")
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
                            // Route sends have no upper gain limit by design
                            // (SendParams deliberately accepts any finite
                            // headroom), so the +1 dB step never clamps.
                            .child(button("route-down", "−1 dB").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetSend {
                                    edge: id,
                                    params: SendParams {
                                        gain: step_db(params.gain, -1., 0., f64::INFINITY),
                                        ..params
                                    },
                                },
                            )))
                            .child(button("route-up", "+1 dB").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetSend {
                                    edge: id,
                                    params: SendParams {
                                        gain: step_db(params.gain, 1., 0., f64::INFINITY),
                                        ..params
                                    },
                                },
                            ))),
                    );
            } else {
                view = view
                    .child(badge(t!("Not connected"), theme::TEXT_FAINT))
                    .child(
                        button("connect-route", t!("Connect"))
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
            view = view.when(shell.presentation.page != Page::Graph, |view| {
                view.child(button("show-in-graph", t!("Show in Graph")).on_click(
                    move |_, _, cx| {
                        this.update(cx, |s, cx| {
                            s.selected_node = None;
                            s.set_page(Page::Graph, cx);
                            s.open_inspector();
                        })
                    },
                ))
            });
        } else {
            view = view.child(label(t!(
                "This connection no longer exists. Select a cell."
            )));
        }
    } else {
        view = view.child(label(t!(
            "Select an intersection to inspect or change a direct connection."
        )));
    }
    view.gpui_test_observe().into_any_element()
}

fn find_nodes(
    snapshot: &crate::service::GraphSnapshot,
    source: PortId,
    destination: PortId,
) -> Option<(String, String, usize)> {
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
    let mut from = snapshot.node(src.id())?.label.clone();
    let mut to = snapshot.node(dst.id())?.label.clone();
    if src.outputs().len() > 1 {
        let index = src.outputs().iter().position(|port| port.id() == source)?;
        from.push_str(&t!(" · output %{index}", index = index + 1));
    }
    if dst.inputs().len() > 1 {
        let index = dst
            .inputs()
            .iter()
            .position(|port| port.id() == destination)?;
        to.push_str(&t!(" · input %{index}", index = index + 1));
    }
    Some((from, to, src.channels()))
}
