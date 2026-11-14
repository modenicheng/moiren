use moiren_core::graph::{NodeId, NodeKind};

use super::{AudioBackend, DeviceCatalog, GraphSnapshot, InputSelection, RunLimit, SessionSpec};

/// A restored Source or Sink whose physical device is absent from the
/// catalog, so it cannot take part in the resumed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OfflineIo {
    pub node: NodeId,
    pub label: String,
    pub kind: NodeKind,
    pub endpoint_id: String,
}

/// The restored IO nodes a resumed session runs without.
///
/// Offline means a Source bound to a physical input, or a Sink bound to an
/// output, whose device the catalog does not list. Software and application
/// sources need no device, and export-bound nodes use Moiren-owned virtual
/// endpoints that the export runtime may create after the first catalog, so
/// neither is ever reported here.
///
/// The graph needs one Source and one Sink to start, so a node is only
/// strippable while an online node of its kind remains. When every node of
/// a kind is offline nothing is stripped: the host waits for the devices and
/// recovers on its own when they return.
pub(super) fn strippable_offline_io(
    snapshot: &GraphSnapshot,
    catalog: &DeviceCatalog,
) -> Vec<OfflineIo> {
    let mut offline = Vec::new();
    let (mut online_sources, mut online_sinks) = (0usize, 0usize);
    for node in snapshot.graph.nodes() {
        let kind = node.kind();
        if !matches!(kind, NodeKind::Source | NodeKind::Sink) {
            continue;
        }
        let Some(state) = snapshot.nodes.iter().find(|state| state.id == node.id()) else {
            continue;
        };
        let online = match kind {
            NodeKind::Source => &mut online_sources,
            _ => &mut online_sinks,
        };
        if snapshot
            .exports
            .iter()
            .any(|export| export.binding == Some(node.id()))
        {
            *online += 1;
            continue;
        }
        let presence = match kind {
            NodeKind::Source => match &state.source {
                None => None,
                Some(InputSelection::Physical { endpoint_id }) => Some((
                    catalog
                        .inputs
                        .iter()
                        .any(|row| row.endpoint_id == *endpoint_id),
                    endpoint_id.clone(),
                )),
                Some(_) => Some((true, String::new())),
            },
            _ => state.output_endpoint_id.as_ref().map(|endpoint_id| {
                (
                    catalog
                        .outputs
                        .iter()
                        .any(|row| row.endpoint_id == *endpoint_id)
                        || catalog
                            .vad_endpoints
                            .values()
                            .any(|owned| owned == endpoint_id),
                    endpoint_id.clone(),
                )
            }),
        };
        match presence {
            // An unbound node cannot start anyway, so it is no fallback sibling.
            None => {}
            Some((true, _)) => *online += 1,
            Some((false, endpoint_id)) => offline.push(OfflineIo {
                node: node.id(),
                label: state.label.clone(),
                kind,
                endpoint_id,
            }),
        }
    }
    offline.retain(|io| match io.kind {
        NodeKind::Source => online_sources > 0,
        _ => online_sinks > 0,
    });
    offline
}

/// WASAPI owns system defaults; ASIO has no safe default driver binding in the
/// preferences, so automatic startup remains idle until a stable driver row is
/// selected explicitly.
pub(super) fn default_session_for_backend(
    catalog: &DeviceCatalog,
    backend: AudioBackend,
) -> Option<SessionSpec> {
    if backend == AudioBackend::Asio {
        return None;
    }
    let input = catalog.default_input_endpoint_id.as_ref()?;
    let output = catalog.default_output_endpoint_id.as_ref()?;
    if catalog
        .vad_endpoints
        .values()
        .any(|id| id == input || id == output)
    {
        return None;
    }
    if !catalog.inputs.iter().any(|row| row.endpoint_id == *input)
        || !catalog.outputs.iter().any(|row| row.endpoint_id == *output)
    {
        return None;
    }
    let spec = SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: input.clone(),
        },
        output_endpoint_id: output.clone(),
        limit: RunLimit::UntilStopped,
        gain: 0.0,
        pan: 0.0,
        max_block_frames: 256,
    };
    spec.validate().ok()?;
    Some(spec)
}
