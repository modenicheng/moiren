use super::support::*;
use moiren_core::{
    graph::{NodeId, NodeKind, SendParams},
    protocol::{ParamValue, ParameterId, ProcessorId},
};
use moiren_engine::{
    host::*,
    processor::{
        ChannelMapperSettings, CompressorSettings, Delay, DelaySettings, EqSettings, Gain,
        GateSettings, LimiterSettings, Pan,
    },
    rack::{RackProcessor, RackSettings, RackSlot},
};
use std::sync::atomic::Ordering;

fn gain_slot(id: u32, initial: f64) -> RackSlot {
    RackSlot {
        id,
        bypass: false,
        processor: RackProcessor::Gain { initial },
    }
}

fn rack_config(project: &mut EngineProject, node: NodeId) -> &mut RackSettings {
    let NodeConfig::Rack(settings) = project.nodes.get_mut(&node).unwrap() else {
        panic!("expected rack configuration");
    };
    settings
}

fn slot_target(node: NodeId, slot: u32, parameter: ParameterId) -> ParameterTarget {
    ParameterTarget::RackSlot {
        node,
        slot,
        parameter,
    }
}

fn rack_project() -> (EngineProject, NodeId) {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    let node = project.graph.create_node(NodeKind::Rack, 1).unwrap();
    project.nodes.insert(
        node,
        NodeConfig::Rack(RackSettings {
            slots: vec![gain_slot(90, 2.0), gain_slot(12, 0.5)],

            ..Default::default()
        }),
    );
    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
    let rack_input = project.graph.get_node(node).unwrap().inputs()[0].id();
    let rack_output = project.graph.get_node(node).unwrap().outputs()[0].id();
    project
        .graph
        .connect(source, rack_input, SendParams::default())
        .unwrap();
    project
        .graph
        .connect(rack_output, destination, SendParams::default())
        .unwrap();
    (project, node)
}

#[test]
fn rack_parameter_defaults_include_active_slots_only() {
    let mut project = EngineProject::default();
    let node = project.graph.create_node(NodeKind::Rack, 1).unwrap();
    project.nodes.insert(
        node,
        NodeConfig::Rack(RackSettings {
            slots: vec![
                RackSlot {
                    id: 90,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 2.0 },
                },
                RackSlot {
                    id: 12,
                    bypass: true,
                    processor: RackProcessor::Gain { initial: 0.5 },
                },
            ],

            ..Default::default()
        }),
    );
    project.validate().unwrap();
    assert_eq!(
        project.parameter_values(),
        vec![DesiredParameter {
            target: slot_target(node, 90, Gain::LEVEL),
            value: ParamValue::Float(2.0),
        }]
    );
}

#[test]
fn rack_parameter_values_use_builtin_schemas_and_desired_overrides() {
    let mut project = EngineProject::default();
    let node = project.graph.create_node(NodeKind::Rack, 2).unwrap();
    let processors = [
        RackProcessor::Gain { initial: 2.0 },
        RackProcessor::Pan {
            initial: -0.25,
            mono: true,
            swap: false,
        },
        RackProcessor::Compressor(CompressorSettings::default()),
        RackProcessor::Eq(EqSettings::default()),
        RackProcessor::Gate(GateSettings::default()),
        RackProcessor::Limiter(LimiterSettings::default()),
        RackProcessor::Delay(DelaySettings::default()),
        RackProcessor::ChannelMapper(ChannelMapperSettings::default()),
    ];
    let mut expected = Vec::new();
    let slots = processors
        .into_iter()
        .enumerate()
        .map(|(index, processor)| {
            let id = 100 + index as u32;
            for spec in processor.checked_parameters(2, ProcessorId(999)).unwrap() {
                expected.push(DesiredParameter {
                    target: slot_target(node, id, spec.key.parameter),
                    value: spec.initial,
                });
            }
            RackSlot {
                id,
                bypass: false,
                processor,
            }
        })
        .collect();
    project.nodes.insert(
        node,
        NodeConfig::Rack(RackSettings {
            slots,
            ..Default::default()
        }),
    );
    project.validate().unwrap();
    assert_eq!(project.parameter_values(), expected);
    assert!(expected.contains(&DesiredParameter {
        target: slot_target(node, 101, Pan::MONO),
        value: ParamValue::Bool(true),
    }));
    project.parameters.push(DesiredParameter {
        target: slot_target(node, 100, Gain::LEVEL),
        value: ParamValue::Float(0.25),
    });
    expected[0].value = ParamValue::Float(0.25);
    project.validate().unwrap();
    assert_eq!(project.parameter_values(), expected);
    rack_config(&mut project, node).slots.reverse();
    assert!(project.parameter_values().contains(&expected[0]));
}

#[test]
fn rack_parameter_validation_rejects_invalid_slot_schema_or_value() {
    let (mut project, node) = rack_project();
    let settings = rack_config(&mut project, node);
    settings.slots[1].bypass = true;
    settings.slots.push(RackSlot {
        id: 33,
        bypass: false,
        processor: RackProcessor::Delay(DelaySettings {
            delay_ms: 10.0,
            max_delay_ms: 25.0,
            ..Default::default()
        }),
    });
    let target = slot_target(node, 90, Gain::LEVEL);
    assert!(project.target_exists(target));
    assert!(
        project
            .validate_parameter(target, ParamValue::Float(0.5))
            .is_ok()
    );
    for value in [
        ParamValue::Float(-0.1),
        ParamValue::Float(17.0),
        ParamValue::Float(f64::NAN),
        ParamValue::Float(f64::INFINITY),
        ParamValue::Bool(true),
    ] {
        assert_eq!(
            project.validate_parameter(target, value),
            Err(ProjectError::InvalidParameter)
        );
    }
    for invalid in [
        slot_target(node, 12, Gain::LEVEL),
        slot_target(node, 999, Gain::LEVEL),
        slot_target(node, 90, ParameterId(999)),
        ParameterTarget::Node {
            node,
            parameter: Gain::LEVEL,
        },
    ] {
        assert_eq!(
            project.validate_parameter(invalid, ParamValue::Float(0.5)),
            Err(ProjectError::InvalidParameter)
        );
    }
    assert!(!project.target_exists(slot_target(node, 12, Gain::LEVEL)));
    assert!(!project.target_exists(slot_target(node, 999, Gain::LEVEL)));
    let delay = slot_target(node, 33, Delay::DELAY);
    assert!(
        project
            .validate_parameter(delay, ParamValue::Float(25.0))
            .is_ok()
    );
    assert_eq!(
        project.validate_parameter(delay, ParamValue::Float(25.1)),
        Err(ProjectError::InvalidParameter)
    );
}

#[test]
fn rack_slot_parameters_apply_while_running_without_recompilation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, node) = rack_project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    wait_sample(&state, 1.0);
    let target = slot_target(node, 90, Gain::LEVEL);
    let receipt = host
        .control()
        .parameter(
            target,
            ParamValue::Float(0.5),
            ParameterTiming::NextBlock,
            8,
        )
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        ParameterOutcome::Applied(_)
    ));
    wait_sample(&state, 0.25);
    let second = host
        .control()
        .parameter(
            slot_target(node, 12, Gain::LEVEL),
            ParamValue::Float(0.8),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    assert!(matches!(second.wait(TIMEOUT), ParameterOutcome::Applied(_)));
    wait_sample(&state, 0.4);
    assert!(
        host.observer()
            .project()
            .parameter_values()
            .contains(&DesiredParameter {
                target,
                value: ParamValue::Float(0.5),
            })
    );
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(host.observer().snapshot().desired_revision, 1);
}

#[test]
fn rack_host_rejects_bypassed_slot_parameter() {
    let (mut project, node) = rack_project();
    rack_config(&mut project, node).slots[0].bypass = true;
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let receipt = host
        .control()
        .parameter(
            slot_target(node, 90, Gain::LEVEL),
            ParamValue::Float(0.25),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    assert_eq!(
        receipt.wait(TIMEOUT),
        ParameterOutcome::Rejected(ParameterFailure::InvalidValue)
    );
    assert!(host.observer().project().parameters.is_empty());
}

#[test]
fn rack_project_edits_prune_intent_for_bypassed_or_deleted_slots_and_nodes() {
    for change in 0..3 {
        let (mut project, node) = rack_project();
        let target = slot_target(node, 90, Gain::LEVEL);
        let retained = DesiredParameter {
            target: slot_target(node, 12, Gain::LEVEL),
            value: ParamValue::Float(0.7),
        };
        project.parameters.extend([
            DesiredParameter {
                target,
                value: ParamValue::Float(0.25),
            },
            retained,
        ]);
        project.validate().unwrap();
        project
            .edit(|candidate| {
                match change {
                    0 => rack_config(candidate, node).slots[0].bypass = true,
                    1 => {
                        rack_config(candidate, node).slots.remove(0);
                    }
                    _ => {
                        candidate.graph.remove_node(node)?;
                    }
                }
                Ok(())
            })
            .unwrap();
        assert!(!project.target_exists(target));
        assert!(
            !project
                .parameters
                .iter()
                .any(|value| value.target == target)
        );
        if change != 2 {
            assert_eq!(project.parameters, vec![retained]);
        } else {
            assert!(project.parameters.is_empty());
        }
    }
}

#[test]
fn rack_project_edit_rejects_new_invalid_slot_intent() {
    let (mut project, node) = rack_project();
    let original = project.clone();
    let result = project.edit(|candidate| {
        candidate.parameters.push(DesiredParameter {
            target: slot_target(node, 999, Gain::LEVEL),
            value: ParamValue::Float(0.25),
        });
        Ok(())
    });
    assert_eq!(result, Err(ProjectError::InvalidParameter));
    assert_eq!(project, original);
}

#[test]
fn rack_pending_slot_target_is_pruned_when_deleted_or_bypassed_before_activation() {
    for bypass in [false, true] {
        let backend = FakeBackend::default();
        let state = backend.0.clone();
        let (project, node) = rack_project();
        let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
        wait(&host.observer(), |snapshot| {
            snapshot.execution == ExecutionState::Running
        });
        state.block_update.store(true, Ordering::SeqCst);
        let _addition = host
            .control()
            .edit(move |project| {
                rack_config(project, node).slots.push(gain_slot(77, 1.0));
                Ok(())
            })
            .unwrap();
        wait_count(&state.prepares, 2);
        let target = slot_target(node, 77, Gain::LEVEL);
        let parameter = host
            .control()
            .parameter(
                target,
                ParamValue::Float(0.25),
                ParameterTiming::NextBlock,
                0,
            )
            .unwrap();
        assert_eq!(
            wait_parameter_consumed(&parameter),
            ParameterOutcome::Accepted
        );
        let removal = host
            .control()
            .edit(move |project| {
                let settings = rack_config(project, node);
                if bypass {
                    settings.slots.last_mut().unwrap().bypass = true;
                } else {
                    settings.slots.pop();
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            parameter.wait(TIMEOUT),
            ParameterOutcome::Rejected(ParameterFailure::TargetRemoved)
        );
        state.block_update.store(false, Ordering::SeqCst);
        assert!(matches!(
            removal.wait(TIMEOUT),
            EditOutcome::Activated { .. }
        ));
        let observed = host.observer().project();
        assert!(!observed.target_exists(target));
        assert!(
            !observed
                .parameters
                .iter()
                .any(|value| value.target == target)
        );
    }
}

#[test]
fn rack_slot_saved_parameter_is_prepared_before_initial_audio() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (mut project, node) = rack_project();
    project.parameters.push(DesiredParameter {
        target: slot_target(node, 90, Gain::LEVEL),
        value: ParamValue::Float(0.25),
    });
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    wait_sample(&state, 0.125);
}

#[test]
fn rack_slot_live_parameter_survives_structural_repreparation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, node) = rack_project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let receipt = host
        .control()
        .parameter(
            slot_target(node, 90, Gain::LEVEL),
            ParamValue::Float(0.25),
            ParameterTiming::NextBlock,
            0,
        )
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        ParameterOutcome::Applied(_)
    ));
    wait_sample(&state, 0.125);
    let edit = host
        .control()
        .edit(move |project| {
            rack_config(project, node).slots.push(RackSlot {
                id: 999,
                bypass: true,
                processor: RackProcessor::Gain { initial: 1.0 },
            });
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    wait_sample(&state, 0.125);
}

#[test]
fn structural_edit_applies_changed_desired_value_to_reused_slot() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, node) = rack_project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    wait_sample(&state, 1.0);
    let edit = host
        .control()
        .edit(move |project| {
            rack_config(project, node).slots.push(gain_slot(999, 1.0));
            project.parameters.push(DesiredParameter {
                target: slot_target(node, 90, Gain::LEVEL),
                value: ParamValue::Float(0.25),
            });
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    wait_sample(&state, 0.125);
}

#[test]
fn superseded_structure_keeps_parameter_override_until_latest_plan_activates() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, node) = rack_project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    state.block_update.store(true, Ordering::SeqCst);
    state.ignore_update_cancel.store(true, Ordering::SeqCst);
    let first = host
        .control()
        .edit(move |project| {
            rack_config(project, node).slots.push(gain_slot(999, 1.0));
            project.parameters.push(DesiredParameter {
                target: slot_target(node, 90, Gain::LEVEL),
                value: ParamValue::Float(0.25),
            });
            Ok(())
        })
        .unwrap();
    wait_count(&state.prepares, 2);
    let latest = host
        .control()
        .edit(move |project| {
            rack_config(project, node).slots.push(gain_slot(998, 1.0));
            Ok(())
        })
        .unwrap();
    wait(&host.observer(), |snapshot| snapshot.desired_revision == 3);
    assert!(matches!(first.outcome(), EditOutcome::Superseded { .. }));
    state.block_update.store(false, Ordering::SeqCst);
    assert!(matches!(
        latest.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 3,
            ..
        }
    ));
    wait_sample(&state, 0.125);
}

#[test]
fn rack_slot_schema_uses_post_mapper_input_width_not_node_width() {
    let mut project = EngineProject::default();
    let node = project.graph.create_rack(&[1], &[1]).unwrap();
    let slots = vec![
        RackSlot {
            id: 1,
            bypass: false,
            processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                mapping: vec![0, 0],
                ..Default::default()
            }),
        },
        RackSlot {
            id: 2,
            bypass: false,
            processor: RackProcessor::Pan {
                initial: 0.25,
                mono: false,
                swap: true,
            },
        },
        RackSlot {
            id: 3,
            bypass: false,
            processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                matrix: vec![vec![0.5, 0.5]],
                ..Default::default()
            }),
        },
    ];
    project.nodes.insert(
        node,
        NodeConfig::Rack(RackSettings {
            slots,
            ..Default::default()
        }),
    );
    project.validate().unwrap();
    let target = slot_target(node, 2, Pan::POSITION);
    project
        .validate_parameter(target, ParamValue::Float(-0.5))
        .unwrap();
    assert!(project.parameter_values().contains(&DesiredParameter {
        target,
        value: ParamValue::Float(0.25)
    }));
    assert!(project.parameter_values().contains(&DesiredParameter {
        target: slot_target(node, 2, Pan::SWAP),
        value: ParamValue::Bool(true)
    }));
}
