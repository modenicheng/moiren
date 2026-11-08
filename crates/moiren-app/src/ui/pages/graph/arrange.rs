//! The "arrange" auto-layout: DAG longest-path layering into left-to-right
//! columns, each column's cards stacked symmetrically around the world's
//! horizontal axis with their real painted heights. Pure computation — the
//! caller derives card sizes from the projected world, writes the results
//! into the presentation position table, and lets the page re-project; like
//! card drags, arrange is not an undo step.

use super::*;
use moiren_core::graph::{LogicalGraph, NodeId};
use std::collections::HashMap;

/// Column gap between layers and row gap inside a layer.
const GAP_X: f32 = 96.;
const GAP_Y: f32 = 48.;

/// One node's new top-left position after arranging.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ArrangedNode {
    pub node_id: NodeId,
    pub position: (f32, f32),
}

/// Card size lookup: `(width, height)` per node id.
pub(super) type CardSizes = HashMap<NodeId, (f32, f32)>;

/// The bounding box over the arranged cards, for the camera fit.
pub(super) fn bounds(arranged: &[ArrangedNode], sizes: &CardSizes) -> Bounds<f32> {
    let mut result = Option::<Bounds<f32>>::None;
    for node in arranged {
        let Some((width, height)) = sizes.get(&node.node_id) else {
            continue;
        };
        let rect = Bounds {
            origin: point(node.position.0, node.position.1),
            size: size(*width, *height),
        };
        result = Some(match result {
            Some(existing) => existing.union(&rect),
            None => rect,
        });
    }
    result.unwrap_or_else(|| Bounds {
        origin: point(0., 0.),
        size: size(1., 1.),
    })
}

/// Layers every node by its longest input path (the graph is validated
/// acyclic), lays the layers out as left-to-right columns, and stacks each
/// column's cards symmetrically around `y = 0`. In-column order is
/// deterministic: current y, then node id.
pub(super) fn arrange(graph: &LogicalGraph, sizes: &CardSizes) -> Vec<ArrangedNode> {
    let columns = columns_by_layer(graph, sizes);
    let mut arranged = Vec::new();
    let mut column_x = 0.;
    for column in &columns {
        let total: f32 = column.iter().map(|(_, _, height)| *height).sum::<f32>()
            + GAP_Y * column.len().saturating_sub(1) as f32;
        let mut top = -total / 2.;
        let mut widest: f32 = 0.;
        for (node_id, _, height) in column {
            let (width, _) = sizes.get(node_id).copied().unwrap_or((0., *height));
            arranged.push(ArrangedNode {
                node_id: *node_id,
                position: (column_x, top),
            });
            widest = widest.max(width);
            top += height + GAP_Y;
        }
        column_x += widest + GAP_X;
    }
    arranged
}

/// Column membership by longest-path layer, in deterministic order
/// (layer, then node id), carrying each card's painted height.
fn columns_by_layer(graph: &LogicalGraph, sizes: &CardSizes) -> Vec<Vec<(NodeId, f32, f32)>> {
    // Kahn layering: longest-path depth per node.
    let mut incoming: HashMap<NodeId, usize> = HashMap::new();
    let mut outgoing: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for node in graph.nodes() {
        incoming.entry(node.id()).or_insert(0);
    }
    for edge in graph.edges() {
        *incoming.entry(edge.dst()).or_insert(0) += 1;
        outgoing.entry(edge.src()).or_default().push(edge.dst());
    }
    let mut layers: HashMap<NodeId, usize> = HashMap::new();
    let mut ready: Vec<NodeId> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(&id, _)| id)
        .collect();
    while let Some(id) = ready.pop() {
        let depth = layers.get(&id).copied().unwrap_or(0);
        for &next in outgoing.get(&id).into_iter().flatten() {
            let count = incoming.get_mut(&next).expect("edge target exists");
            *count -= 1;
            let entry = layers.entry(next).or_insert(0);
            *entry = (*entry).max(depth + 1);
            if *count == 0 {
                ready.push(next);
            }
        }
    }

    let mut columns: Vec<Vec<(NodeId, f32, f32)>> = Vec::new(); // (id, unused y slot, height)
    // Deterministic scan of the graph's own order keeps the layout stable
    // across calls; node ids break ties within a layer.
    let mut ordered: Vec<(NodeId, usize, f32)> = graph
        .nodes()
        .iter()
        .map(|node| {
            (
                node.id(),
                layers.get(&node.id()).copied().unwrap_or(0),
                sizes
                    .get(&node.id())
                    .map(|(_, height)| *height)
                    .unwrap_or(0.),
            )
        })
        .collect();
    ordered.sort_by_key(|(id, layer, _)| (*layer, *id));
    for (node_id, layer, height) in ordered {
        if columns.len() <= layer {
            columns.resize(layer + 1, Vec::new());
        }
        columns[layer].push((node_id, 0., height));
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::{CardSizes, GAP_X, GAP_Y, arrange, bounds};
    use moiren_core::graph::{LogicalGraph, NodeId, NodeKind, SendParams};

    fn chain_graph() -> (LogicalGraph, Vec<NodeId>) {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        let source_out = graph.get_node(source).unwrap().outputs()[0].id();
        let gain_in = graph.get_node(gain).unwrap().inputs()[0].id();
        let gain_out = graph.get_node(gain).unwrap().outputs()[0].id();
        let sink_in = graph.get_node(sink).unwrap().inputs()[0].id();
        graph
            .connect(source_out, gain_in, SendParams::default())
            .unwrap();
        graph
            .connect(gain_out, sink_in, SendParams::default())
            .unwrap();
        (graph, vec![source, gain, sink])
    }

    fn sizes(ids: &[NodeId], width: f32, height: f32) -> CardSizes {
        ids.iter().map(|&id| (id, (width, height))).collect()
    }

    #[test]
    fn chain_lays_three_columns_left_to_right_and_centers_each() {
        let (graph, ids) = chain_graph();
        let arranged = arrange(&graph, &sizes(&ids, 100., 80.));
        assert_eq!(arranged.len(), 3);
        // Column x grows left to right by width + gap.
        let mut x: Vec<f32> = arranged.iter().map(|n| n.position.0).collect();
        x.sort_by(f32::total_cmp);
        assert_eq!(x[0], 0.);
        assert_eq!(x[1], 100. + GAP_X);
        assert_eq!(x[2], 2. * (100. + GAP_X));
        // A single-card column centers its card on the axis: top = -h/2.
        for node in &arranged {
            assert_eq!(node.position.1, -40.);
        }
    }

    #[test]
    fn parallel_nodes_share_a_column_and_stack_symmetrically() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
        let eq = graph.create_node(NodeKind::Eq, 2).unwrap();
        // The merge node is a Bus: Sinks carry a fixed port layout, while
        // Bus inputs grow dynamically.
        let sink = graph.create_node(NodeKind::Bus, 2).unwrap();
        let source_out = graph.get_node(source).unwrap().outputs()[0].id();
        let gain_in = graph.get_node(gain).unwrap().inputs()[0].id();
        let eq_in = graph.get_node(eq).unwrap().inputs()[0].id();
        let gain_out = graph.get_node(gain).unwrap().outputs()[0].id();
        let eq_out = graph.get_node(eq).unwrap().outputs()[0].id();
        // Bus starts with no inputs; both arrive as added ports, and inputs
        // are single-connection, so each parallel branch gets its own.
        let sink_in = graph.add_input_port(sink, 2).unwrap();
        let sink_in_2 = graph.add_input_port(sink, 2).unwrap();
        for (src, dst) in [
            (source_out, gain_in),
            (source_out, eq_in),
            (gain_out, sink_in),
            (eq_out, sink_in_2),
        ] {
            graph.connect(src, dst, SendParams::default()).unwrap();
        }
        let ids = vec![source, gain, eq, sink];
        let arranged = arrange(&graph, &sizes(&ids, 120., 60.));
        // Source and sink each own a column; gain and eq share the middle.
        let middle: Vec<_> = arranged
            .iter()
            .filter(|n| n.position.0 == 120. + GAP_X)
            .collect();
        assert_eq!(middle.len(), 2);
        // Two 60-high cards plus the gap: the stack centers on the axis.
        let mut tops: Vec<f32> = middle.iter().map(|n| n.position.1).collect();
        tops.sort_by(f32::total_cmp);
        assert_eq!(tops[0], -(60. + GAP_Y / 2.));
        assert_eq!(tops[1], GAP_Y / 2.);
        // The bounding box covers all four cards.
        let bbox = bounds(&arranged, &sizes(&ids, 120., 60.));
        assert_eq!(bbox.size.width, 2. * (120. + GAP_X) + 120.);
        assert_eq!(bbox.size.height, 2. * 60. + GAP_Y);
    }

    #[test]
    fn arranging_twice_is_stable() {
        let (graph, ids) = chain_graph();
        let card_sizes = sizes(&ids, 100., 80.);
        let first = arrange(&graph, &card_sizes);
        let second = arrange(&graph, &card_sizes);
        assert_eq!(first, second);
    }
}
