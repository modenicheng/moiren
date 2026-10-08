//! Non-RT editable topology. Ports carry streams, not individual channels or
//! realtime buffer addresses. Only graph operations may mutate this model.
use std::collections::{BTreeMap, BTreeSet};

mod model;
pub use model::*;
pub mod edit;

#[cfg(test)]
mod tests;

// Prepared engine IO uses u16 port numbers; 0 through u16::MAX are usable.
const MAX_INPUT_PORTS: usize = u16::MAX as usize + 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct IdCounter {
    node: u64,
    port: u64,
    edge: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogicalGraph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    counts: IdCounter,
}

impl LogicalGraph {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn get_node(&self, id: NodeId) -> Result<&Node, GraphError> {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or(GraphError::NodeNotFound)
    }

    pub fn get_port(&self, id: PortId) -> Result<&Port, GraphError> {
        self.find_port(id).map(|(_, port)| port)
    }

    pub fn get_edge(&self, id: EdgeId) -> Result<&Edge, GraphError> {
        self.edges
            .iter()
            .find(|e| e.id == id)
            .ok_or(GraphError::EdgeNotFound)
    }

    fn find_port(&self, id: PortId) -> Result<(&Node, &Port), GraphError> {
        self.nodes
            .iter()
            .find_map(|node| {
                node.inputs
                    .iter()
                    .chain(&node.outputs)
                    .find(|port| port.id == id)
                    .map(|port| (node, port))
            })
            .ok_or(GraphError::PortNotFound)
    }

    /// Fixed layouts are created together; a Bus starts with no inputs.
    /// Validate and reserve the entire ID range before changing graph state.
    pub fn create_node(&mut self, kind: NodeKind, channels: usize) -> Result<NodeId, GraphError> {
        kind.validate_channels(channels)?;
        let (input_count, output_count) = kind.initial_ports();
        let next_node = self
            .counts
            .node
            .checked_add(1)
            .ok_or(GraphError::TooManyNodes)?;
        let next_port = self
            .counts
            .port
            .checked_add((input_count + output_count) as u64)
            .ok_or(GraphError::TooManyPorts)?;
        let id = NodeId(self.counts.node);
        let mut port_id = self.counts.port;
        let mut make_ports = |count, role| {
            (0..count)
                .map(|_| {
                    let port = Port {
                        id: PortId(port_id),
                        channels,
                        role,
                    };
                    port_id += 1;
                    port
                })
                .collect()
        };
        let inputs = make_ports(input_count, PortRole::Input);
        let outputs = make_ports(output_count, PortRole::Output);
        self.nodes.push(Node {
            id,
            kind,
            channels,
            inputs,
            outputs,
        });
        self.counts.node = next_node;
        self.counts.port = next_port;
        Ok(id)
    }

    /// Only Bus supports dynamic inputs; each must match its output channels.
    pub fn add_input_port(
        &mut self,
        node_id: NodeId,
        channels: usize,
    ) -> Result<PortId, GraphError> {
        if channels == 0 {
            return Err(GraphError::InvalidChannelCount);
        }
        let index = self
            .nodes
            .iter()
            .position(|n| n.id == node_id)
            .ok_or(GraphError::NodeNotFound)?;
        let node = &self.nodes[index];
        if node.kind != NodeKind::Bus {
            return Err(GraphError::FixedPortLayout);
        }
        if node.channels != channels {
            return Err(GraphError::ChannelMismatch);
        }
        if node.inputs.len() >= MAX_INPUT_PORTS {
            return Err(GraphError::TooManyPorts);
        }
        let next = self
            .counts
            .port
            .checked_add(1)
            .ok_or(GraphError::TooManyPorts)?;
        let id = PortId(self.counts.port);
        self.nodes[index].inputs.push(Port {
            id,
            channels,
            role: PortRole::Input,
        });
        self.counts.port = next;
        Ok(id)
    }

    /// Disconnect first. Empty inputs otherwise survive disconnect for reuse.
    pub fn remove_input_port(
        &mut self,
        node_id: NodeId,
        port_id: PortId,
    ) -> Result<Port, GraphError> {
        let node = self.get_node(node_id)?;
        if node.kind != NodeKind::Bus {
            return Err(GraphError::FixedPortLayout);
        }
        let index = node
            .inputs
            .iter()
            .position(|p| p.id == port_id)
            .ok_or(GraphError::PortNotFound)?;
        if self.edges.iter().any(|e| e.dst_port == port_id) {
            return Err(GraphError::PortConnected);
        }
        let node = self
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .expect("checked node");
        Ok(node.inputs.remove(index))
    }

    /// Connect existing ports only. Gain is linear; no implicit conversion or
    /// mixing is performed. Send parameters remain compiler input metadata.
    pub fn connect(
        &mut self,
        src_port: PortId,
        dst_port: PortId,
        params: SendParams,
    ) -> Result<EdgeId, GraphError> {
        let (source, src) = self.find_port(src_port)?;
        let (destination, dst) = self.find_port(dst_port)?;
        if src.role != PortRole::Output || dst.role != PortRole::Input {
            return Err(GraphError::PortDirectionMismatch);
        }
        if src.channels != dst.channels {
            return Err(GraphError::ChannelMismatch);
        }
        params.validate()?;
        if self.edges.iter().any(|e| e.dst_port == dst_port) {
            return Err(GraphError::InputAlreadyConnected);
        }
        let (src, dst) = (source.id, destination.id);
        if self.would_cycle(src, dst)? {
            return Err(GraphError::CycleDetected);
        }
        let next = self
            .counts
            .edge
            .checked_add(1)
            .ok_or(GraphError::TooManyEdges)?;
        let id = EdgeId(self.counts.edge);
        self.edges.push(Edge {
            id,
            src,
            dst,
            src_port,
            dst_port,
            params,
        });
        self.counts.edge = next;
        Ok(id)
    }

    pub fn disconnect(&mut self, edge_id: EdgeId) -> Result<Edge, GraphError> {
        let index = self
            .edges
            .iter()
            .position(|e| e.id == edge_id)
            .ok_or(GraphError::EdgeNotFound)?;
        Ok(self.edges.remove(index))
    }

    /// Node deletion removes every incident edge, including all fan-out sends.
    pub fn remove_node(&mut self, node_id: NodeId) -> Result<Node, GraphError> {
        let index = self
            .nodes
            .iter()
            .position(|n| n.id == node_id)
            .ok_or(GraphError::NodeNotFound)?;
        self.edges.retain(|e| e.src != node_id && e.dst != node_id);
        Ok(self.nodes.remove(index))
    }

    /// A control-side value edit; does not alter ports or topology.
    pub fn set_send_params(
        &mut self,
        edge_id: EdgeId,
        params: SendParams,
    ) -> Result<(), GraphError> {
        self.get_edge(edge_id)?;
        params.validate()?;
        self.edges
            .iter_mut()
            .find(|e| e.id == edge_id)
            .expect("checked edge")
            .params = params;
        Ok(())
    }

    /// Kahn ordering, choosing the oldest ready node each time. Parallel edges
    /// are counted individually. Traversal is iterative even for deep graphs.
    pub fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut topology = self.topology()?;
        let mut ready = topology
            .indegree
            .iter()
            .enumerate()
            .filter_map(|(i, &count)| (count == 0).then_some(i))
            .collect::<BTreeSet<_>>();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(index) = ready.pop_first() {
            order.push(self.nodes[index].id);
            for &next in &topology.successors[index] {
                topology.indegree[next] -= 1;
                if topology.indegree[next] == 0 {
                    ready.insert(next);
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(GraphError::CycleDetected);
        }
        Ok(order)
    }

    /// Recheck structural contracts before a future compiler consumes the graph.
    pub fn validate(&self) -> Result<(), GraphError> {
        let mut node_ids = BTreeSet::new();
        let mut port_ids = BTreeSet::new();
        for node in &self.nodes {
            node.kind.validate_channels(node.channels)?;
            let (inputs, outputs) = node.kind.initial_ports();
            if !node_ids.insert(node.id)
                || node.outputs.len() != outputs
                || (node.kind != NodeKind::Bus && node.inputs.len() != inputs)
                || node.inputs.len() > MAX_INPUT_PORTS
            {
                return Err(GraphError::InvalidNodeLayout);
            }
            for (ports, role) in [
                (&node.inputs, PortRole::Input),
                (&node.outputs, PortRole::Output),
            ] {
                for port in ports {
                    if !port_ids.insert(port.id)
                        || port.role != role
                        || port.channels != node.channels
                    {
                        return Err(GraphError::InvalidNodeLayout);
                    }
                }
            }
        }
        let mut edge_ids = BTreeSet::new();
        let mut incoming = BTreeSet::new();
        for edge in &self.edges {
            let (source, src) = self.find_port(edge.src_port)?;
            let (destination, dst) = self.find_port(edge.dst_port)?;
            if source.id != edge.src || destination.id != edge.dst || !edge_ids.insert(edge.id) {
                return Err(GraphError::InvalidNodeLayout);
            }
            if src.role != PortRole::Output || dst.role != PortRole::Input {
                return Err(GraphError::PortDirectionMismatch);
            }
            if src.channels != dst.channels {
                return Err(GraphError::ChannelMismatch);
            }
            if !incoming.insert(dst.id) {
                return Err(GraphError::InputAlreadyConnected);
            }
            edge.params.validate()?;
        }
        self.topological_order().map(|_| ())
    }

    fn topology(&self) -> Result<Topology, GraphError> {
        let indices = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect::<BTreeMap<_, _>>();
        let mut successors = vec![Vec::new(); self.nodes.len()];
        let mut indegree = vec![0; self.nodes.len()];
        for edge in &self.edges {
            let &src = indices.get(&edge.src).ok_or(GraphError::NodeNotFound)?;
            let &dst = indices.get(&edge.dst).ok_or(GraphError::NodeNotFound)?;
            successors[src].push(dst);
            indegree[dst] += 1;
        }
        Ok(Topology {
            indices,
            successors,
            indegree,
        })
    }

    fn would_cycle(&self, src: NodeId, dst: NodeId) -> Result<bool, GraphError> {
        if src == dst {
            return Ok(true);
        }
        let topology = self.topology()?;
        let goal = topology.indices[&src];
        let mut visited = vec![false; self.nodes.len()];
        let mut stack = vec![topology.indices[&dst]];
        while let Some(index) = stack.pop() {
            if index == goal {
                return Ok(true);
            }
            if !visited[index] {
                visited[index] = true;
                stack.extend(&topology.successors[index]);
            }
        }
        Ok(false)
    }
}

struct Topology {
    indices: BTreeMap<NodeId, usize>,
    successors: Vec<Vec<usize>>,
    indegree: Vec<usize>,
}
