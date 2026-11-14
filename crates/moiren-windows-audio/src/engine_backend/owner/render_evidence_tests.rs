use super::*;
use crate::{render_evidence::*, test_allocation::count_allocations};
use moiren_engine::audio_config::SampleFormat;

struct Stream {
    writer: Option<RenderEvidenceWriter>,
    dropped: Arc<AtomicBool>,
    panic_drop: bool,
}
impl NativeStream for Stream {
    fn info(&self) -> NativeStreamInfo {
        unreachable!()
    }
    fn start(&mut self) -> Result<(), BackendError> {
        unreachable!()
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        unreachable!()
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        unreachable!()
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        unreachable!()
    }
    fn write(&mut self, _: &[StereoFrame]) -> Result<usize, BackendError> {
        unreachable!()
    }
    fn take_render_evidence(&mut self) -> Option<RenderEvidenceWriter> {
        self.writer.take()
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        assert!(
            self.writer.is_none(),
            "scratch must leave native stream before Drop"
        );
        self.dropped.store(true, Ordering::Release);
        assert!(!self.panic_drop, "injected native destructor failure");
    }
}

fn stream(panic_drop: bool) -> (Box<dyn NativeStream>, RenderEvidenceReader, Arc<AtomicBool>) {
    let (mut writer, reader) = render_evidence_stream(
        RenderEvidenceConfig {
            capacity_frames: 2,
            max_packet_frames: 2,
        },
        RenderEvidenceFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::Float32,
            channels: 2,
            container_bits: 32,
            valid_bits: 32,
        },
    )
    .unwrap();
    writer.stage_packet(&[7; 8], 1).unwrap();
    writer.record_event(RenderEventSample {
        kind: RenderEventKind::Start,
        ..Default::default()
    });
    let dropped = Arc::new(AtomicBool::new(false));
    (
        Box::new(Stream {
            writer: Some(writer),
            dropped: dropped.clone(),
            panic_drop,
        }),
        reader,
        dropped,
    )
}

#[test]
fn render_evidence_retirement_survives_native_destructor_panic() {
    for panic_drop in [false, true] {
        let (stream, mut reader, dropped) = stream(panic_drop);
        let mut retired = None;
        let guard = RenderEvidenceStreamGuard::new(stream, &mut retired);
        let outcome = catch_unwind(AssertUnwindSafe(|| drop(guard)));
        assert_eq!(outcome.is_err(), panic_drop);
        assert!(dropped.load(Ordering::Acquire));
        let writer = retired.as_ref().unwrap();
        assert!(writer.is_closed());
        assert_eq!(writer.snapshot().pending_abandoned_frames, 1);
        assert_eq!(writer.snapshot().accepted_frames, 0);
        assert!(reader.is_finished());
        assert!(reader.pop().is_none());
        assert!(!reader.events_finished());
        assert_eq!(
            reader.pop_event().unwrap().sample.kind,
            RenderEventKind::Start
        );
        assert!(reader.events_finished());
    }
}

#[test]
fn render_evidence_retirement_frees_only_native_box_not_evidence_storage() {
    let (stream, reader, dropped) = stream(false);
    let mut retired = None;
    let guard = RenderEvidenceStreamGuard::new(stream, &mut retired);
    let (_, counts) = count_allocations(|| drop(guard));
    assert_eq!(
        counts,
        (0, 1),
        "only the native stream box is reclaimed here"
    );
    assert!(dropped.load(Ordering::Acquire));
    assert!(retired.as_ref().unwrap().is_closed());
    drop(reader);
    let (_, (_, freed)) = count_allocations(|| drop(retired));
    assert!(
        freed >= 3,
        "writer scratch, ring and metadata retire on control"
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Normal,
    PreparedCancel,
    InvalidMetadata,
    StartPanic,
    DropPanic,
}

#[test]
fn render_evidence_worker_returns_storage_on_run_cancel_preparation_failure_and_panic() {
    struct Lifecycle {
        inner: Stream,
        phase: Phase,
        state: Arc<RunState>,
    }
    impl NativeStream for Lifecycle {
        fn info(&self) -> NativeStreamInfo {
            NativeStreamInfo {
                sample_rate: if self.phase == Phase::InvalidMetadata {
                    0
                } else {
                    48_000
                },
                format: SampleFormat::Float32,
                channels: 2,
                buffer_frames: 128,
            }
        }
        fn start(&mut self) -> Result<(), BackendError> {
            self.state.cancel();
            assert!(self.phase != Phase::StartPanic, "injected Start failure");
            Ok(())
        }
        fn stop(&mut self) -> Result<(), BackendError> {
            Ok(())
        }
        fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
            unreachable!()
        }
        fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
            unreachable!()
        }
        fn write(&mut self, _: &[StereoFrame]) -> Result<usize, BackendError> {
            unreachable!()
        }
        fn take_render_evidence(&mut self) -> Option<RenderEvidenceWriter> {
            self.inner.writer.take()
        }
    }
    for phase in [
        Phase::Normal,
        Phase::PreparedCancel,
        Phase::InvalidMetadata,
        Phase::StartPanic,
        Phase::DropPanic,
    ] {
        let (writer, reader) = render_evidence_stream(
            RenderEvidenceConfig {
                capacity_frames: 128,
                max_packet_frames: 128,
            },
            RenderEvidenceFormat {
                sample_rate: 48_000,
                sample_format: SampleFormat::Float32,
                channels: 2,
                container_bits: 32,
                valid_bits: 32,
            },
        )
        .unwrap();
        let state = Arc::new(RunState::default());
        let native_state = state.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let native_dropped = dropped.clone();
        let (render_tx, render_rx) = rtrb::RingBuffer::new(1024);
        let result = spawn(
            WorkerRequest::<f32> {
                factory: Box::new(move || {
                    Ok(Box::new(Lifecycle {
                        inner: Stream {
                            writer: Some(writer),
                            dropped: native_dropped,
                            panic_drop: phase == Phase::DropPanic,
                        },
                        phase,
                        state: native_state,
                    }))
                }),
                capture: None,
                capture_clock: None,
                render: Some(render_rx),
                counters: Arc::new(Counters::default()),
                state: state.clone(),
                slot: Arc::new(EngineSlot::default()),
                timeline: Arc::new(Timeline::default()),
                engine_rate: 48_000,
                capacity: 1024,
                master: false,
                recovery: None,
            },
            &CancellationToken::default(),
            std::time::Duration::from_secs(1),
        );
        if phase == Phase::InvalidMetadata {
            assert!(result.is_err());
        } else {
            let (mut worker, _) = result.unwrap();
            if phase == Phase::PreparedCancel {
                state.cancel();
            } else {
                state.activate().unwrap();
            }
            worker.join().unwrap();
            let retired = worker.retired_render_evidence.as_ref().unwrap();
            assert!(retired.is_closed());
            assert_eq!(retired.snapshot().attempts, 0);
            assert!(
                state.failure.lock().unwrap().is_some()
                    == matches!(phase, Phase::StartPanic | Phase::DropPanic)
            );
        }
        assert!(dropped.load(Ordering::Acquire));
        assert!(reader.is_finished());
        if phase != Phase::InvalidMetadata {
            assert!(
                reader.final_summary().is_some(),
                "every joined exit path must publish its terminal evidence"
            );
        }
        drop(render_tx);
    }
}
