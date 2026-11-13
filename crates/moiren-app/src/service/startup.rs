use moiren_core::graph::{NodeId, NodeKind};

use super::{AudioBackend, DeviceCatalog, GraphSnapshot, InputSelection, SessionSpec};

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
        gain: 0.0,
        pan: 0.0,
        max_block_frames: 256,
    };
    spec.validate().ok()?;
    Some(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{DeviceRow, GraphEdit, ServiceGraph};

    fn fixture() -> DeviceCatalog {
        DeviceCatalog {
            inputs: vec![DeviceRow {
                endpoint_id: "input".into(),
                name: "input".into(),
            }],
            outputs: vec![DeviceRow {
                endpoint_id: "output".into(),
                name: "output".into(),
            }],
            default_input_endpoint_id: Some("input".into()),
            default_output_endpoint_id: Some("output".into()),
            ..Default::default()
        }
    }

    #[test]
    fn default_session_is_muted_and_follows_the_default_endpoints() {
        let session = default_session_for_backend(&fixture(), AudioBackend::Wasapi).unwrap();
        assert_eq!(session.gain, 0.0);
        assert_eq!(session.pan, 0.0);
        assert_eq!(
            session.source,
            InputSelection::Physical {
                endpoint_id: "input".into()
            }
        );
    }

    #[test]
    fn asio_never_uses_wasapi_defaults_for_automatic_startup() {
        assert!(default_session_for_backend(&fixture(), AudioBackend::Asio).is_none());
        assert!(default_session_for_backend(&fixture(), AudioBackend::Wasapi).is_some());
    }

    #[test]
    fn owned_vad_defaults_never_enter_the_physical_startup_path() {
        for endpoint in ["input", "output"] {
            let mut catalog = fixture();
            catalog.vad_endpoints.insert(
                moiren_engine::node::VirtualEndpointId::from_bytes([0x42; 16]),
                endpoint.into(),
            );
            assert!(default_session_for_backend(&catalog, AudioBackend::Wasapi).is_none());
        }
    }

    #[test]
    fn missing_or_stale_defaults_never_choose_an_arbitrary_device() {
        let mut catalog = fixture();
        catalog.default_input_endpoint_id = None;
        assert!(default_session_for_backend(&catalog, AudioBackend::Wasapi).is_none());
        catalog.default_input_endpoint_id = Some("vanished".into());
        assert!(default_session_for_backend(&catalog, AudioBackend::Wasapi).is_none());
        catalog.default_input_endpoint_id = Some("input".into());
        catalog.default_output_endpoint_id = Some("vanished".into());
        assert!(default_session_for_backend(&catalog, AudioBackend::Wasapi).is_none());
        catalog.default_output_endpoint_id = Some("output".into());
        catalog.outputs.clear();
        assert!(default_session_for_backend(&catalog, AudioBackend::Wasapi).is_none());
    }

    /// The default stereo chain bound to the fixture devices, plus whatever
    /// `extend` adds.
    fn restored_graph(extend: impl FnOnce(&mut ServiceGraph)) -> ServiceGraph {
        let mut graph = ServiceGraph::default();
        let source = graph
            .snapshot()
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        let sink = graph
            .snapshot()
            .node_of_kind(NodeKind::Sink)
            .unwrap()
            .0
            .id();
        graph
            .edit(GraphEdit::BindSource {
                node: source,
                source: InputSelection::Physical {
                    endpoint_id: "input".into(),
                },
            })
            .unwrap();
        graph
            .edit(GraphEdit::BindOutput {
                node: sink,
                endpoint_id: "output".into(),
            })
            .unwrap();
        extend(&mut graph);
        graph
    }

    fn add_source(graph: &mut ServiceGraph, label: &str, endpoint_id: &str) -> NodeId {
        graph
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Source,
                channels: 2,
            })
            .unwrap();
        let node = graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .rfind(|node| node.kind() == NodeKind::Source)
            .unwrap()
            .id();
        graph
            .edit(GraphEdit::SetLabel {
                node,
                label: label.into(),
            })
            .unwrap();
        graph
            .edit(GraphEdit::BindSource {
                node,
                source: InputSelection::Physical {
                    endpoint_id: endpoint_id.into(),
                },
            })
            .unwrap();
        node
    }

    #[test]
    fn offline_io_is_strippable_only_beside_an_online_sibling() {
        let mut extra = None;
        let graph = restored_graph(|graph| {
            extra = Some(add_source(graph, "Unplugged mic", "missing-input"));
        });
        let offline = strippable_offline_io(&graph.snapshot(), &fixture());
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].node, extra.unwrap());
        assert_eq!(offline[0].label, "Unplugged mic");
        assert_eq!(offline[0].kind, NodeKind::Source);
        assert_eq!(offline[0].endpoint_id, "missing-input");

        // The only Source offline: stripping it would leave an unstartable
        // graph, so the host must wait for the device instead.
        let graph = restored_graph(|graph| {
            let source = graph
                .snapshot()
                .node_of_kind(NodeKind::Source)
                .unwrap()
                .0
                .id();
            graph
                .edit(GraphEdit::BindSource {
                    node: source,
                    source: InputSelection::Physical {
                        endpoint_id: "missing-input".into(),
                    },
                })
                .unwrap();
        });
        assert!(strippable_offline_io(&graph.snapshot(), &fixture()).is_empty());

        // Two offline Sources and no online one: still nothing to strip.
        let graph = restored_graph(|graph| {
            let source = graph
                .snapshot()
                .node_of_kind(NodeKind::Source)
                .unwrap()
                .0
                .id();
            graph
                .edit(GraphEdit::BindSource {
                    node: source,
                    source: InputSelection::Physical {
                        endpoint_id: "missing-a".into(),
                    },
                })
                .unwrap();
            add_source(graph, "Second", "missing-b");
        });
        assert!(strippable_offline_io(&graph.snapshot(), &fixture()).is_empty());
    }

    #[test]
    fn device_free_and_owned_bindings_are_never_offline() {
        // A tone source needs no device and counts as the online sibling that
        // makes the unplugged physical source strippable.
        let graph = restored_graph(|graph| {
            let source = graph
                .snapshot()
                .node_of_kind(NodeKind::Source)
                .unwrap()
                .0
                .id();
            graph
                .edit(GraphEdit::BindSource {
                    node: source,
                    source: InputSelection::Tone {
                        frequency_hz: 440.0,
                    },
                })
                .unwrap();
            add_source(graph, "Unplugged mic", "missing-input");
        });
        let offline = strippable_offline_io(&graph.snapshot(), &fixture());
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].label, "Unplugged mic");

        // An output the export runtime owns may appear after the first catalog
        // enumeration; it is matched through the owned-endpoint table.
        let graph = restored_graph(|graph| {
            let sink = graph
                .snapshot()
                .node_of_kind(NodeKind::Sink)
                .unwrap()
                .0
                .id();
            graph
                .edit(GraphEdit::BindOutput {
                    node: sink,
                    endpoint_id: "{owned-virtual-speaker}".into(),
                })
                .unwrap();
        });
        let mut catalog = fixture();
        catalog.vad_endpoints.insert(
            moiren_engine::node::VirtualEndpointId::from_bytes([0x42; 16]),
            "{owned-virtual-speaker}".into(),
        );
        assert!(strippable_offline_io(&graph.snapshot(), &catalog).is_empty());
    }
}
