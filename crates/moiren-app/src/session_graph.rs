use moiren_core::graph::{GraphError, LogicalGraph, NodeId, NodeKind, SendParams};

pub(crate) struct SessionGraph {
    pub graph: LogicalGraph,
    pub input: NodeId,
    pub gain: NodeId,
    pub pan: NodeId,
    pub output: NodeId,
}

impl SessionGraph {
    pub fn new() -> Result<Self, GraphError> {
        let mut graph = LogicalGraph::new();
        let input = graph.create_node(NodeKind::Source, 2)?;
        let gain = graph.create_node(NodeKind::Gain, 2)?;
        let pan = graph.create_node(NodeKind::Pan, 2)?;
        let output = graph.create_node(NodeKind::Sink, 2)?;
        for (source, destination) in [(input, gain), (gain, pan), (pan, output)] {
            graph.connect(
                graph.get_node(source)?.outputs()[0].id(),
                graph.get_node(destination)?.inputs()[0].id(),
                SendParams::default(),
            )?;
        }
        Ok(Self {
            graph,
            input,
            gain,
            pan,
            output,
        })
    }
}
