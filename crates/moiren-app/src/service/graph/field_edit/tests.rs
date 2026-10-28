use super::super::*;
use moiren_engine::processor::{ChannelMapperSettings, DelaySettings, EqSettings};

mod eq;
mod mapper;
mod scalar;

fn fixture(
    kind: NodeKind,
    inputs: usize,
    outputs: usize,
    builtin: BuiltinState,
) -> (ServiceGraph, NodeId) {
    let mut topology = LogicalGraph::new();
    let node = if kind == NodeKind::ChannelMapper {
        topology.create_channel_mapper(inputs, outputs).unwrap()
    } else {
        topology.create_node(kind, outputs).unwrap()
    };
    let state = GraphNodeState {
        id: node,
        label: "Preserved node label".into(),
        gain: 1.,
        pan: 0.,
        mono: false,
        swap: false,
        mixer: MixerState::default(),
        compressor: CompressorState::default(),
        builtin,
        source: None,
        output_endpoint_id: None,
    };
    let mut service = ServiceGraph::default();
    service.replace(topology, vec![state]).unwrap();
    service.journal = UndoJournal::default();
    (service, node)
}

fn builtin(service: &ServiceGraph, node: NodeId) -> BuiltinState {
    service.snapshot().node(node).unwrap().builtin.clone()
}

fn assert_rejected(service: &mut ServiceGraph, edit: GraphEdit) {
    let before = service.snapshot();
    let undo = service.journal.undo.clone();
    let redo = service.journal.redo.clone();
    let last = service.journal.last.clone();
    assert!(service.edit(edit).is_err());
    let after = service.snapshot();
    assert_eq!(after.nodes, before.nodes);
    assert_eq!(after.graph, before.graph);
    assert_eq!(after.candidate_revision, before.candidate_revision);
    assert_eq!(after.active_revision, before.active_revision);
    assert_eq!(after.pending_revision, before.pending_revision);
    assert_eq!(service.journal.undo, undo);
    assert_eq!(service.journal.redo, redo);
    assert_eq!(service.journal.last, last);
}

#[test]
fn field_edits_have_node_subjects_and_rebuild_without_inventing_live_parameters() {
    let (service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    for edit in [
        GraphEdit::SetMapperWeight {
            node,
            input: 0,
            output: 0,
            weight: -2.,
        },
        GraphEdit::MapperPreset { node, swap: true },
        GraphEdit::SetBuiltinParameter {
            node,
            parameter: RackParameter::EqGain,
            value: 3.,
        },
        GraphEdit::SetEqBand {
            node,
            frequency_hz: 2000.,
            gain_db: 3.,
        },
        GraphEdit::AdjustEqQ { node, factor: 1.5 },
    ] {
        assert_eq!(
            edit_subject(&edit, &service.snapshot().graph),
            GraphSubject::Node(node)
        );
        assert!(edit_affects_prepared_structure(
            &edit,
            &service.snapshot().graph
        ));
        assert!(edit_parameter_values(&edit).is_empty());
    }
}

#[test]
fn field_edits_reject_missing_nodes_without_creating_replacement_state() {
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    service.edit(GraphEdit::RemoveNode(node)).unwrap();
    for edit in [
        GraphEdit::SetMapperWeight {
            node,
            input: 0,
            output: 0,
            weight: 1.,
        },
        GraphEdit::MapperPreset { node, swap: false },
        GraphEdit::SetBuiltinParameter {
            node,
            parameter: RackParameter::EqQ,
            value: 1.,
        },
        GraphEdit::SetEqBand {
            node,
            frequency_hz: 2000.,
            gain_db: 3.,
        },
        GraphEdit::AdjustEqQ { node, factor: 1.5 },
    ] {
        assert_rejected(&mut service, edit);
    }
}
