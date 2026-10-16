use super::*;
use crate::{
    Activation, ActivationGate,
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    engine_backend::WindowsBackend,
    render::wasapi::{TestOutcome, prepare_test_native, prepare_test_native_with_info},
};
use moiren_core::graph::{LogicalGraph, NodeKind};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

fn nodes() -> [NodeId; 2] {
    let mut graph = LogicalGraph::new();
    [
        graph.create_node(NodeKind::Sink, 2).unwrap(),
        graph.create_node(NodeKind::Sink, 2).unwrap(),
    ]
}
fn timing() -> NativeStreamTiming {
    NativeStreamTiming {
        sample_rate: 48_000,
        period_frames: Some(336),
        capacity_frames: 738,
    }
}
fn report(diagnostics: &Diagnostics) -> serde_json::Value {
    serde_json::from_str(diagnostics.lock().unwrap().as_ref().unwrap()).unwrap()
}
fn assert_unstarted(
    report: &serde_json::Value,
    expected_outputs: usize,
    expected_captures: usize,
    failure: &BackendError,
) {
    assert_eq!(report["failure"], failure.to_string());
    let prepared = report["prepared"].as_array().unwrap();
    assert_eq!(prepared.len(), expected_outputs);
    for output in prepared {
        assert!(output["endpoint_id"].as_str().unwrap().starts_with("fake-"));
        assert_eq!(output["stream_started"], false);
        assert_eq!(output["stats"]["processed_frames"], 0);
    }
    let captures = report["capture"].as_array().unwrap();
    assert_eq!(captures.len(), expected_captures);
    for capture in captures {
        assert_eq!(capture["stream_started"], false);
    }
}

#[test]
fn first_output_rejected_after_negotiation_replaces_previous_report_and_joins() {
    let backend = WindowsBackend::new(Default::default()).unwrap();
    *backend.diagnostics.lock().unwrap() = Some("previous preparation".into());
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let mut group = PreparationGroup::new(stop.clone(), backend.diagnostics.clone());
    let native = prepare_test_native_with_info(
        stop,
        gate.clone(),
        TestOutcome::RunUntilStopped,
        NativeRenderInfo {
            timing: NativeStreamTiming {
                period_frames: Some(240),
                capacity_frames: 300,
                ..timing()
            },
            shared_period: Default::default(),
        },
    )
    .unwrap();
    let failure = group
        .admit_output(
            nodes()[0],
            native,
            NativeLatencyProfile::Low,
            RenderQueuePolicy::FullCapacity,
        )
        .unwrap_err();
    drop(group);
    let current: serde_json::Value =
        serde_json::from_str(&backend.last_native_report().unwrap()).unwrap();
    assert_unstarted(&current, 1, 0, &failure);
    assert_eq!(current["prepared"][0]["buffer_frames"], 300);
    assert_eq!(current["prepared"][0]["endpoint_id"], "fake-300");
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
}

#[test]
fn follower_bridge_rejection_keeps_every_negotiated_output_in_cleanup() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let diagnostics = Arc::new(Mutex::new(None));
    let mut group = PreparationGroup::new(stop.clone(), diagnostics.clone());
    for (node, capacity) in nodes().into_iter().zip([738, 600]) {
        let native = prepare_test_native_with_info(
            stop.clone(),
            gate.clone(),
            TestOutcome::RunUntilStopped,
            NativeRenderInfo {
                timing: NativeStreamTiming {
                    capacity_frames: capacity,
                    ..timing()
                },
                shared_period: Default::default(),
            },
        )
        .unwrap();
        group
            .admit_output(
                node,
                native,
                NativeLatencyProfile::Low,
                RenderQueuePolicy::FullCapacity,
            )
            .unwrap();
    }
    let master_queue = crate::latency::prepare_render_queue(timing(), group.prepared[0].2).unwrap();
    let input = NativeLatencyProfile::Low
        .follower_input(
            timing(),
            timing(),
            LatencyBudgetPolicy::Compatible,
            master_queue,
        )
        .unwrap();
    let mut config = ClockBridgeConfig::with_prepared_input(48_000, 2, input);
    config.byte_budget = 0;
    let failure = match group
        .check(capture_bridge(config).map_err(|error| BackendError::Unsupported(error.to_string())))
    {
        Err(error) => error,
        Ok(_) => panic!("follower ring unexpectedly fit a zero byte budget"),
    };
    assert_eq!(group.prepared.len(), 2);
    drop(group);
    assert_unstarted(&report(&diagnostics), 2, 0, &failure);
    let current = report(&diagnostics);
    assert_eq!(current["prepared"][0]["endpoint_id"], "fake-738");
    assert_eq!(current["prepared"][1]["endpoint_id"], "fake-600");
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
}

#[test]
fn capture_rate_budget_and_attachment_rejections_preserve_native_and_peer_reports() {
    for rejection in 0..3 {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let diagnostics = Arc::new(Mutex::new(None));
        let mut group = PreparationGroup::new(stop.clone(), diagnostics.clone());
        let native =
            prepare_test_native(stop.clone(), gate.clone(), TestOutcome::RunUntilStopped).unwrap();
        group
            .admit_output(
                nodes()[0],
                native,
                NativeLatencyProfile::Compatible,
                RenderQueuePolicy::FullCapacity,
            )
            .unwrap();
        let exited = Arc::new(AtomicBool::new(false));
        let capture =
            crate::capture::wasapi::prepare_test_native(stop, gate.clone(), exited.clone())
                .unwrap();
        let (requested, profile, output, policy) = match rejection {
            0 => (
                Some(44_100),
                NativeLatencyProfile::Compatible,
                timing(),
                LatencyBudgetPolicy::Compatible,
            ),
            1 => (
                None,
                NativeLatencyProfile::Low,
                NativeStreamTiming {
                    capacity_frames: 300,
                    ..timing()
                },
                LatencyBudgetPolicy::Compatible,
            ),
            _ => (
                None,
                NativeLatencyProfile::Compatible,
                timing(),
                LatencyBudgetPolicy::Custom {
                    input_capacity_frames: 1_000_000,
                    input_target_frames: 2048,
                    input_jitter_frames: 0,
                    render: RenderQueuePolicy::FullCapacity,
                },
            ),
        };
        let failure = match group.admit_capture(capture, requested, profile, output, policy) {
            Err(error) => error,
            Ok(_) => panic!("invalid capture admission succeeded"),
        };
        drop(group);
        assert!(
            exited.load(Ordering::Acquire),
            "capture owner must join before rejection returns"
        );
        let current = report(&diagnostics);
        assert_unstarted(&current, 1, 1, &failure);
        assert_eq!(current["capture"][0]["sample_rate"], 48_000);
        assert_eq!(current["capture"][0]["endpoint_id"], "fake-capture");
        assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
    }
}
