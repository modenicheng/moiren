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
    data: Rc<UnsafeCell<Vec<f32>>>,
    calls: Rc<RefCell<Vec<(u32, u32)>>>,
    null_buffer: bool,
}
impl IAudioRenderClient_Impl for FakeRender_Impl {
    fn GetBuffer(&self, frames: u32) -> WinResult<*mut u8> {
        // SAFETY: This fake and its buffer are confined to the test thread.
        assert!(frames as usize * 2 <= unsafe { &*self.data.get() }.len());
        self.calls.borrow_mut().push((frames, u32::MAX));
        Ok(if self.null_buffer {
            std::ptr::null_mut()
        } else {
            // SAFETY: The Vec is never resized while the COM fake is alive.
            unsafe { &mut *self.data.get() }.as_mut_ptr().cast()
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
        let data = Rc::new(UnsafeCell::new(vec![99.0; 4]));
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
            unsafe { &*data.get() }.as_slice(),
            if null_buffer {
                [99.0; 4]
            } else {
                [0.1, -0.1, 0.2, -0.2]
            }
        );
    }
}

fn capture_renderer() -> (
    crate::clock_bridge::CaptureIngress,
    crate::clock_bridge::BridgeObserver,
    DemandRenderer,
) {
    use crate::clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge};
    use moiren_core::graph::*;
    use moiren_engine::{boundary::audio_bridge, compiler::*, runtime::EngineConfig};

    let (mut ingress, capture, observer) = capture_bridge(ClockBridgeConfig {
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    let bytes = 0.25f32.to_le_bytes().repeat(2048 * 2);
    assert_eq!(
        ingress
            .push_packet(
                &bytes,
                CapturePacket {
                    frames: 2048,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1,
                },
            )
            .unwrap(),
        2048
    );
    let (writer, reader) = audio_bridge::<f32>(2, 256, 4096).unwrap();
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
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, capture).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let engine = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 256,
                max_events_per_block: 8,
            },
            audio_byte_budget: 8192,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48000,
        },
    )
    .unwrap()
    .engine;
    (
        ingress,
        observer,
        DemandRenderer::new(engine, reader).unwrap(),
    )
}

#[test]
fn prestart_dsp_burst_exhausts_prepared_capture_reserve() {
    // Model the old startup demand, keeping ingress alive but with no capture
    // cadence during preparation. The actual engine/output bridge stay intact.
    let (_ingress, capture, mut renderer) = capture_renderer();
    let mut samples = vec![99.0; 4096 * 2];
    renderer.render_interleaved(&mut samples).unwrap();
    let snapshot = capture.snapshot();
    assert_eq!(renderer.timeline(), 4096);
    assert_eq!(snapshot.output_frames, 2047);
    assert_eq!(snapshot.underrun_frames, 1);
    assert_eq!(snapshot.last_underrun_at_output_frame, 2047);
    assert!(!snapshot.last_underrun_producer_finished);
    assert_eq!(snapshot.priming_frames, 2048);
    assert_eq!(snapshot.resets, 1);
    assert!(samples[..2047 * 2].iter().all(|&sample| sample == 0.25));
    assert!(samples[2047 * 2..].iter().all(|&sample| sample == 0.0));
}

#[test]
fn oversized_native_prime_submits_silence_without_consuming_capture_or_timeline() {
    let (_ingress, capture, mut renderer) = capture_renderer();
    let observer = crate::render::RenderObserver::default();
    renderer.set_observer(observer.clone()).unwrap();
    let data = Rc::new(UnsafeCell::new(vec![99.0; 4096 * 2]));
    let calls = Rc::new(RefCell::new(Vec::with_capacity(4)));
    let client: IAudioRenderClient = FakeRender {
        data: Rc::clone(&data),
        calls: Rc::clone(&calls),
        null_buffer: false,
    }
    .into();
    let mut staging = vec![99.0; 4096 * 2];
    let mut stats = crate::render::RenderStats::default();
    stream::prime(&client, &mut staging, &mut stats).unwrap();
    assert_eq!(renderer.timeline(), 0);
    assert_eq!(
        renderer.counters(),
        crate::render::DemandCounters::default()
    );
    assert!(!observer.snapshot().stream_started);
    assert_eq!(*calls.borrow(), [(4096, u32::MAX), (4096, 0)]);
    // SAFETY: Single-threaded inspection after ReleaseBuffer ended the lease.
    assert!(unsafe { &*data.get() }.iter().all(|&sample| sample == 0.0));
    assert_eq!(stats.primed_frames, 4096);
    assert_eq!(stats.submitted_frames, 4096);
    assert_eq!(stats.dsp_blocks, 0);
    assert_eq!(stats.dsp_segments, 0);
    assert_eq!(stats.nonzero_samples, 0);
    let snapshot = capture.snapshot();
    assert_eq!(snapshot.written_frames, 2048);
    assert_eq!(snapshot.output_frames, 0);
    assert_eq!(snapshot.underrun_frames, 0);
    assert_eq!(snapshot.priming_frames, 0);
    assert_eq!(snapshot.prime_discarded_frames, 0);
    assert_eq!(snapshot.resets, 0);

    // The first ordinary demand, after native Start, consumes the same reserve.
    renderer.set_stream_started(true);
    renderer
        .render_interleaved(&mut staging[..256 * 2])
        .unwrap();
    submit(&client, &staging[..256 * 2]).unwrap();
    assert!(staging[..256 * 2].iter().all(|&sample| sample == 0.25));
    assert_eq!(renderer.timeline(), 256);
    assert_eq!(capture.snapshot().output_frames, 256);
    assert_eq!(capture.snapshot().underrun_frames, 0);
    assert!(observer.snapshot().stream_started);
    // SAFETY: Single-threaded inspection after the second lease ended.
    assert!(
        unsafe { &*data.get() }[..256 * 2]
            .iter()
            .all(|&sample| sample == 0.25)
    );
}

struct DropTrackedSource(std::sync::Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>);
impl moiren_engine::boundary::RtAudioSource<f32> for DropTrackedSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        ctx: &moiren_engine::processor::ProcessContext,
        mut output: moiren_engine::buffer::AudioBlockMut<'_, f32>,
    ) -> moiren_engine::boundary::BoundaryReport {
        for channel in output.channels_mut() {
            channel.fill(0.0);
        }
        moiren_engine::boundary::BoundaryReport {
            transferred_frames: ctx.frames,
            ..Default::default()
        }
    }
}
impl Drop for DropTrackedSource {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = Some(std::thread::current().id());
    }
}

fn tracked_renderer(
    dropped_on: Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>,
) -> DemandRenderer {
    use moiren_core::graph::*;
    use moiren_engine::{boundary::audio_bridge, compiler::*, runtime::EngineConfig};
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
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(source, DropTrackedSource(dropped_on))
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let engine = compile(
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
    .unwrap()
    .engine;
    DemandRenderer::new(engine, reader).unwrap()
}

#[test]
fn stopped_and_panicking_owner_return_resources_after_scope_cleanup() {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct OwnerScope(Arc<AtomicBool>);
    impl Drop for OwnerScope {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let caller = std::thread::current().id();
    for panic in [false, true] {
        for cleanup in 0..3 {
            let dropped_on = Arc::new(Mutex::new(None));
            let cleaned = Arc::new(AtomicBool::new(false));
            let renderer = tracked_renderer(Arc::clone(&dropped_on));
            let stop = Arc::new(stop_event().unwrap());
            let worker_stop = Arc::clone(&stop);
            let worker_cleaned = Arc::clone(&cleaned);
            let worker = std::thread::spawn(move || {
                stream::run_owner_with(
                    RenderOptions::continuous("synthetic"),
                    renderer,
                    worker_stop,
                    |_, _, _, _| {
                        let _native_scope = OwnerScope(worker_cleaned);
                        if panic {
                            panic!("synthetic owner failure");
                        }
                        Ok(crate::render::RenderStatus::Stopped)
                    },
                )
            });
            let session = RenderSession {
                stop,
                worker: Some(worker),
            };
            let expected = if panic {
                crate::render::RenderStatus::Failed
            } else {
                crate::render::RenderStatus::Stopped
            };
            match cleanup {
                0 => {
                    let (report, renderer) = session.join_with_renderer().unwrap();
                    assert_eq!(report.status, expected);
                    assert_eq!(
                        report.failure.as_deref(),
                        panic.then_some("render worker panicked")
                    );
                    assert!(cleaned.load(Ordering::SeqCst));
                    assert_eq!(*dropped_on.lock().unwrap(), None);
                    drop(renderer);
                }
                1 => {
                    let result = session.join();
                    if panic {
                        assert!(matches!(result, Err(RenderError::WorkerPanicked)));
                    } else {
                        assert_eq!(result.unwrap().status, expected);
                    }
                }
                _ => drop(session),
            }
            assert!(cleaned.load(Ordering::SeqCst));
            assert_eq!(*dropped_on.lock().unwrap(), Some(caller));
        }
    }
}
