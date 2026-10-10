use super::*;
use super::{endpoint::project_render_endpoints, format::validate_format, packet::submit};
use crate::{catalog, owner::OwnedHandle};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};
use windows::Win32::Media::Audio::*;
use windows::core::{Result as WinResult, implement};

#[test]
fn render_preparation_rejects_foreign_gate_stop_before_spawning() {
    use crate::{ActivationGate, GateError, SessionDuration};
    let (tx, rx) = std::sync::mpsc::channel();
    let result = start_render_prepared_with_gate(
        RenderOptions {
            endpoint_id: "unopened".into(),
            duration: SessionDuration::UntilStopped,
        },
        probe_renderer(tx),
        StopSignal::new().unwrap(),
        ActivationGate::new(StopSignal::new().unwrap()).unwrap(),
    );
    assert!(matches!(result, Err(RenderError::Gate(GateError::Api {
        stage: "ActivationGate stop identity", hresult,
    })) if hresult == 0x80070057u32 as i32));
    assert_eq!(rx.recv().unwrap(), std::thread::current().id());
}

#[test]
fn usable_render_choices_survive_missing_capture_and_local_errors() {
    let mut mix_format = catalog::FormatSnapshot::from_base(WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    });
    mix_format.channel_mask = Some(3);
    let endpoints = project_render_endpoints(catalog::CatalogSnapshot {
        defaults: vec![catalog::DefaultEndpoint {
            flow: "capture",
            role: "console",
            id: None,
        }],
        endpoints: vec![catalog::EndpointSnapshot {
            id: "speakers".into(),
            name: Some("Usable speakers".into()),
            flow: "render",
            mix_format: Some(mix_format),
            default_period_100ns: None,
            minimum_period_100ns: None,
            volume_scalar: None,
            muted: None,
            sessions: Vec::new(),
            errors: vec![catalog::ApiFailure {
                stage: "session enumeration".into(),
                hresult: "0x80070005".into(),
            }],
        }],
        errors: vec![catalog::ApiFailure {
            stage: "missing capture default".into(),
            hresult: "0x80070490".into(),
        }],
    });
    assert_eq!(endpoints[0].endpoint_id, "speakers");
    assert_eq!(endpoints[0].format_supported, Some(true));
    assert!(endpoints[0].errors[0].contains("0x80070005"));
    assert_eq!(endpoints[1].endpoint_id, "");
    assert!(endpoints[1].errors[0].contains("0x80070490"));
}

#[test]
fn stop_wakes_without_any_audio_event_and_takes_priority() {
    let stop = stop_event().unwrap();
    let audio = OwnedHandle::event().unwrap();
    assert_eq!(wait(handle(&stop), audio.0, 0).unwrap(), Wake::Timeout);
    unsafe {
        SetEvent(handle(&stop)).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
    unsafe {
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
}
#[test]
fn native_format_validation_rejects_conversion_and_bad_layouts() {
    let base = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    assert!(validate_format(base, true, None, None).is_ok());
    assert!(validate_format(base, true, Some(32), Some(3)).is_ok());
    for bad in [
        WAVEFORMATEX {
            nSamplesPerSec: 44100,
            ..base
        },
        WAVEFORMATEX {
            nChannels: 1,
            ..base
        },
        WAVEFORMATEX {
            nBlockAlign: 4,
            ..base
        },
        WAVEFORMATEX {
            wBitsPerSample: 24,
            ..base
        },
        WAVEFORMATEX {
            nAvgBytesPerSec: 192000,
            ..base
        },
    ] {
        assert_eq!(
            validate_format(bad, true, None, None),
            Err(RenderError::UnsupportedFormat)
        );
    }
    assert_eq!(
        validate_format(base, false, None, None),
        Err(RenderError::UnsupportedFormat)
    );
    assert_eq!(
        validate_format(base, true, Some(24), None),
        Err(RenderError::UnsupportedFormat)
    );
    assert_eq!(
        validate_format(base, true, Some(32), Some(12)),
        Err(RenderError::UnsupportedFormat)
    );
}
#[implement(IAudioRenderClient)]
struct FakeRender {
    data: Rc<UnsafeCell<[f32; 4]>>,
    calls: Rc<RefCell<Vec<(u32, u32)>>>,
    null_buffer: bool,
}
impl IAudioRenderClient_Impl for FakeRender_Impl {
    fn GetBuffer(&self, frames: u32) -> WinResult<*mut u8> {
        assert_eq!(frames, 2);
        self.calls.borrow_mut().push((frames, u32::MAX));
        Ok(if self.null_buffer {
            std::ptr::null_mut()
        } else {
            self.data.get().cast()
        })
    }
    fn ReleaseBuffer(&self, frames: u32, flags: u32) -> WinResult<()> {
        self.calls.borrow_mut().push((frames, flags));
        Ok(())
    }
}
#[test]
fn render_leases_copy_full_pcm_or_cancel_without_submitting_stale_data() {
    for null_buffer in [false, true] {
        let data = Rc::new(UnsafeCell::new([99.0; 4]));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let client: IAudioRenderClient = FakeRender {
            data: Rc::clone(&data),
            calls: Rc::clone(&calls),
            null_buffer,
        }
        .into();
        assert!(submit(&client, &[]).is_ok());
        assert!(calls.borrow().is_empty());
        let result = submit(&client, &[0.1, -0.1, 0.2, -0.2]);
        assert_eq!(result.is_err(), null_buffer);
        assert_eq!(
            *calls.borrow(),
            [(2, u32::MAX), (if null_buffer { 0 } else { 2 }, 0)]
        );
        // SAFETY: Single-threaded test reads after the buffer lease ended.
        assert_eq!(
            unsafe { *data.get() },
            if null_buffer {
                [99.0; 4]
            } else {
                [0.1, -0.1, 0.2, -0.2]
            }
        );
    }
}

#[test]
fn gate_cancel_is_terminal_and_stop_has_priority() {
    use crate::{Activation, ActivationGate, GateError, StopSignal};
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop).unwrap();
    gate.cancel().unwrap();
    assert_eq!(gate.activate(), Err(GateError::AlreadyResolved));
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);

    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop).unwrap();
    gate.activate().unwrap();
    assert_eq!(gate.cancel(), Err(GateError::AlreadyResolved));
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
}

#[test]
fn shared_gate_releases_both_owners_only_after_activation() {
    use crate::{Activation, ActivationGate, StopSignal};
    let gate = ActivationGate::new(StopSignal::new().unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let gate = gate.clone();
            let tx = tx.clone();
            std::thread::spawn(move || tx.send(gate.wait().unwrap()).unwrap())
        })
        .collect();
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(20))
            .is_err()
    );
    gate.activate().unwrap();
    for _ in 0..2 {
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
            Activation::Start
        );
    }
    for worker in workers {
        worker.join().unwrap();
    }
}

fn probe_renderer(dropped: std::sync::mpsc::Sender<std::thread::ThreadId>) -> DemandRenderer {
    use moiren_core::graph::*;
    use moiren_engine::{boundary::*, compiler::*, runtime::*};
    struct DropProbe(std::sync::mpsc::Sender<std::thread::ThreadId>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.send(std::thread::current().id()).unwrap();
        }
    }
    impl RtAudioSource<f32> for DropProbe {
        fn channel_count(&self) -> usize {
            2
        }
        fn read(
            &mut self,
            ctx: &moiren_engine::processor::ProcessContext,
            mut output: moiren_engine::buffer::AudioBlockMut<'_, f32>,
        ) -> BoundaryReport {
            for channel in output.channels_mut() {
                channel.fill(0.25);
            }
            BoundaryReport {
                transferred_frames: ctx.frames,
                ..BoundaryReport::default()
            }
        }
    }
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
    let (writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, DropProbe(dropped)).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48000,
        },
    )
    .unwrap();
    DemandRenderer::new(compiled.engine, reader).unwrap()
}

#[test]
fn prepared_render_waits_before_dsp_and_returns_renderer_to_joiner() {
    use crate::{Activation, ActivationGate, SessionDuration};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    for cancel in [false, true] {
        let (tx, rx) = mpsc::channel();
        let renderer = probe_renderer(tx);
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let ran = runs.clone();
        let prepared = start_owner(
            renderer,
            stop,
            gate,
            move |renderer, stop, gate, ready, started| {
                stream::run_owner_with(
                    RenderOptions {
                        endpoint_id: "fake".into(),
                        duration: SessionDuration::UntilStopped,
                    },
                    renderer,
                    stop,
                    gate,
                    ready,
                    started,
                    move |_, renderer, _, report, gate, ready, started| {
                        if ready_and_wait(ready, gate)? == Activation::Cancelled {
                            return Ok(super::super::RenderStatus::Stopped);
                        }
                        renderer.render_interleaved(&mut [0.0; 2])?;
                        ran.fetch_add(1, Ordering::SeqCst);
                        started.store(true, Ordering::Release);
                        report.stream_started = true;
                        Ok(super::super::RenderStatus::Completed)
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert!(!prepared.has_started());
        assert!(rx.try_recv().is_err());
        if cancel {
            prepared.request_stop().unwrap();
        } else {
            prepared.activate().unwrap();
        }
        let exit = prepared.into_session().join_with_renderer().unwrap();
        assert_eq!(exit.renderer.timeline(), u64::from(!cancel));
        assert_eq!(runs.load(Ordering::SeqCst), usize::from(!cancel));
        let json = serde_json::to_value(&exit.report).unwrap();
        assert_eq!(json["schema_version"], 2);
        assert!(json["requested_seconds"].is_null());
        assert!(rx.try_recv().is_err());
        drop(exit.renderer);
        assert_eq!(rx.recv().unwrap(), std::thread::current().id());
    }
}

#[test]
fn post_ready_failure_returns_unstarted_renderer_and_finite_report() {
    use crate::{Activation, ActivationGate, SessionDuration};
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = start_owner(
        probe_renderer(tx),
        stop,
        gate,
        |renderer, stop, gate, ready, started| {
            stream::run_owner_with(
                RenderOptions {
                    endpoint_id: "fake".into(),
                    duration: SessionDuration::For(std::time::Duration::from_secs(10)),
                },
                renderer,
                stop,
                gate,
                ready,
                started,
                |_, _, _, _, gate, ready, _| {
                    assert_eq!(ready_and_wait(ready, gate)?, Activation::Start);
                    Err(RenderError::Api {
                        stage: "Start",
                        hresult: -1,
                    })
                },
            )
        },
    )
    .unwrap();
    prepared.activate().unwrap();
    let session = prepared.into_session();
    while !session.is_finished() {
        std::thread::yield_now();
    }
    assert!(!session.has_started());
    let exit = session.join_with_renderer().unwrap();
    assert_eq!(exit.report.status, super::super::RenderStatus::Failed);
    assert!(!exit.report.stream_started);
    assert_eq!(exit.renderer.timeline(), 0);
    let json = serde_json::to_value(&exit.report).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["requested_seconds"], 10.0);
    assert!(rx.try_recv().is_err());
    drop(exit.renderer);
    assert_eq!(rx.recv().unwrap(), std::thread::current().id());
}

#[test]
fn render_prepare_failure_joins_and_drops_renderer_on_preparing_thread() {
    use crate::{ActivationGate, SessionDuration};
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let result = start_owner(
        probe_renderer(tx),
        stop,
        gate,
        |renderer, stop, gate, ready, started| {
            stream::run_owner_with(
                RenderOptions {
                    endpoint_id: "fake".into(),
                    duration: SessionDuration::UntilStopped,
                },
                renderer,
                stop,
                gate,
                ready,
                started,
                |_, _, _, _, _, _, _| Err(RenderError::UnsupportedFormat),
            )
        },
    );
    assert!(matches!(result, Err(RenderError::UnsupportedFormat)));
    assert_eq!(rx.recv().unwrap(), std::thread::current().id());
}
