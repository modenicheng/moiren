use super::*;
use crate::service::{
    ManagedDriver, ProcessingPrecision,
    test_backend::{FakeBackend, State},
};
use std::sync::{Arc, atomic::Ordering};

fn options() -> MonitorOptions {
    let mut options = MonitorOptions::default();
    options.set("--process", "4294967294").unwrap();
    options.set("--output", "pinned-output").unwrap();
    options
}

fn source() -> InputSelection {
    InputSelection::Process {
        pid: 4294967294,
        creation_time_100ns: 12345,
    }
}

#[test]
fn actual_context_is_only_io_bus_with_stable_process_and_pinned_output() {
    let (context, spec) = context(&options(), source()).unwrap();
    let graph = context.graph.snapshot();
    assert_eq!(
        graph
            .graph
            .nodes()
            .iter()
            .map(|node| node.kind())
            .collect::<Vec<_>>(),
        vec![NodeKind::Source, NodeKind::Bus, NodeKind::Sink]
    );
    assert_eq!(graph.graph.edges().len(), 2);
    assert_eq!(spec.source, source());
    assert_eq!(spec.output_endpoint_id, "pinned-output");
    assert_eq!(
        graph.node_of_kind(NodeKind::Source).unwrap().1.source,
        Some(source())
    );
    assert_eq!(
        context.transport_latency,
        TransportLatencySettings::default()
    );
    assert!(context.loaded_project.is_none());
    assert!(context.graph.snapshot().exports.is_empty());
}

#[test]
fn context_refuses_missing_or_mismatched_stable_identity_before_start() {
    for selection in [
        InputSelection::Process {
            pid: 4294967294,
            creation_time_100ns: 0,
        },
        InputSelection::Process {
            pid: 12,
            creation_time_100ns: 12345,
        },
        InputSelection::Tone {
            frequency_hz: 440.0,
        },
    ] {
        assert!(context(&options(), selection).is_err());
    }
}

#[test]
fn supervised_real_service_confirms_both_precisions_and_reaps_owner() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let state = Arc::new(State::default());
        let f32_backend = FakeBackend(state.clone());
        let f64_backend = f32_backend.clone();
        let driver = ManagedDriver::new(move |_| Ok(Box::new(f32_backend.clone())))
            .with_f64_factory(move |_| Ok(Box::new(f64_backend.clone())));
        let mut options = options();
        options.audio.processing.precision = precision;
        let report = run_with_driver(options, source(), driver, Duration::from_millis(20)).unwrap();
        assert!(report.run_complete, "{report:?}");
        assert!(report.cleanup_complete, "{report:?}");
        assert_eq!(report.failure, None);
        assert!(!report.qualified);
        assert_eq!(
            report.confirmed_active_audio.unwrap().processing.precision,
            precision
        );
        assert_eq!(
            report.confirmed_active_transport,
            Some(TransportLatencySettings::default())
        );
        assert_eq!(state.starts.load(Ordering::SeqCst), 1);
        assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn preparation_refusal_is_not_active_or_successful_but_still_joins() {
    let driver = ManagedDriver::new(|_| {
        Err(moiren_engine::host::BackendError::Unsupported(
            "refused test budget".into(),
        ))
    });
    let report = run_with_driver(options(), source(), driver, Duration::from_millis(20)).unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete);
    assert!(report.confirmed_active_audio.is_none());
    assert!(report.failure.unwrap().contains("refused test budget"));
}

#[test]
fn owner_fault_keeps_failure_and_reaps_without_claiming_run_complete() {
    let state = Arc::new(State::default());
    state.fail_owner.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state.clone());
    let report = run_with_driver(
        options(),
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(!report.run_complete);
    // OwnerReturn carries this injected error through cleanup as well. A
    // successful service join must not erase that final owner error.
    assert!(!report.cleanup_complete, "{report:?}");
    assert!(report.failure.is_some());
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn unavailable_resource_fails_without_claiming_an_active_session() {
    let state = Arc::new(State::default());
    state.unavailable.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state);
    let mut options = options();
    options.startup_timeout_ms = 30;
    let report = run_with_driver(
        options,
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete);
    assert!(report.failure.is_some(), "{report:?}");
    assert!(report.confirmed_generation.is_none());
}

#[test]
fn startup_deadline_exits_and_joins_when_no_owner_is_ever_confirmed() {
    struct WaitingDriver;
    impl ServiceDriver for WaitingDriver {
        fn request(
            &mut self,
            context: &mut ServiceContext,
            _: &mut crate::service::JobPool,
            request: crate::service::AppRequest,
        ) {
            if let crate::service::AppRequest::Start(spec) = request {
                context.core.start(spec).unwrap();
            }
        }
        fn is_drained(&self) -> bool {
            true
        }
    }
    let mut options = options();
    options.startup_timeout_ms = 30;
    let report =
        run_with_driver(options, source(), WaitingDriver, Duration::from_millis(20)).unwrap();
    assert!(!report.run_complete);
    assert!(report.cleanup_complete, "{report:?}");
    assert!(report.failure.unwrap().contains("startup deadline"));
}

#[test]
fn cleanup_failure_is_retained_even_after_the_run_duration_completes() {
    let state = Arc::new(State::default());
    state.fail_stop.store(true, Ordering::SeqCst);
    let backend = FakeBackend(state.clone());
    let report = run_with_driver(
        options(),
        source(),
        ManagedDriver::new(move |_| Ok(Box::new(backend.clone()))),
        Duration::from_millis(20),
    )
    .unwrap();
    assert!(report.run_complete, "{report:?}");
    assert!(!report.cleanup_complete, "{report:?}");
    assert!(
        report
            .failure
            .as_deref()
            .is_some_and(|error| error.contains("injected stop failure")),
        "{report:?}"
    );
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}
