//! Non-RT sparse indices over one borrowed logical graph. The graph remains the
//! authority; borrowing prevents edits from making an index silently stale.
use std::collections::BTreeMap;

use super::{Edge, GraphError, LogicalGraph, NodeId, PortId};

/// Direct adjacency and input occupancy, retaining every parallel Edge.
/// Storage scales with nodes, input ports and edges, not the product of axes.
pub struct GraphIndex<'graph> {
    graph: &'graph LogicalGraph,
    incoming: BTreeMap<NodeId, Vec<&'graph Edge>>,
    outgoing: BTreeMap<NodeId, Vec<&'graph Edge>>,
    pairs: BTreeMap<(NodeId, NodeId), Vec<&'graph Edge>>,
    inputs: BTreeMap<PortId, Option<&'graph Edge>>,
}

impl<'graph> GraphIndex<'graph> {
    /// Builds allocating lookup tables on the control side. An index should be
    /// rebuilt for a changed graph snapshot and must never be built on RT.
    pub fn new(graph: &'graph LogicalGraph) -> Self {
        let mut index = Self {
            graph,
            incoming: graph
                .nodes()
                .iter()
                .map(|node| (node.id(), Vec::new()))
                .collect(),
            outgoing: graph
                .nodes()
                .iter()
                .map(|node| (node.id(), Vec::new()))
                .collect(),
            pairs: BTreeMap::new(),
            inputs: graph
                .nodes()
                .iter()
                .flat_map(|node| node.inputs().iter().map(|port| (port.id(), None)))
                .collect(),
        };
        for edge in graph.edges() {
            index
                .incoming
                .get_mut(&edge.dst())
                .expect("graph destination exists")
                .push(edge);
            index
                .outgoing
                .get_mut(&edge.src())
                .expect("graph source exists")
                .push(edge);
            index
                .pairs
                .entry((edge.src(), edge.dst()))
                .or_default()
                .push(edge);
            *index
                .inputs
                .get_mut(&edge.dst_port())
                .expect("graph input exists") = Some(edge);
        }
        index
    }

    /// All direct input edges, in graph edge order; valid isolated nodes return
    /// an empty slice, while identities outside this graph are rejected.
    pub fn incoming(&self, node: NodeId) -> Result<&[&'graph Edge], GraphError> {
        self.incoming
            .get(&node)
            .map(Vec::as_slice)
            .ok_or(GraphError::NodeNotFound)
    }

    pub fn outgoing(&self, node: NodeId) -> Result<&[&'graph Edge], GraphError> {
        self.outgoing
            .get(&node)
            .map(Vec::as_slice)
            .ok_or(GraphError::NodeNotFound)
    }

    /// Node-pair adjacency, not transitive reachability. Multiple Bus inputs
    /// from the same source remain independently addressable by EdgeId.
    pub fn between(
        &self,
        source: NodeId,
        destination: NodeId,
    ) -> Result<&[&'graph Edge], GraphError> {
        if !self.outgoing.contains_key(&source) || !self.incoming.contains_key(&destination) {
            return Err(GraphError::NodeNotFound);
        }
        Ok(self
            .pairs
            .get(&(source, destination))
            .map_or(&[], Vec::as_slice))
    }

    /// Full-graph occupancy regardless of which sources a client displays.
    pub fn input_edge(&self, input: PortId) -> Result<Option<&'graph Edge>, GraphError> {
        if let Some(edge) = self.inputs.get(&input) {
            return Ok(*edge);
        }
        self.graph.get_port(input)?;
        Err(GraphError::PortDirectionMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{NodeKind, SendParams, edit::connect_to_new_bus_input};

    #[test]
    fn index_preserves_parallel_routes_and_distinguishes_direct_paths() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let bus = graph.create_node(NodeKind::Bus, 1).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
        let orphan = graph.create_node(NodeKind::Gain, 1).unwrap();
        let output = graph.get_node(source).unwrap().outputs()[0].id();
        let first =
            connect_to_new_bus_input(&mut graph, output, bus, SendParams::default()).unwrap();
        let second = connect_to_new_bus_input(
            &mut graph,
            output,
            bus,
            SendParams {
                gain: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        let destination = graph.get_node(sink).unwrap().inputs()[0].id();
        graph
            .connect(
                graph.get_node(bus).unwrap().outputs()[0].id(),
                destination,
                SendParams::default(),
            )
            .unwrap();
        let empty = graph.add_input_port(bus, 1).unwrap();
        let index = GraphIndex::new(&graph);
        assert_eq!(
            index
                .outgoing(source)
                .unwrap()
                .iter()
                .map(|edge| edge.id())
                .collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(index.incoming(bus).unwrap().len(), 2);
        assert_eq!(index.between(source, bus).unwrap().len(), 2);
        assert!(index.between(source, sink).unwrap().is_empty());
        assert!(index.incoming(orphan).unwrap().is_empty());
        assert!(index.outgoing(orphan).unwrap().is_empty());
        let input = graph.get_edge(first).unwrap().dst_port();
        assert_eq!(index.input_edge(input).unwrap().unwrap().id(), first);
        assert_eq!(index.input_edge(empty), Ok(None));
        assert_eq!(
            index.input_edge(output),
            Err(GraphError::PortDirectionMismatch)
        );
        assert_eq!(
            index.input_edge(PortId(u64::MAX)),
            Err(GraphError::PortNotFound)
        );
        assert_eq!(
            index.incoming(NodeId(u64::MAX)),
            Err(GraphError::NodeNotFound)
        );
        assert_eq!(
            index.outgoing(NodeId(u64::MAX)),
            Err(GraphError::NodeNotFound)
        );
        assert_eq!(
            index.between(source, NodeId(u64::MAX)),
            Err(GraphError::NodeNotFound)
        );
        assert_eq!(
            index.between(NodeId(u64::MAX), sink),
            Err(GraphError::NodeNotFound)
        );
    }
}
