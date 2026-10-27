//! Port inventory and bus topology edits for the selected node.
use super::*;

pub(super) fn routing(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = node.id();
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let mut view = body();

    for (index, port) in node.inputs().iter().enumerate() {
        view = view.child(port_routes(snapshot, port, index, true, cx));
    }
    for (index, port) in node.outputs().iter().enumerate() {
        view = view.child(port_routes(snapshot, port, index, false, cx));
    }
    if node.kind() == NodeKind::Bus {
        view = view.child(
            button("add-bus-input", t!("Add bus input"))
                .on_click(edit(cx.entity(), GraphEdit::AddBusInput(id))),
        );
        if let Some(port) = node.inputs().last() {
            view = view.child(
                button("remove-bus-input", t!("Remove last input")).on_click(edit(
                    cx.entity(),
                    GraphEdit::RemoveBusInput {
                        node: id,
                        port: port.id(),
                    },
                )),
            );
        }
    }
    Disclosure::new(Section::Routing, id, view).into_any_element()
}

fn port_routes(
    snapshot: &crate::service::AppSnapshot,
    port: &moiren_core::graph::Port,
    index: usize,
    input: bool,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let title = if input {
        t!("Input %{index}", index = index + 1)
    } else {
        t!("Output %{index}", index = index + 1)
    };
    let mut view = div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::SPACE_1))
        .child(detail(&title, format!("{} ch", port.channels())));
    let routes = routes_for_port(&snapshot.graph.graph, port.id(), input);
    if routes.is_empty() {
        return view.child(label(t!("Not connected")).text_size(px(theme::TEXT_XS)));
    }
    for (route_index, edge) in routes.into_iter().enumerate() {
        let peer = if input { edge.src() } else { edge.dst() };
        let name = snapshot
            .graph
            .node(peer)
            .zip(snapshot.graph.kind_of(peer))
            .map(|(entry, kind)| crate::ui::state::display_label(snapshot, entry, kind))
            .unwrap_or_else(|| t!("Unavailable for this node").into_owned());
        let ports = (edge.src_port(), edge.dst_port());
        let this = cx.entity();
        // Selecting a real route opens the existing route inspector; no
        // duplicate routing authority or inline send state lives here.
        view = view.child(
            button(
                format!("inspector-route-{input}-{index}-{route_index}"),
                name,
            )
            .variant(ButtonVariant::Ghost)
            .full_width()
            .justify_start()
            .trailing(db_text(edge.params().gain))
            .tooltip(t!("Inspect connection"))
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.selected_nodes.clear();
                    shell.selected_node = None;
                    shell.selected_route = Some(ports);
                    shell.inspector_open = true;
                    cx.notify();
                });
            }),
        );
    }
    view
}

fn routes_for_port(
    graph: &moiren_core::graph::LogicalGraph,
    port: moiren_core::graph::PortId,
    input: bool,
) -> Vec<&moiren_core::graph::Edge> {
    graph
        .edges()
        .iter()
        .filter(|edge| {
            if input {
                edge.dst_port() == port
            } else {
                edge.src_port() == port
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::routes_for_port;
    use moiren_core::graph::{LogicalGraph, NodeKind};

    #[::core::prelude::v1::test]
    fn port_routes_preserve_output_fanout_and_input_direction() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let first = graph.create_node(NodeKind::Sink, 2).unwrap();
        let second = graph.create_node(NodeKind::Sink, 2).unwrap();
        let output = graph.get_node(source).unwrap().outputs()[0].id();
        let first_input = graph.get_node(first).unwrap().inputs()[0].id();
        let second_input = graph.get_node(second).unwrap().inputs()[0].id();
        graph
            .connect(output, first_input, Default::default())
            .unwrap();
        graph
            .connect(output, second_input, Default::default())
            .unwrap();
        assert_eq!(routes_for_port(&graph, output, false).len(), 2);
        assert_eq!(routes_for_port(&graph, first_input, true).len(), 1);
        assert!(routes_for_port(&graph, first_input, false).is_empty());
    }
}
