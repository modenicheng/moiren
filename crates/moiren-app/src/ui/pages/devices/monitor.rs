//! Device monitoring projection (design 23 §3.7): who is bound to a device
//! in the candidate graph, and which nodes the engine reports as running a
//! live stream on it. Pure data-in/data-out; the Windows-only observation
//! extraction lives behind [`live_nodes`] so every other rule here is
//! testable on any platform.

use crate::service::{AppSnapshot, ExportIntent, GraphSnapshot, InputSelection};
use moiren_core::graph::NodeId;
use moiren_engine::node::VirtualEndpointId;

/// The device a devices-page row stands for. Physical rows carry their
/// direction: WASAPI capture and render ids are distinct namespaces, and an
/// ASIO driver id appears on both tabs — the direction picks which of the
/// driver's streams belongs to which row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeviceKey {
    Physical { endpoint_id: String, input: bool },
    Virtual(VirtualEndpointId),
}

/// How a node relates to a device right now: the candidate graph's binding
/// on one side, the engine's live streams on the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserState {
    /// Bound in the candidate graph and a live stream runs on this device.
    Running,
    /// Bound in the candidate graph, but no live stream on this device:
    /// either audio is stopped, or the binding has not taken effect yet
    /// (the caller knows the phase and worded the two cases apart).
    NotRunning,
    /// A live stream still runs on this device, but the candidate graph no
    /// longer binds this node to it — a hot edit was rejected or has not
    /// been applied, and the old plan keeps playing.
    Detached,
}

pub(crate) struct DeviceUser {
    pub node: NodeId,
    /// The node's own label, exactly as the graph shows it.
    pub label: String,
    pub state: UserState,
}

/// The nodes the candidate graph binds to `device`. Mirrors the engine's
/// projection: a node claimed by an *enabled* virtual device is not a
/// physical device's user, whatever stale fields a hand-written project
/// file carries.
pub(crate) fn bound_nodes(graph: &GraphSnapshot, device: &DeviceKey) -> Vec<NodeId> {
    match device {
        DeviceKey::Physical { endpoint_id, input } => graph
            .nodes
            .iter()
            .filter(|entry| {
                let bound = if *input {
                    matches!(
                        &entry.source,
                        Some(InputSelection::Physical { endpoint_id: id }) if id == endpoint_id
                    )
                } else {
                    entry.output_endpoint_id.as_deref() == Some(endpoint_id.as_str())
                };
                bound && !claimed_by_enabled_export(graph, entry.id)
            })
            .map(|entry| entry.id)
            .collect(),
        DeviceKey::Virtual(endpoint_id) => graph
            .exports
            .iter()
            .filter(|export| export.endpoint_id == *endpoint_id)
            .filter_map(|export| export.binding)
            .collect(),
    }
}

/// An enabled export claiming the node: the engine routes such a node
/// through the virtual boundary, not the physical device.
fn claimed_by_enabled_export(graph: &GraphSnapshot, node: NodeId) -> bool {
    graph
        .exports
        .iter()
        .any(|export: &ExportIntent| export.enabled && export.binding == Some(node))
}

/// The nodes with a live stream on `device`, from the running engine's
/// observations. Streams are attributed by endpoint identity, not by the
/// candidate graph's bindings, so a rejected hot edit cannot make the old
/// stream count as the new device's.
#[cfg(windows)]
pub(crate) fn live_nodes(snapshot: &AppSnapshot, device: &DeviceKey) -> Vec<NodeId> {
    crate::ui::presentation::professional_observations(snapshot)
        .map(|observations| {
            observations
                .streams
                .iter()
                .filter(|stream| endpoint_matches(stream, device))
                .map(|stream| stream.node)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub(crate) fn live_nodes(_snapshot: &AppSnapshot, _device: &DeviceKey) -> Vec<NodeId> {
    Vec::new()
}

#[cfg(windows)]
fn endpoint_matches(
    stream: &moiren_windows_audio::engine_backend::ProfessionalStreamObservation,
    device: &DeviceKey,
) -> bool {
    use moiren_windows_audio::engine_backend::StreamEndpoint;
    match device {
        DeviceKey::Physical { endpoint_id, input } => {
            stream.input == *input
                && matches!(&stream.endpoint, StreamEndpoint::Physical(id) if id == endpoint_id)
        }
        DeviceKey::Virtual(endpoint_id) => {
            matches!(&stream.endpoint, StreamEndpoint::Virtual(id) if id == endpoint_id)
        }
    }
}

/// The merge behind the monitor section: bound nodes and live streams are
/// two independent facts about a device, and every combination is
/// meaningful (design 23 §3.3).
pub(crate) fn device_users(snapshot: &AppSnapshot, device: &DeviceKey) -> Vec<DeviceUser> {
    merge_users(
        snapshot,
        &bound_nodes(&snapshot.graph, device),
        &live_nodes(snapshot, device),
    )
}

fn merge_users(snapshot: &AppSnapshot, bound: &[NodeId], live: &[NodeId]) -> Vec<DeviceUser> {
    bound
        .iter()
        .chain(live)
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|node| DeviceUser {
            node,
            label: snapshot
                .graph
                .node(node)
                .map(|entry| entry.label.clone())
                .unwrap_or_else(|| format!("{node:?}")),
            state: match (bound.contains(&node), live.contains(&node)) {
                (true, true) => UserState::Running,
                (true, false) => UserState::NotRunning,
                (false, true) => UserState::Detached,
                (false, false) => unreachable!("node came from one of the two inputs"),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ExportDirection, ServiceGraph};
    use moiren_core::graph::NodeKind;

    /// A service graph with its default nodes plus a second Source: the
    /// ids physical bindings can be attached to, and one spare node.
    fn graph_with_state() -> (GraphSnapshot, NodeId, NodeId, NodeId) {
        let mut service = ServiceGraph::default();
        let sink = service
            .snapshot()
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Sink)
            .unwrap()
            .id();
        let primary_source = service
            .snapshot()
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .unwrap()
            .id();
        service
            .edit(crate::service::GraphEdit::AddNode {
                kind: NodeKind::Source,
                channels: 2,
            })
            .unwrap();
        let second_source = service
            .snapshot()
            .graph
            .nodes()
            .iter()
            .filter(|node| node.kind() == NodeKind::Source)
            .map(|node| node.id())
            .max()
            .unwrap();
        (service.snapshot(), primary_source, second_source, sink)
    }

    #[test]
    fn physical_users_follow_bindings_and_direction() {
        let (mut graph, source, second_source, sink) = graph_with_state();
        let device = DeviceKey::Physical {
            endpoint_id: "microphone".into(),
            input: true,
        };
        assert!(bound_nodes(&graph, &device).is_empty());
        for node in [source, second_source] {
            graph
                .nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .unwrap()
                .source = Some(InputSelection::Physical {
                endpoint_id: "microphone".into(),
            });
        }
        assert_eq!(bound_nodes(&graph, &device), vec![source, second_source]);
        // The sink's output binding does not leak into the input direction.
        let output_device = DeviceKey::Physical {
            endpoint_id: "microphone".into(),
            input: false,
        };
        assert!(bound_nodes(&graph, &output_device).is_empty());
        graph
            .nodes
            .iter_mut()
            .find(|entry| entry.id == sink)
            .unwrap()
            .output_endpoint_id = Some("microphone".into());
        assert_eq!(bound_nodes(&graph, &output_device), vec![sink]);
    }

    #[test]
    fn an_enabled_virtual_export_claims_its_node_away_from_the_physical_device() {
        let (mut graph, source, _second_source, _sink) = graph_with_state();
        graph
            .nodes
            .iter_mut()
            .find(|entry| entry.id == source)
            .unwrap()
            .source = Some(InputSelection::Physical {
            endpoint_id: "microphone".into(),
        });
        let virtual_device = DeviceKey::Virtual(VirtualEndpointId::from_bytes([9; 16]));
        graph.exports.push(ExportIntent {
            endpoint_id: VirtualEndpointId::from_bytes([9; 16]),
            direction: ExportDirection::Playback,
            name: "speaker".into(),
            channels: 2,
            enabled: true,
            binding: Some(source),
        });
        let physical = DeviceKey::Physical {
            endpoint_id: "microphone".into(),
            input: true,
        };
        assert!(
            bound_nodes(&graph, &physical).is_empty(),
            "the enabled export routes the node virtually"
        );
        assert_eq!(bound_nodes(&graph, &virtual_device), vec![source]);
        // The engine only honors enabled exports; a disabled one leaves the
        // retained physical binding in charge, and the export's candidate
        // binding still lists the node as the virtual device's user.
        graph.exports[0].enabled = false;
        assert_eq!(bound_nodes(&graph, &physical), vec![source]);
        assert_eq!(bound_nodes(&graph, &virtual_device), vec![source]);
    }

    #[test]
    fn merging_bound_and_live_nodes_yields_the_three_states() {
        let runtime = crate::ui::spawn_service();
        let handle = runtime.handle();
        let state =
            crate::ui::state::UiState::new(handle.clone(), crate::ui::initial_projection(&handle));
        let (graph, bound_source, spare_source, _sink) = graph_with_state();
        let mut snapshot = (*state.snapshot).clone();
        snapshot.graph = std::sync::Arc::new(graph);
        let users = merge_users(&snapshot, &[bound_source], &[bound_source]);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].state, UserState::Running);
        assert!(!users[0].label.is_empty());

        let users = merge_users(&snapshot, &[bound_source], &[]);
        assert_eq!(users[0].state, UserState::NotRunning);

        let users = merge_users(&snapshot, &[bound_source], &[spare_source]);
        assert_eq!(users.len(), 2);
        assert_eq!(
            users
                .iter()
                .find(|user| user.node == bound_source)
                .unwrap()
                .state,
            UserState::NotRunning
        );
        assert_eq!(
            users
                .iter()
                .find(|user| user.node == spare_source)
                .unwrap()
                .state,
            UserState::Detached
        );
        crate::ui::finish_service(runtime);
    }
}
