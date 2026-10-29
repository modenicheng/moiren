use super::*;
use crate::{
    latency::{
        InputDeliveryRecorder,
        events::event_history,
        transport::{DeliveryTimingSource, TransportInputPolicy},
    },
    process_loopback::{MeasuredProcessDelivery, ProcessIdentity},
};
use moiren_engine::{
    host::EngineProject,
    node::{InputConfig, InputSource},
};

fn evidence() -> (ProcessIdentity, MeasuredProcessDelivery) {
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    };
    let (mut producer, producer_observer) = event_history();
    let (mut consumer, consumer_observer) = event_history();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for stamp in [1_000_000, 1_020_000, 1_040_000] {
        delivery.packet(96, Some(stamp - 1));
        delivery.callback_burst(96);
        producer.record(96, Some(stamp));
        consumer.record(96, Some(stamp + 1000));
    }
    let measured = MeasuredProcessDelivery::from_observations(
        process.clone(),
        7,
        48_000,
        48_000,
        96,
        delivery.snapshot(),
        producer_observer.snapshot().unwrap(),
        consumer_observer.snapshot().unwrap(),
    )
    .unwrap();
    (process, measured)
}

fn target() -> TransportInputPolicy {
    TransportInputPolicy::Target {
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    }
}

#[test]
fn measured_backend_revalidates_actual_whole_demand_without_falling_back() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let (process, measured) = evidence();
    let backend = ProfessionalBackend::<f32>::new()
        .unwrap()
        .with_measured_process_bridge(node, &process, 7, measured, target())
        .unwrap();
    let admitted = backend
        .process_bridge_for(node, (42, 123), 48_000, 96)
        .unwrap();
    assert_eq!(admitted.input, target());
    assert_eq!(
        admitted.delivery.unwrap().source,
        DeliveryTimingSource::Empirical
    );
    assert!(
        backend
            .process_bridge_for(node, (42, 123), 48_000, 1024)
            .is_err()
    );
    assert!(
        backend
            .process_bridge_for(node, (42, 124), 48_000, 96)
            .is_err()
    );
    // A different source keeps the compatibility default, and does not borrow
    // this node's observations even if the request has the same frame shape.
    let other = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let compatible = backend
        .process_bridge_for(other, (43, 456), 48_000, 96)
        .unwrap();
    assert_eq!(compatible.input, TransportInputPolicy::Compatible);
    assert!(compatible.delivery.is_none());
}

#[test]
fn measured_backend_refuses_stale_generation_and_removed_or_replaced_sources() {
    let mut project = EngineProject::default();
    let node = project
        .graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let (process, measured) = evidence();
    assert!(
        ProfessionalBackend::<f64>::new()
            .unwrap()
            .with_measured_process_bridge(node, &process, 8, measured.clone(), target())
            .is_err()
    );
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_measured_process_bridge(node, &process, 7, measured, target())
        .unwrap();
    assert!(backend.validate_measured_sources(&project).is_err());
    project.nodes.insert(
        node,
        NodeConfig::Input(InputConfig {
            source: InputSource::Application {
                executable: "moiren:process:42:123".into(),
                intent: CaptureIntent::PassiveTap,
            },
            channels: 2,
            device: Default::default(),
        }),
    );
    backend.validate_measured_sources(&project).unwrap();
    let NodeConfig::Input(input) = project.nodes.get_mut(&node).unwrap() else {
        unreachable!()
    };
    input.source = InputSource::Application {
        executable: "moiren:process:42:124".into(),
        intent: CaptureIntent::PassiveTap,
    };
    assert!(backend.validate_measured_sources(&project).is_err());
}

#[test]
fn evaluated_backend_refuses_growth_beyond_the_exact_replayed_rate_and_whole_wake() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let (process, measured) = evidence();
    let options = measured
        .admit_for_consumer(&process, 7, target(), 48_000, 96)
        .unwrap();
    let backend = ProfessionalBackend::<f32>::new()
        .unwrap()
        .with_evaluated_process_bridge(node, &process, 7, measured, options)
        .unwrap();
    assert_eq!(
        backend
            .process_bridge_for(node, (42, 123), 48_000, 96)
            .unwrap(),
        options
    );
    assert!(
        backend
            .process_bridge_for(node, (42, 123), 48_000, 128)
            .is_err(),
        "target could fit128 but that demand was not replayed"
    );
    assert!(
        backend
            .process_bridge_for(node, (42, 123), 96_000, 96)
            .is_err()
    );
}

#[test]
fn ingress_elapsed_measured_builders_refuse_global_policy_before_recording_override() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    for limit in [1, u64::MAX] {
        let (process, measured) = evidence();
        let options = measured
            .admit_for_consumer(&process, 7, target(), 48_000, 96)
            .unwrap();
        let global = ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some(limit),
            ..Default::default()
        };
        let backend = ProfessionalBackend::<f32>::new()
            .unwrap()
            .with_process_bridge(global)
            .unwrap();
        let result =
            backend.with_measured_process_bridge(node, &process, 7, measured.clone(), target());
        assert!(matches!(result, Err(error) if error.to_string().contains("ingress")));
        let backend = ProfessionalBackend::<f64>::new()
            .unwrap()
            .with_process_bridge(global)
            .unwrap();
        let result = backend.with_evaluated_process_bridge(node, &process, 7, measured, options);
        assert!(matches!(result, Err(error) if error.to_string().contains("ingress")));
    }
}

#[test]
fn ingress_elapsed_resolver_refuses_later_global_policy_for_measured_and_evaluated_overrides() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    for evaluated in [false, true] {
        let (process, measured) = evidence();
        let options = measured
            .admit_for_consumer(&process, 7, target(), 48_000, 96)
            .unwrap();
        let backend = ProfessionalBackend::<f64>::new().unwrap();
        let backend = if evaluated {
            backend.with_evaluated_process_bridge(node, &process, 7, measured, options)
        } else {
            backend.with_measured_process_bridge(node, &process, 7, measured, target())
        }
        .unwrap()
        .with_process_bridge(ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some(123_456),
            ..Default::default()
        })
        .unwrap();
        for demand in [1, 96] {
            let error = backend
                .process_bridge_for(node, (42, 123), 48_000, demand)
                .unwrap_err();
            assert!(error.to_string().contains("ingress"));
        }
        let other = graph
            .create_node(moiren_core::graph::NodeKind::Source, 2)
            .unwrap();
        assert_eq!(
            backend
                .process_bridge_for(other, (43, 456), 48_000, 96)
                .unwrap()
                .max_ingress_elapsed_100ns,
            Some(123_456)
        );
    }
}

#[test]
fn ingress_elapsed_evaluated_builder_refuses_explicit_candidate_limit() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let (process, measured) = evidence();
    let mut options = measured
        .admit_for_consumer(&process, 7, target(), 48_000, 96)
        .unwrap();
    options.max_ingress_elapsed_100ns = Some(10_000);
    let result = ProfessionalBackend::<f32>::new()
        .unwrap()
        .with_evaluated_process_bridge(node, &process, 7, measured, options);
    assert!(matches!(result, Err(error) if error.to_string().contains("ingress")));
}
