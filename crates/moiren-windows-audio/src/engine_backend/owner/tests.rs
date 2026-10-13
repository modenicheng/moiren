use super::*;
use crate::{
    engine_backend::WindowsBackendConfig,
    render::wasapi::{TestOutcome, prepare_test_native},
};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, NodeBindings, compile},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use std::{
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

struct Source {
    panic: bool,
    dropped: mpsc::Sender<std::thread::ThreadId>,
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}
impl RtAudioSource<f32> for Source {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        context: &ProcessContext,
        mut output: AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        assert!(!self.panic, "fake audio processor panic");
        for channel in output.channels_mut() {
            channel.fill(0.25);
        }
        BoundaryReport {
            transferred_frames: context.frames,
            ..BoundaryReport::default()
        }
    }
}

fn transport(
    outcome: TestOutcome,
    panic: bool,
) -> (
    Box<WindowsTransport>,
    Engine<f32>,
    CompiledBindings,
    mpsc::Receiver<std::thread::ThreadId>,
    Diagnostics,
) {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let native = prepare_test_native(stop.clone(), gate.clone(), outcome).unwrap();
    let (writer, reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let (dropped, receiver) = mpsc::channel();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(source, Source { panic, dropped })
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 3,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    let plans = compiled.engine.enable_plan_switching(1).unwrap();
    drop(plans);
    let diagnostics = Arc::new(Mutex::new(None));
    let resources = Resources::new(WindowsBackendConfig {
        probe_interval: Duration::from_secs(60),
        ..Default::default()
    })
    .unwrap();
    (
        Box::new(WindowsTransport {
            native: Some(native),
            captures: Vec::new(),
            output: Some(reader),
            sink,
            stop,
            gate,
            generation: 7,
            queue: RenderQueuePolicy::FullCapacity,
            diagnostics: diagnostics.clone(),
            resources: Arc::new(resources),
        }),
        compiled.engine,
        compiled.bindings,
        receiver,
        diagnostics,
    )
}

#[test]
fn managed_native_owner_waits_for_host_activation_and_returns_engine_after_cancel() {
    let (transport, engine, bindings, released, diagnostics) =
        transport(TestOutcome::RunUntilStopped, false);
    let owner = match transport.attach(engine, &bindings, CancellationToken::default()) {
        Ok(owner) => owner,
        Err(_) => panic!("fake native attachment failed"),
    };
    assert_eq!(owner.status(), OwnerStatus::Starting);
    assert_eq!(owner.timeline().unwrap().frame, 0);
    owner.activate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while owner.status() == OwnerStatus::Starting && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(owner.status(), OwnerStatus::Running);
    assert_eq!(
        owner.timeline(),
        Some(OwnerTimeline {
            epoch: 3,
            revision: 1,
            frame: 1
        })
    );
    assert!(released.try_recv().is_err());
    owner.cancel();
    let returned = owner.stop_and_join();
    assert!(returned.error.is_none());
    assert_eq!(returned.engine.timeline(), 1);
    assert!(
        diagnostics
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("stopped")
    );
    drop(returned);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(1)).unwrap(),
        std::thread::current().id()
    );
}

#[test]
fn audio_processor_panic_returns_engine_and_diagnostic_without_rt_destruction() {
    let (transport, engine, bindings, released, diagnostics) = transport(TestOutcome::Run, true);
    let owner = match transport.attach(engine, &bindings, CancellationToken::default()) {
        Ok(owner) => owner,
        Err(_) => panic!("fake attachment failed before audio processing"),
    };
    owner.activate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !matches!(owner.status(), OwnerStatus::Failed(_)) && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(released.try_recv().is_err());
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, Some(BackendError::WorkerPanicked));
    assert!(
        diagnostics
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("render worker panicked")
    );
    drop(returned);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(1)).unwrap(),
        std::thread::current().id()
    );
}

#[test]
fn native_attach_failure_keeps_owned_engine_and_failed_report() {
    let (transport, engine, bindings, released, diagnostics) =
        transport(TestOutcome::FailAfterReceive, false);
    let failure = match transport.attach(engine, &bindings, CancellationToken::default()) {
        Err(failure) => failure,
        Ok(_) => panic!("fake failing transport attached"),
    };
    assert!(matches!(failure.error, BackendError::Unsupported(_)));
    assert_eq!(failure.engine.timeline(), 0);
    assert!(released.try_recv().is_err());
    assert!(
        diagnostics
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("staging")
    );
    drop(failure);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(1)).unwrap(),
        std::thread::current().id()
    );
}

#[test]
fn stale_completion_can_cancel_and_return_engine_without_starting_native_stream() {
    let (transport, engine, bindings, released, _) = transport(TestOutcome::RunUntilStopped, false);
    let cancel = CancellationToken::default();
    let owner = match transport.attach(engine, &bindings, cancel.clone()) {
        Ok(owner) => owner,
        Err(_) => panic!("fake waiting transport failed"),
    };
    cancel.cancel();
    assert_eq!(owner.activate(), Err(BackendError::Cancelled));
    let returned = owner.stop_and_join();
    assert_eq!(returned.engine.timeline(), 0);
    assert!(returned.error.is_none());
    assert!(released.try_recv().is_err());
    drop(returned);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(1)).unwrap(),
        std::thread::current().id()
    );
}
