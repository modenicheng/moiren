//! Bus input inventory and actions over the authoritative graph topology.
use super::*;
use moiren_core::graph::{Edge, LogicalGraph, PortId};

pub(super) fn content(
    shell: &StudioShell,
    id: NodeId,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let snapshot = &shell.state.read(cx).snapshot;
    let topology = snapshot.graph.graph.get_node(id).ok()?;
    let mut rows = div()
        .id(format!("card-bus-inputs-{id:?}"))
        .flex()
        .flex_col()
        .gap(px(4.))
        .max_h(px(360.))
        .overflow_y_scroll();
    if topology.inputs().is_empty() {
        rows = rows.child(note("No input streams. Add one, then connect a source."));
    }
    for (index, port) in topology.inputs().iter().enumerate() {
        let mut row = div().flex().flex_col().gap(px(2.)).child(note(
            t!(
                "Input %{index} / %{channels} ch",
                index = index + 1,
                channels = port.channels()
            )
            .to_string(),
        ));
        if let Some(edge) = bus_input_connection(&snapshot.graph.graph, port.id()) {
            let name = snapshot
                .graph
                .node(edge.src())
                .zip(snapshot.graph.kind_of(edge.src()))
                .map(|(node, kind)| crate::ui::state::display_label(snapshot, node, kind))
                .unwrap_or_else(|| t!("Unavailable source").to_string());
            let output = snapshot
                .graph
                .graph
                .get_node(edge.src())
                .ok()
                .and_then(|node| {
                    node.outputs()
                        .iter()
                        .position(|port| port.id() == edge.src_port())
                });
            let title = t!(
                "%{name} / Out %{output}%{muted}",
                name = name,
                output = output.map_or_else(|| "?".to_owned(), |index| (index + 1).to_string()),
                muted = if edge.params().mute {
                    t!(" (muted)").to_string()
                } else {
                    String::new()
                }
            )
            .to_string();
            let ports = (edge.src_port(), edge.dst_port());
            let this = cx.entity();
            row = row.child(
                controls::button(
                    format!("card-bus-route-{id:?}-{:?}", port.id()),
                    title.clone(),
                )
                .variant(ButtonVariant::Ghost)
                .full_width()
                .justify_start()
                .truncate_label()
                .tooltip(title)
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.selected_nodes.clear();
                        shell.selected_node = None;
                        shell.selected_route = Some(ports);
                        shell.open_inspector();
                        cx.notify();
                    })
                }),
            );
        } else {
            row = row.child(note("Not connected").text_color(theme::TEXT_FAINT));
        }
        rows = rows.child(row);
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .child(detail_line(
                "Input streams",
                &topology.inputs().len().to_string(),
            ))
            .child(
                controls::button(format!("card-add-input-{id:?}"), t!("+ Add input"))
                    .full_width()
                    .on_click(controls::edit(cx.entity(), GraphEdit::AddBusInput(id))),
            )
            .child(rows)
            .into_any_element(),
    )
}

fn bus_input_connection(graph: &LogicalGraph, port: PortId) -> Option<&Edge> {
    graph.edges().iter().find(|edge| edge.dst_port() == port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn bus_connections_are_resolved_per_input_not_from_card_counts() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let first = graph.add_input_port(bus, 2).unwrap();
        let second = graph.add_input_port(bus, 2).unwrap();
        let output = graph.get_node(source).unwrap().outputs()[0].id();
        let unrelated = graph.get_node(sink).unwrap().inputs()[0].id();
        let connected = graph.connect(output, second, Default::default()).unwrap();
        graph
            .connect(output, unrelated, Default::default())
            .unwrap();
        assert!(bus_input_connection(&graph, first).is_none());
        let edge = bus_input_connection(&graph, second).unwrap();
        assert_eq!(edge.id(), connected);
        assert_eq!(edge.src(), source);
        assert_eq!(edge.dst(), bus);
        assert!(bus_input_connection(&graph, output).is_none());
    }
}
