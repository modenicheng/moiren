use super::*;
use moiren_engine::{
    processor::{ChannelMapperSettings, EqSettings},
    rack::{RackProcessor, RackRoute, RackSettings, RackSlot},
};

mod toggles;

fn slot(id: u32, processor: RackProcessor) -> RackSlot {
    RackSlot {
        id,
        bypass: false,
        processor,
    }
}

fn fixture(settings: RackSettings, inputs: &[usize], outputs: &[usize]) -> (ServiceGraph, NodeId) {
    let mut service = ServiceGraph::default();
    let mut snapshot = service.snapshot();
    let node = snapshot.graph.create_rack(inputs, outputs).unwrap();
    let mut state = snapshot.nodes[1].clone();
    state.id = node;
    state.label = "Rack".into();
    state.gain = 1.;
    state.builtin = BuiltinState::Rack(settings);
    snapshot.nodes.push(state);
    service.replace(snapshot.graph, snapshot.nodes).unwrap();
    service.journal = UndoJournal::default();
    (service, node)
}

fn eq_fixture() -> (ServiceGraph, NodeId) {
    fixture(
        RackSettings {
            slots: vec![
                slot(7, RackProcessor::Eq(EqSettings::default())),
                slot(8, RackProcessor::Gain { initial: 1. }),
            ],
            routes: Vec::new(),
        },
        &[2],
        &[2],
    )
}

fn settings(service: &ServiceGraph, node: NodeId) -> RackSettings {
    service
        .snapshot()
        .node(node)
        .unwrap()
        .builtin
        .rack()
        .unwrap()
        .clone()
}

fn epoch(service: &ServiceGraph, node: NodeId) -> u64 {
    service.snapshot().rack_epochs[&node]
}

fn edit(service: &mut ServiceGraph, node: NodeId, edit: RackEdit) -> Result<bool, String> {
    service.edit(GraphEdit::EditRack {
        node,
        epoch: epoch(service, node),
        edit,
    })
}

fn parameter(slot: u32, parameter: RackParameter, value: f64) -> RackEdit {
    RackEdit::SetParameter {
        slot,
        parameter,
        value,
    }
}

fn assert_rejected(service: &mut ServiceGraph, request: GraphEdit) {
    let before = service.snapshot();
    let history = (service.journal.undo.len(), service.journal.redo.len());
    assert!(service.edit(request).is_err());
    let after = service.snapshot();
    assert_eq!(after.nodes, before.nodes);
    assert_eq!(after.graph, before.graph);
    assert_eq!(after.rack_epochs, before.rack_epochs);
    assert_eq!(after.candidate_revision, before.candidate_revision);
    assert_eq!(
        (service.journal.undo.len(), service.journal.redo.len()),
        history
    );
}

#[test]
fn queued_fields_merge_on_latest_candidate_and_keep_epoch() {
    let (mut service, node) = eq_fixture();
    let epoch = epoch(&service, node);
    for edit in [
        parameter(7, RackParameter::EqFrequency, 2200.),
        parameter(7, RackParameter::EqGain, -6.),
    ] {
        assert!(
            service
                .edit(GraphEdit::EditRack { node, epoch, edit })
                .unwrap()
        );
    }
    let rack = settings(&service, node);
    assert_eq!(
        rack.slots[0].processor,
        RackProcessor::Eq(EqSettings {
            frequency_hz: 2200.,
            gain_db: -6.,
            q: 1.
        })
    );
    assert_eq!(rack.slots[1].processor, RackProcessor::Gain { initial: 1. });
    assert_eq!(service.snapshot().rack_epochs[&node], epoch);
}

#[test]
fn undo_coalesces_only_the_same_slot_and_field() {
    let (mut service, node) = eq_fixture();
    for value in [2., 3., 4.] {
        edit(&mut service, node, parameter(7, RackParameter::EqQ, value)).unwrap();
    }
    edit(&mut service, node, parameter(7, RackParameter::EqGain, -6.)).unwrap();
    edit(&mut service, node, parameter(8, RackParameter::Gain, 0.5)).unwrap();
    assert_eq!(service.journal.undo.len(), 3);
    assert!(service.undo());
    assert_eq!(
        settings(&service, node).slots[1].processor,
        RackProcessor::Gain { initial: 1. }
    );
    assert!(service.undo());
    assert_eq!(
        settings(&service, node).slots[0].processor,
        RackProcessor::Eq(EqSettings {
            q: 4.,
            ..EqSettings::default()
        })
    );
    assert!(service.undo());
    assert_eq!(
        settings(&service, node).slots[0].processor,
        RackProcessor::Eq(EqSettings::default())
    );
    assert!(!service.undo());
}

#[test]
fn removed_slot_reusing_id_and_type_rejects_old_epoch() {
    let (mut service, node) = eq_fixture();
    let old = epoch(&service, node);
    edit(&mut service, node, RackEdit::Remove { slot: 8 }).unwrap();
    let removed = epoch(&service, node);
    edit(
        &mut service,
        node,
        RackEdit::Add {
            route: 0,
            processor: RackProcessor::Gain { initial: 1. },
        },
    )
    .unwrap();
    assert_eq!(settings(&service, node).slots[1].id, 8);
    assert!(epoch(&service, node) > removed && removed > old);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch: old,
            edit: parameter(8, RackParameter::Gain, 0.),
        },
    );
}

#[test]
fn undo_redo_and_reload_never_revive_editor_epochs() {
    let (mut service, node) = eq_fixture();
    let old = epoch(&service, node);
    edit(&mut service, node, parameter(7, RackParameter::EqGain, -6.)).unwrap();
    assert!(service.undo());
    let undone = epoch(&service, node);
    assert!(undone > old);
    assert!(service.redo());
    let redone = epoch(&service, node);
    assert!(redone > undone);
    let snapshot = service.snapshot();
    service.replace(snapshot.graph, snapshot.nodes).unwrap();
    assert!(epoch(&service, node) > redone);
    for stale in [old, undone, redone] {
        assert_rejected(
            &mut service,
            GraphEdit::EditRack {
                node,
                epoch: stale,
                edit: parameter(7, RackParameter::EqGain, 1.),
            },
        );
    }
}

#[test]
fn removed_topology_and_reloaded_same_node_id_reject_stale_commands() {
    let (mut service, node) = eq_fixture();
    let snapshot = service.snapshot();
    let request = GraphEdit::EditRack {
        node,
        epoch: epoch(&service, node),
        edit: RackEdit::Clear,
    };
    service.edit(GraphEdit::RemoveNode(node)).unwrap();
    assert!(!service.snapshot().rack_epochs.contains_key(&node));
    assert_rejected(&mut service, request.clone());
    service.replace(snapshot.graph, snapshot.nodes).unwrap();
    assert_rejected(&mut service, request);
}

#[test]
fn whole_rack_replacement_and_clear_refresh_identity_without_coalescing() {
    let (mut service, node) = eq_fixture();
    let original = settings(&service, node);
    let mut last = epoch(&service, node);
    for state in [BuiltinState::Rack(original), BuiltinState::None] {
        service.edit(GraphEdit::SetBuiltin { node, state }).unwrap();
        assert!(epoch(&service, node) > last);
        last = epoch(&service, node);
    }
    edit(&mut service, node, RackEdit::Clear).unwrap();
    assert!(epoch(&service, node) > last);
    assert_eq!(service.journal.undo.len(), 3);
}

#[test]
fn invalid_values_wrong_processor_and_missing_slots_are_atomic() {
    let (mut service, node) = eq_fixture();
    for edit in [
        parameter(7, RackParameter::EqQ, 0.),
        parameter(7, RackParameter::EqFrequency, f64::NAN),
        parameter(7, RackParameter::EqGain, f64::INFINITY),
        parameter(7, RackParameter::Gain, 0.5),
        parameter(99, RackParameter::EqGain, 0.),
        RackEdit::SetMono {
            slot: 7,
            enabled: true,
        },
        RackEdit::SetSwap {
            slot: 7,
            enabled: true,
        },
        RackEdit::Move {
            slot: 7,
            direction: 0,
        },
        RackEdit::Add {
            route: 99,
            processor: RackProcessor::Gain { initial: 1. },
        },
    ] {
        let epoch = epoch(&service, node);
        assert_rejected(&mut service, GraphEdit::EditRack { node, epoch, edit });
    }
}

#[test]
fn route_local_reorder_and_bypass_preserve_ids_epoch_and_independent_undo() {
    let (mut service, node) = fixture(
        RackSettings {
            slots: vec![
                slot(1, RackProcessor::Gain { initial: 1. }),
                slot(2, RackProcessor::Gain { initial: 2. }),
                slot(3, RackProcessor::Gain { initial: 3. }),
            ],
            routes: vec![
                RackRoute {
                    input_port: 0,
                    output_port: 0,
                    slots: vec![1, 3],
                },
                RackRoute {
                    input_port: 1,
                    output_port: 1,
                    slots: vec![2],
                },
            ],
        },
        &[2, 1],
        &[2, 1],
    );
    let old = epoch(&service, node);
    edit(
        &mut service,
        node,
        RackEdit::Move {
            slot: 1,
            direction: 1,
        },
    )
    .unwrap();
    edit(
        &mut service,
        node,
        RackEdit::Bypass {
            slot: 1,
            bypass: true,
        },
    )
    .unwrap();
    let rack = settings(&service, node);
    assert_eq!(
        rack.slots.iter().map(|slot| slot.id).collect::<Vec<_>>(),
        [3, 2, 1]
    );
    assert!(rack.slots[2].bypass);
    assert_eq!(epoch(&service, node), old);
    assert_eq!(service.journal.undo.len(), 2);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch: old,
            edit: RackEdit::Move {
                slot: 2,
                direction: 1,
            },
        },
    );
}

#[test]
fn mapper_signed_weight_preserves_other_cells_and_actual_route_widths() {
    let (mut service, node) = fixture(
        RackSettings {
            slots: vec![slot(
                3,
                RackProcessor::ChannelMapper(ChannelMapperSettings {
                    mapping: vec![2, 0],
                    matrix: Vec::new(),
                }),
            )],
            routes: vec![RackRoute {
                input_port: 1,
                output_port: 1,
                slots: vec![3],
            }],
        },
        &[1, 3],
        &[1, 2],
    );
    edit(
        &mut service,
        node,
        RackEdit::SetMatrixWeight {
            slot: 3,
            input: 1,
            output: 0,
            value: -1.5,
        },
    )
    .unwrap();
    edit(
        &mut service,
        node,
        RackEdit::SetMatrixWeight {
            slot: 3,
            input: 2,
            output: 1,
            value: 0.25,
        },
    )
    .unwrap();
    let RackProcessor::ChannelMapper(mapper) = &settings(&service, node).slots[0].processor else {
        panic!("mapper")
    };
    assert!(mapper.mapping.is_empty());
    assert_eq!(mapper.matrix, vec![vec![0., -1.5, 1.], vec![1., 0., 0.25]]);
    assert_eq!(service.journal.undo.len(), 2);
    for edit in [
        RackEdit::SetMatrixWeight {
            slot: 3,
            input: 3,
            output: 0,
            value: 1.,
        },
        RackEdit::SetMatrixWeight {
            slot: 3,
            input: 0,
            output: 2,
            value: 1.,
        },
        RackEdit::SetMatrixWeight {
            slot: 3,
            input: 0,
            output: 0,
            value: f64::NEG_INFINITY,
        },
        RackEdit::Bypass {
            slot: 3,
            bypass: true,
        },
        RackEdit::Remove { slot: 3 },
        RackEdit::Clear,
    ] {
        let epoch = epoch(&service, node);
        assert_rejected(&mut service, GraphEdit::EditRack { node, epoch, edit });
    }
}

#[test]
fn bypassed_mapper_edits_use_configured_output_width_and_pan_uses_actual_input() {
    let mut mapper = slot(
        0,
        RackProcessor::ChannelMapper(ChannelMapperSettings {
            mapping: vec![0],
            matrix: Vec::new(),
        }),
    );
    mapper.bypass = true;
    let (mut service, node) = fixture(
        RackSettings {
            slots: vec![
                mapper,
                slot(
                    1,
                    RackProcessor::Pan {
                        initial: 0.,
                        mono: false,
                        swap: false,
                    },
                ),
            ],
            routes: Vec::new(),
        },
        &[2],
        &[2],
    );
    edit(
        &mut service,
        node,
        RackEdit::SetMatrixWeight {
            slot: 0,
            input: 1,
            output: 0,
            value: -1.,
        },
    )
    .unwrap();
    edit(
        &mut service,
        node,
        RackEdit::SetMono {
            slot: 1,
            enabled: true,
        },
    )
    .unwrap();
    edit(
        &mut service,
        node,
        RackEdit::SetSwap {
            slot: 1,
            enabled: true,
        },
    )
    .unwrap();
    assert_eq!(
        settings(&service, node).slots[1].processor,
        RackProcessor::Pan {
            initial: 0.,
            mono: true,
            swap: true
        }
    );
    let epoch = epoch(&service, node);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch,
            edit: RackEdit::Bypass {
                slot: 0,
                bypass: false,
            },
        },
    );
}

#[test]
fn mapper_presets_preserve_width_and_reject_mono_swap() {
    let (mut service, node) = fixture(
        RackSettings {
            slots: vec![slot(
                0,
                RackProcessor::ChannelMapper(ChannelMapperSettings::default()),
            )],
            routes: Vec::new(),
        },
        &[2],
        &[2],
    );
    edit(
        &mut service,
        node,
        RackEdit::MapperPreset {
            slot: 0,
            swap: true,
        },
    )
    .unwrap();
    assert_eq!(
        settings(&service, node).slots[0].processor,
        RackProcessor::ChannelMapper(ChannelMapperSettings {
            mapping: vec![1, 0],
            matrix: Vec::new()
        })
    );
    edit(
        &mut service,
        node,
        RackEdit::MapperPreset {
            slot: 0,
            swap: false,
        },
    )
    .unwrap();
    assert_eq!(
        settings(&service, node).slots[0].processor,
        RackProcessor::ChannelMapper(ChannelMapperSettings::identity(2))
    );
    let (mut service, node) = fixture(
        RackSettings {
            slots: vec![slot(
                0,
                RackProcessor::ChannelMapper(ChannelMapperSettings::default()),
            )],
            routes: Vec::new(),
        },
        &[1],
        &[1],
    );
    let epoch = epoch(&service, node);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch,
            edit: RackEdit::MapperPreset {
                slot: 0,
                swap: true,
            },
        },
    );
}

#[test]
fn mapper_swap_first_two_preserves_remaining_channels_in_preview_and_service() {
    let original = RackSettings {
        slots: vec![slot(
            0,
            RackProcessor::ChannelMapper(ChannelMapperSettings::default()),
        )],
        routes: Vec::new(),
    };
    let request = RackEdit::MapperPreset {
        slot: 0,
        swap: true,
    };
    let preview = request.preview(&original, &[4], &[4]).unwrap();
    assert_eq!(
        original.slots[0].processor,
        RackProcessor::ChannelMapper(ChannelMapperSettings::default())
    );
    assert_eq!(
        preview.slots[0].processor,
        RackProcessor::ChannelMapper(ChannelMapperSettings {
            mapping: vec![1, 0, 2, 3],
            matrix: Vec::new(),
        })
    );
    let (mut service, node) = fixture(original, &[4], &[4]);
    let original_epoch = epoch(&service, node);
    edit(&mut service, node, request).unwrap();
    assert_eq!(settings(&service, node), preview);
    assert_eq!(epoch(&service, node), original_epoch);
}

#[test]
fn every_scalar_parameter_changes_only_its_matching_engine_field() {
    use moiren_core::protocol::{ParamValue, ProcessorId};
    use moiren_engine::processor::{Delay, Eq, Gain, Gate, Limiter, Pan};

    let mut cases = vec![
        (
            RackProcessor::Gain { initial: 1. },
            RackParameter::Gain,
            Gain::LEVEL,
            0.25,
        ),
        (
            RackProcessor::Pan {
                initial: 0.,
                mono: true,
                swap: true,
            },
            RackParameter::Pan,
            Pan::POSITION,
            -0.5,
        ),
        (
            RackProcessor::Eq(Default::default()),
            RackParameter::EqFrequency,
            Eq::FREQUENCY,
            2500.,
        ),
        (
            RackProcessor::Eq(Default::default()),
            RackParameter::EqGain,
            Eq::GAIN,
            -7.,
        ),
        (
            RackProcessor::Eq(Default::default()),
            RackParameter::EqQ,
            Eq::Q,
            2.,
        ),
        (
            RackProcessor::Gate(Default::default()),
            RackParameter::GateThreshold,
            Gate::THRESHOLD,
            -32.,
        ),
        (
            RackProcessor::Gate(Default::default()),
            RackParameter::GateRange,
            Gate::RANGE,
            -45.,
        ),
        (
            RackProcessor::Gate(Default::default()),
            RackParameter::GateAttack,
            Gate::ATTACK,
            8.,
        ),
        (
            RackProcessor::Gate(Default::default()),
            RackParameter::GateHold,
            Gate::HOLD,
            22.,
        ),
        (
            RackProcessor::Gate(Default::default()),
            RackParameter::GateRelease,
            Gate::RELEASE,
            180.,
        ),
        (
            RackProcessor::Limiter(Default::default()),
            RackParameter::LimitInput,
            Limiter::INPUT_GAIN,
            3.,
        ),
        (
            RackProcessor::Limiter(Default::default()),
            RackParameter::LimitThreshold,
            Limiter::THRESHOLD,
            -3.,
        ),
        (
            RackProcessor::Limiter(Default::default()),
            RackParameter::LimitRelease,
            Limiter::RELEASE,
            110.,
        ),
        (
            RackProcessor::Delay(Default::default()),
            RackParameter::DelayTime,
            Delay::DELAY,
            600.,
        ),
        (
            RackProcessor::Delay(Default::default()),
            RackParameter::DelayFeedback,
            Delay::FEEDBACK,
            0.4,
        ),
        (
            RackProcessor::Delay(Default::default()),
            RackParameter::DelayMix,
            Delay::MIX,
            0.25,
        ),
    ];
    cases.extend(CompressorParameter::ALL.into_iter().map(|parameter| {
        let (min, max) = parameter.range();
        (
            RackProcessor::Compressor(Default::default()),
            RackParameter::Compressor(parameter),
            parameter.id(),
            (min + max) / 2.,
        )
    }));
    for (processor, parameter, id, value) in cases {
        let original = RackSettings {
            slots: vec![slot(1, processor)],
            routes: Vec::new(),
        };
        let before = original.slots[0]
            .processor
            .checked_parameters(2, ProcessorId(0))
            .unwrap();
        let edit = RackEdit::SetParameter {
            slot: 1,
            parameter,
            value,
        };
        let after = edit.preview(&original, &[2], &[2]).unwrap();
        let after = after.slots[0]
            .processor
            .checked_parameters(2, ProcessorId(0))
            .unwrap();
        for (before, after) in before.iter().zip(after) {
            assert_eq!(after.key, before.key);
            assert_eq!(
                after.initial,
                if before.key.parameter == id {
                    ParamValue::Float(value)
                } else {
                    before.initial
                },
                "{parameter:?}"
            );
        }
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -f64::MAX,
            f64::MAX,
        ] {
            assert!(
                RackEdit::SetParameter {
                    slot: 1,
                    parameter,
                    value: invalid
                }
                .preview(&original, &[2], &[2])
                .is_err(),
                "{parameter:?} accepted {invalid}"
            );
        }
    }
}

#[test]
fn preview_failures_do_not_change_settings_and_enforce_stage_limit() {
    let full = RackSettings {
        slots: (0..RackSettings::MAX_SLOTS as u32)
            .map(|id| slot(id, RackProcessor::Gain { initial: 1. }))
            .collect(),
        routes: Vec::new(),
    };
    let before = full.clone();
    assert!(
        RackEdit::Add {
            route: 0,
            processor: RackProcessor::Gain { initial: 1. }
        }
        .preview(&full, &[2], &[2])
        .is_err()
    );
    assert_eq!(full, before);
    let (mut service, node) = fixture(full, &[2], &[2]);
    let epoch = epoch(&service, node);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch,
            edit: RackEdit::Add {
                route: 0,
                processor: RackProcessor::Gain { initial: 1. },
            },
        },
    );
}

#[test]
fn adding_rack_publishes_identity_and_non_rack_topology_rejects_forged_epoch() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Rack,
            channels: 2,
        })
        .unwrap();
    let snapshot = service.snapshot();
    let rack = snapshot.node_of_kind(NodeKind::Rack).unwrap().0.id();
    assert_eq!(snapshot.rack_epochs[&rack], snapshot.candidate_revision);
    let gain = snapshot.node_of_kind(NodeKind::Gain).unwrap().0.id();
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node: gain,
            epoch: snapshot.rack_epochs[&rack],
            edit: RackEdit::Clear,
        },
    );
}

#[test]
fn clear_preserves_explicit_multiroute_ports_order_and_empty_routes() {
    let original = RackSettings {
        slots: vec![
            slot(7, RackProcessor::Gain { initial: 0.5 }),
            slot(11, RackProcessor::Gain { initial: 0.25 }),
            slot(9, RackProcessor::Eq(EqSettings::default())),
        ],
        routes: vec![
            RackRoute {
                input_port: 1,
                output_port: 0,
                slots: vec![11],
            },
            RackRoute {
                input_port: 0,
                output_port: 1,
                slots: vec![9, 7],
            },
            RackRoute {
                input_port: 2,
                output_port: 2,
                slots: Vec::new(),
            },
        ],
    };
    let inputs = [2, 1, 4];
    let outputs = [1, 2, 4];
    let preview = RackEdit::Clear
        .preview(&original, &inputs, &outputs)
        .unwrap();
    let expected = RackSettings {
        slots: Vec::new(),
        routes: original
            .routes
            .iter()
            .map(|route| RackRoute {
                slots: Vec::new(),
                ..route.clone()
            })
            .collect(),
    };
    assert_eq!(preview, expected);
    assert_eq!(original.slots.len(), 3, "preview never clears its input");
    assert_eq!(
        preview.resolve_layout(&inputs, &outputs).unwrap().routes,
        expected.routes
    );

    let (mut service, node) = fixture(original.clone(), &inputs, &outputs);
    let before = service.snapshot();
    edit(&mut service, node, RackEdit::Clear).unwrap();
    assert_eq!(settings(&service, node), expected);
    assert_eq!(service.snapshot().graph, before.graph);
    assert!(epoch(&service, node) > before.rack_epochs[&node]);
    assert_eq!(service.journal.undo.len(), 1);
    assert!(service.undo());
    assert_eq!(settings(&service, node), original);
    assert!(service.redo());
    assert_eq!(settings(&service, node), expected);
}

#[test]
fn rack_epochs_are_node_scoped_and_load_undo_redo_use_the_live_monotonic_source() {
    let (mut service, first) = eq_fixture();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Rack,
            channels: 2,
        })
        .unwrap();
    let second = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Rack && node.id() != first)
        .unwrap()
        .id();
    let saved = service.snapshot();
    service.replace(saved.graph, saved.nodes).unwrap();
    let saved = service.snapshot();
    // One load may give multiple nodes the same generation; NodeId is part
    // of every command's identity, so epochs need not be unique across nodes.
    let generation = epoch(&service, first);
    assert_eq!(generation, epoch(&service, second));
    edit(
        &mut service,
        first,
        parameter(7, RackParameter::EqGain, -6.),
    )
    .unwrap();
    assert_eq!(service.snapshot().rack_epochs, saved.rack_epochs);
    edit(
        &mut service,
        second,
        RackEdit::Add {
            route: 0,
            processor: RackProcessor::Gain { initial: 1. },
        },
    )
    .unwrap();
    let mut newest = epoch(&service, second);
    assert!(newest > generation);
    assert_eq!(epoch(&service, first), generation);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node: second,
            epoch: generation,
            edit: RackEdit::Clear,
        },
    );
    assert_eq!(epoch(&service, second), newest);

    service.replace(saved.graph, saved.nodes).unwrap();
    for step in 0..3 {
        if step == 1 {
            assert!(service.undo());
        }
        if step == 2 {
            assert!(service.redo());
        }
        let snapshot = service.snapshot();
        assert_eq!(snapshot.rack_epochs.len(), 2);
        for node in [first, second] {
            assert_eq!(snapshot.rack_epochs[&node], snapshot.candidate_revision);
            assert!(snapshot.rack_epochs[&node] > newest);
        }
        newest = snapshot.candidate_revision;
    }
}

#[test]
fn exhausted_revision_rejects_rack_edit_and_load_without_wrapping_epoch_or_history() {
    let (mut service, node) = eq_fixture();
    service.snapshot.candidate_revision = u64::MAX;
    let before = service.snapshot();
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch: before.rack_epochs[&node],
            edit: RackEdit::Clear,
        },
    );
    assert!(
        service
            .replace(before.graph.clone(), before.nodes.clone())
            .is_err()
    );
    assert_eq!(service.snapshot(), before);
    assert!(service.journal.undo.is_empty());
    assert!(service.journal.redo.is_empty());
}
