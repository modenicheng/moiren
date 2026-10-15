use super::*;
use moiren_engine::boundary::ConstantSource;

fn ids(service: &ServiceGraph) -> (NodeId, NodeId, NodeId, NodeId) {
    let nodes = service.snapshot.graph.nodes();
    (nodes[0].id(), nodes[1].id(), nodes[2].id(), nodes[3].id())
}
fn render(service: &ServiceGraph) -> Vec<f32> {
    let mut prepared = crate::session_graph::prepare_routing(
        &service.preparation(),
        ConstantSource {
            channels: 2,
            value: 1.0,
        },
        32,
        None,
    )
    .unwrap();
    assert_eq!(
        prepared.compiled.engine.revision(),
        service.snapshot.candidate_revision
    );
    prepared.compiled.engine.render(8).unwrap();
    let mut samples = vec![0.0; 16];
    assert_eq!(
        prepared
            .output
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        8
    );
    samples
}
#[test]
fn rejected_edits_are_atomic_and_do_not_consume_revision_or_node_ids() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    let before = service.snapshot();
    let output = before.graph.get_node(pan).unwrap().outputs()[0].id();
    let input = before.graph.get_node(gain).unwrap().inputs()[0].id();
    assert!(
        service
            .edit(GraphEdit::Connect {
                source: output,
                destination: input
            })
            .is_err()
    );
    let after = service.snapshot();
    assert_eq!(before.graph, after.graph);
    assert_eq!(before.nodes, after.nodes);
    assert_eq!(before.candidate_revision, after.candidate_revision);
    assert!(after.last_error.is_some());
    assert!(
        service
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: f64::NAN
            })
            .is_err()
    );
    assert_eq!(before.graph, service.snapshot.graph);
}
#[test]
fn candidate_edit_does_not_claim_native_application_and_stale_start_cannot_confirm_it() {
    let mut service = ServiceGraph::default();
    let old = service.snapshot.candidate_revision;
    service.preparing(SessionGeneration(1), old);
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .unwrap();
    assert_eq!(service.snapshot.active_revision, None);
    service.running(SessionGeneration(2), old);
    assert_eq!(service.snapshot.active_revision, None);
    service.running(SessionGeneration(1), old);
    assert_eq!(service.snapshot.active_revision, Some(old));
    assert_ne!(
        service.snapshot.active_revision,
        Some(service.snapshot.candidate_revision)
    );
    service.stopped();
    assert_eq!(service.snapshot.active_revision, None);
}
#[test]
fn edited_gain_pan_and_edge_send_reach_real_compiled_audio() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetPan {
            node: pan,
            pan: 0.5,
        })
        .unwrap();
    assert_eq!(render(&service), [0.25, 0.5].repeat(8));
    let edge = service.snapshot.graph.edges()[2].id();
    service
        .edit(GraphEdit::SetSend {
            edge,
            params: SendParams {
                mute: true,
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(render(&service), vec![0.0; 16]);
    service.edit(GraphEdit::Disconnect(edge)).unwrap();
    assert_eq!(render(&service), vec![0.0; 16]);
}
#[test]
fn deleting_processors_and_reconnecting_a_direct_route_changes_audio() {
    let mut service = ServiceGraph::default();
    let (source, gain, pan, sink) = ids(&service);
    service.edit(GraphEdit::RemoveNode(gain)).unwrap();
    service.edit(GraphEdit::RemoveNode(pan)).unwrap();
    let graph = &service.snapshot.graph;
    let source = graph.get_node(source).unwrap().outputs()[0].id();
    let destination = graph.get_node(sink).unwrap().inputs()[0].id();
    service
        .edit(GraphEdit::Connect {
            source,
            destination,
        })
        .unwrap();
    assert_eq!(service.preparation().gain_pan_nodes(), None);
    assert_eq!(render(&service), vec![1.0; 16]);
}
#[test]
fn bus_input_and_additional_gain_nodes_are_compiled_with_individual_values() {
    let mut service = ServiceGraph::default();
    let (source, gain, pan, sink) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Gain,
            channels: 2,
        })
        .unwrap();
    let extra_gain = service.snapshot.graph.nodes().last().unwrap().id();
    service
        .edit(GraphEdit::SetGain {
            node: extra_gain,
            gain: 0.25,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .unwrap();
    let bus = service.snapshot.graph.nodes().last().unwrap().id();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    let old_output = service
        .snapshot
        .graph
        .edges()
        .iter()
        .find(|e| e.dst() == sink)
        .unwrap()
        .id();
    service.edit(GraphEdit::Disconnect(old_output)).unwrap();
    for (src, dst, input_index) in [
        (source, extra_gain, 0),
        (pan, bus, 0),
        (extra_gain, bus, 1),
        (bus, sink, 0),
    ] {
        let graph = &service.snapshot.graph;
        let source = graph.get_node(src).unwrap().outputs()[0].id();
        let destination = graph.get_node(dst).unwrap().inputs()[input_index].id();
        service
            .edit(GraphEdit::Connect {
                source,
                destination,
            })
            .unwrap();
    }
    assert_eq!(render(&service), vec![0.75; 16]);
}
#[test]
fn unsupported_device_width_or_multiple_sources_are_reported_without_losing_candidate() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Source,
            channels: 2,
        })
        .unwrap();
    assert!(service.session_spec(None).unwrap_err().contains("1 个输入"));
    assert_eq!(service.snapshot.graph.nodes().len(), 5);
    assert_eq!(service.snapshot.active_revision, None);
}

#[test]
fn mono_and_swap_candidate_values_are_compiled_and_reject_other_node_kinds() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 1.0,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetPan {
            node: pan,
            pan: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetSwap {
            node: pan,
            swap: true,
        })
        .unwrap();
    assert_eq!(render(&service), [1.0, 0.5].repeat(8));
    service
        .edit(GraphEdit::SetMono {
            node: pan,
            mono: true,
        })
        .unwrap();
    assert_eq!(render(&service), [0.75, 0.75].repeat(8));
    let before = service.snapshot();
    assert!(
        service
            .edit(GraphEdit::SetMono {
                node: gain,
                mono: true
            })
            .is_err()
    );
    assert_eq!(before.nodes, service.snapshot.nodes);
    assert_eq!(
        before.candidate_revision,
        service.snapshot.candidate_revision
    );
}
