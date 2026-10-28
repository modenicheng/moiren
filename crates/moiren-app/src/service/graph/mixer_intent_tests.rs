use super::*;
use moiren_engine::{host::StripParameter, processor::Pan};

fn node_of(service: &ServiceGraph, kind: NodeKind) -> NodeId {
    service.snapshot.node_of_kind(kind).unwrap().0.id()
}

fn mixer(node: NodeId, edit: MixerEdit) -> GraphEdit {
    GraphEdit::EditMixer { node, edit }
}

#[test]
fn mixer_intents_merge_fields_in_both_orders_without_replacing_siblings() {
    for mute_first in [false, true] {
        let mut service = ServiceGraph::default();
        let node = node_of(&service, NodeKind::Source);
        let initial = MixerState {
            balance: 0.25,
            dimmed: true,
            solo: true,
            channel_mode: 3,
            ..Default::default()
        };
        service
            .edit(GraphEdit::SetMixer {
                node,
                state: initial,
            })
            .unwrap();
        let mut edits = [MixerEdit::SetGain(0.5), MixerEdit::ToggleMute];
        if mute_first {
            edits.reverse();
        }
        for edit in edits {
            assert!(!service.edit(mixer(node, edit)).unwrap());
        }
        assert_eq!(
            service.snapshot.node(node).unwrap().mixer,
            MixerState {
                gain: 0.5,
                muted: true,
                ..initial
            }
        );
    }
}

#[test]
fn mixer_gain_and_balance_have_independent_coalescing_keys() {
    let mut service = ServiceGraph::default();
    let node = node_of(&service, NodeKind::Source);
    let edits = [
        MixerEdit::SetGain(0.5),
        MixerEdit::SetGain(0.25),
        MixerEdit::SetBalance(0.5),
        MixerEdit::AdjustBalance(0.125),
    ];
    assert_eq!(
        mixer(node, edits[0]).undo_key(),
        mixer(node, edits[1]).undo_key()
    );
    assert_eq!(
        mixer(node, edits[2]).undo_key(),
        mixer(node, edits[3]).undo_key()
    );
    assert_ne!(
        mixer(node, edits[0]).undo_key(),
        mixer(node, edits[2]).undo_key()
    );
    for edit in edits {
        service.edit(mixer(node, edit)).unwrap();
    }
    assert_eq!(service.journal.undo.len(), 2);
    assert!(service.undo());
    assert_eq!(service.snapshot.node(node).unwrap().mixer.gain, 0.25);
    assert_eq!(service.snapshot.node(node).unwrap().mixer.balance, 0.0);
    assert!(service.undo());
    assert_eq!(
        service.snapshot.node(node).unwrap().mixer,
        MixerState::default()
    );
    assert!(service.redo());
    assert!(service.redo());
    assert_eq!(service.snapshot.node(node).unwrap().mixer.balance, 0.625);
}

#[test]
fn mixer_and_pan_toggles_resolve_latest_state_and_undo_every_click() {
    let source = node_of(&ServiceGraph::default(), NodeKind::Source);
    let pan = node_of(&ServiceGraph::default(), NodeKind::Pan);
    for edit in [
        mixer(source, MixerEdit::ToggleMute),
        mixer(source, MixerEdit::ToggleSolo),
        mixer(source, MixerEdit::ToggleDim),
        GraphEdit::ToggleMono { node: pan },
        GraphEdit::ToggleSwap { node: pan },
    ] {
        let mut service = ServiceGraph::default();
        let original = service.snapshot.nodes.clone();
        assert_eq!(edit.undo_key(), None);
        assert!(!service.edit(edit.clone()).unwrap());
        let once = service.snapshot.nodes.clone();
        assert_ne!(once, original);
        assert!(!service.edit(edit).unwrap());
        assert_eq!(service.snapshot.nodes, original);
        assert_eq!(service.journal.undo.len(), 2);
        assert!(service.undo());
        assert_eq!(service.snapshot.nodes, once);
        assert!(service.undo());
        assert_eq!(service.snapshot.nodes, original);
        assert!(service.redo());
        assert_eq!(service.snapshot.nodes, once);
        assert!(service.redo());
        assert_eq!(service.snapshot.nodes, original);
    }
}

#[test]
fn mixer_cycle_uses_current_channel_width_and_never_coalesces() {
    for channels in [1, 2, 32, 256] {
        let mut service = ServiceGraph::default();
        service
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Source,
                channels,
            })
            .unwrap();
        let node = service.snapshot.nodes.last().unwrap().id;
        let edit = mixer(node, MixerEdit::CycleMode);
        assert_eq!(edit.undo_key(), None);
        for expected in 1..=channels + 3 {
            assert!(!service.edit(edit.clone()).unwrap());
            assert_eq!(
                service.snapshot.node(node).unwrap().mixer.channel_mode,
                expected % (channels + 3)
            );
        }
        assert!(service.undo());
        assert_eq!(
            service.snapshot.node(node).unwrap().mixer.channel_mode,
            channels + 2
        );
    }
}

#[test]
fn mixer_adjust_balance_is_relative_and_clamps_only_finite_deltas() {
    let mut service = ServiceGraph::default();
    let node = node_of(&service, NodeKind::Source);
    for (edit, expected) in [
        (MixerEdit::SetBalance(0.5), 0.5),
        (MixerEdit::AdjustBalance(0.125), 0.625),
        (MixerEdit::AdjustBalance(0.125), 0.75),
        (MixerEdit::AdjustBalance(f64::MAX), 1.0),
        (MixerEdit::AdjustBalance(-f64::MAX), -1.0),
    ] {
        service.edit(mixer(node, edit)).unwrap();
        assert_eq!(service.snapshot.node(node).unwrap().mixer.balance, expected);
    }
}

#[test]
fn mixer_invalid_intents_preserve_candidate_history_and_fault_subject() {
    let mut service = ServiceGraph::default();
    let node = node_of(&service, NodeKind::Source);
    service.edit(mixer(node, MixerEdit::SetGain(0.5))).unwrap();
    assert!(service.undo());
    let before = service.snapshot.clone();
    let undo = service.journal.undo.len();
    let redo = service.journal.redo.len();
    let invalid = [
        MixerEdit::SetGain(-0.1),
        MixerEdit::SetGain(16.1),
        MixerEdit::SetGain(f64::NAN),
        MixerEdit::SetGain(f64::INFINITY),
        MixerEdit::SetBalance(-1.1),
        MixerEdit::SetBalance(1.1),
        MixerEdit::SetBalance(f64::NAN),
        MixerEdit::SetBalance(f64::NEG_INFINITY),
        MixerEdit::AdjustBalance(f64::NAN),
        MixerEdit::AdjustBalance(f64::INFINITY),
        MixerEdit::AdjustBalance(f64::NEG_INFINITY),
    ];
    for edit in invalid.into_iter().map(|edit| mixer(node, edit)).chain([
        GraphEdit::ToggleMono { node },
        GraphEdit::ToggleSwap { node },
    ]) {
        assert!(service.edit(edit).is_err());
        assert_eq!(service.snapshot.nodes, before.nodes);
        assert_eq!(
            service.snapshot.candidate_revision,
            before.candidate_revision
        );
        assert_eq!(service.journal.undo.len(), undo);
        assert_eq!(service.journal.redo.len(), redo);
        assert_eq!(service.snapshot.can_redo, before.can_redo);
        assert!(
            service
                .snapshot
                .faults
                .iter()
                .all(|fault| fault.subject == GraphSubject::Node(node))
        );
    }
}

#[test]
fn mixer_removed_node_intents_are_rejected() {
    let mut service = ServiceGraph::default();
    let node = node_of(&service, NodeKind::Pan);
    service.edit(GraphEdit::RemoveNode(node)).unwrap();
    let before = service.snapshot.clone();
    let undo = service.journal.undo.len();
    for edit in [
        mixer(node, MixerEdit::SetGain(0.5)),
        mixer(node, MixerEdit::CycleMode),
        GraphEdit::ToggleMono { node },
        GraphEdit::ToggleSwap { node },
    ] {
        assert!(service.edit(edit).is_err());
        assert_eq!(service.snapshot.nodes, before.nodes);
        assert_eq!(
            service.snapshot.candidate_revision,
            before.candidate_revision
        );
        assert_eq!(service.journal.undo.len(), undo);
    }
}

#[test]
fn mixer_accepted_mapping_reads_final_fields_without_reapplying_intents() {
    let mut service = ServiceGraph::default();
    let node = node_of(&service, NodeKind::Pan);
    for edit in [
        MixerEdit::SetBalance(0.5),
        MixerEdit::AdjustBalance(0.25),
        MixerEdit::CycleMode,
    ] {
        let edit = mixer(node, edit);
        service.edit(edit.clone()).unwrap();
        let state = service.snapshot.node(node).unwrap().mixer;
        assert_eq!(
            service.accepted_parameter_values(&edit),
            edit_parameter_values(&GraphEdit::SetMixer { node, state })
        );
    }
    assert_eq!(service.snapshot.node(node).unwrap().mixer.balance, 0.75);
    assert_eq!(service.snapshot.node(node).unwrap().mixer.channel_mode, 1);
    for (edit, parameter) in [
        (GraphEdit::ToggleMono { node }, Pan::MONO),
        (GraphEdit::ToggleSwap { node }, Pan::SWAP),
    ] {
        for expected in [true, false] {
            service.edit(edit.clone()).unwrap();
            assert_eq!(
                service.accepted_parameter_values(&edit),
                vec![(
                    ParameterTarget::Node { node, parameter },
                    ParamValue::Bool(expected)
                )]
            );
        }
    }
    let edit = mixer(node, MixerEdit::ToggleMute);
    service.edit(edit.clone()).unwrap();
    let values = service.accepted_parameter_values(&edit);
    assert!(
        values.iter().all(|(target, _)| matches!(
            target,
            ParameterTarget::Strip {
                parameter: StripParameter::Balance | StripParameter::Mode,
                ..
            }
        )),
        "effective gain/mute belong to whole-graph strip synchronization"
    );
}
