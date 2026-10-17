use super::support::*;
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::{
    host::*,
    processor::DelaySettings,
    rack::{RackProcessor, RackSettings, RackSlot},
};
use std::sync::atomic::Ordering;

#[test]
fn unrelated_slot_edit_preserves_running_delay_history() {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    project.processing.engine.processing_sr = 1_000.0;
    let rack = project.graph.create_node(NodeKind::Rack, 1).unwrap();
    project.nodes.insert(
        rack,
        NodeConfig::Rack(RackSettings {
            slots: vec![RackSlot {
                id: 42,
                bypass: false,
                processor: RackProcessor::Delay(DelaySettings {
                    delay_ms: 1_000.0,
                    max_delay_ms: 1_000.0,
                    ..Default::default()
                }),
            }],
            ..Default::default()
        }),
    );
    for (source, destination) in [(input, rack), (rack, output)] {
        project
            .graph
            .connect(
                project.graph.get_node(source).unwrap().outputs()[0].id(),
                project.graph.get_node(destination).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    wait_sample(&state, 1.0);
    let receipt = host
        .control()
        .edit(move |project| {
            let NodeConfig::Rack(settings) = project.nodes.get_mut(&rack).unwrap() else {
                unreachable!();
            };
            settings.slots.insert(
                0,
                RackSlot {
                    id: 99,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 1.0 },
                },
            );
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        receipt.wait(TIMEOUT),
        EditOutcome::Activated { .. }
    ));
    assert_eq!(
        f64::from_bits(state.last_sample.load(Ordering::SeqCst)),
        1.0
    );
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
}
