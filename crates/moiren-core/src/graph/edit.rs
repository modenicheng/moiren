//! Composed non-RT editor commands, separate from primitive topology operations.
use super::{EdgeId, GraphError, LogicalGraph, NodeId, PortId, SendParams};

/// Add an independent Bus input and connect it. A failed connection removes
/// the new input; existing graph objects are untouched. IDs stay monotonic,
/// including IDs consumed by an unsuccessful composed edit.
pub fn connect_to_new_bus_input(
    graph: &mut LogicalGraph,
    src_port: PortId,
    bus: NodeId,
    params: SendParams,
) -> Result<EdgeId, GraphError> {
    let channels = graph.get_node(bus)?.channels();
    let input = graph.add_input_port(bus, channels)?;
    match graph.connect(src_port, input, params) {
        Ok(edge) => Ok(edge),
        Err(error) => {
            graph
                .remove_input_port(bus, input)
                .expect("new disconnected Bus input");
            Err(error)
        }
    }
}
