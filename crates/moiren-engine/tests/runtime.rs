use moiren_core::protocol::*;
use moiren_engine::{boundary::*, buffer::*, control::*, meter::*, processor::*, runtime::*};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

// Counts only this test thread while render is active; unrelated parallel test
// allocations cannot contaminate the result. TLS has constant initialization.
struct CountingAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a + 1, d));
                });
            }
        });
        // SAFETY: forwarding the caller's allocation contract unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a, d + 1));
                });
            }
        });
        // SAFETY: ptr/layout originated from this allocator's System call.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const REV: u64 = 7;
const EPOCH: u64 = 3;
const SOURCE: ProcessorId = ProcessorId(10);
const GAIN: ProcessorId = ProcessorId(20);
const METER: ProcessorId = ProcessorId(30);
const SINK: ProcessorId = ProcessorId(40);

struct Capture {
    samples: Arc<[AtomicU64]>,
}
impl RtAudioSink<f64> for Capture {
    fn channel_count(&self) -> usize {
        1
    }
    fn write(&mut self, ctx: &ProcessContext, input: AudioBlock<'_, f64>) -> BoundaryReport {
        for (i, sample) in input.channel(0).iter().enumerate() {
            self.samples[ctx.timeline_start as usize + i]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}
fn request(id: u64, at: ApplyAt, value: f64, ramp_frames: u32) -> ParameterRequest {
    ParameterRequest {
        request_id: id,
        plan_revision: REV,
        timeline_epoch: EPOCH,
        target: ParameterKey {
            processor: GAIN,
            parameter: Gain::LEVEL,
        },
        at,
        value: ParamValue::Float(value),
        ramp_frames,
    }
}
fn config(events: usize) -> EngineConfig {
    EngineConfig {
        processing_sr: 48000.0,
        max_block_frames: 16,
        max_events_per_block: events,
    }
}
fn pipeline(
    in_place: bool,
    events: usize,
    queue: usize,
) -> (Engine<f64>, ControlPort, MeterReader, Arc<[AtomicU64]>) {
    let (control, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, queue, 1024).unwrap();
    let (meter, reader) = level_meter(1, 1).unwrap();
    let samples: Arc<[AtomicU64]> = (0..256)
        .map(|_| AtomicU64::new(f64::NAN.to_bits()))
        .collect::<Vec<_>>()
        .into();
    let resources = RtResources::new(vec![
        ProcessorInstance::new(
            SOURCE,
            SourceAdapter(ConstantSource {
                channels: 1,
                value: 0.25,
            }),
        ),
        ProcessorInstance::new(GAIN, Gain),
        ProcessorInstance::new(METER, Observer(meter)),
        ProcessorInstance::new(
            SINK,
            SinkAdapter(Capture {
                samples: Arc::clone(&samples),
            }),
        ),
    ])
    .unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }; 2],
        4096,
    )
    .unwrap();
    let src = arena.slot(0).unwrap();
    let dst = if in_place {
        src
    } else {
        arena.slot(1).unwrap()
    };
    let source = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot: src }])
        .unwrap();
    let observer = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot: src }])
        .unwrap();
    let gain = arena
        .prepare_io(&if in_place {
            vec![PortAccess::InPlace {
                input: 0,
                output: 0,
                slot: src,
            }]
        } else {
            vec![
                PortAccess::Read { port: 0, slot: src },
                PortAccess::Write { port: 0, slot: dst },
            ]
        })
        .unwrap();
    let sink = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot: dst }])
        .unwrap();
    // Observer is an additional consumer BEFORE the last-use in-place mutation.
    let plan = ExecutionPlan::prepare(
        arena,
        vec![
            OpSpec {
                processor: SOURCE,
                io: source,
            },
            OpSpec {
                processor: METER,
                io: observer,
            },
            OpSpec {
                processor: GAIN,
                io: gain,
            },
            OpSpec {
                processor: SINK,
                io: sink,
            },
        ],
        &resources,
        &params,
        config(events),
    )
    .unwrap();
    (
        Engine::new(plan, resources, params).unwrap(),
        control,
        reader,
        samples,
    )
}
fn values(samples: &[AtomicU64], frames: usize) -> Vec<f64> {
    samples[..frames]
        .iter()
        .map(|v| f64::from_bits(v.load(Ordering::Relaxed)))
        .collect()
}

#[test]
fn event_at_end_waits_until_next_block_and_ramps_cross_blocks() {
    for in_place in [false, true] {
        let (mut engine, mut control, mut meter, samples) = pipeline(in_place, 16, 16);
        assert_eq!(
            control
                .submit(request(1, ApplyAt::Frame(3), 2.0, 2), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(
            control
                .submit(request(2, ApplyAt::Frame(8), 0.0, 0), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(engine.render(4).unwrap().segments, 2);
        engine.render(4).unwrap();
        assert_eq!(
            values(&samples, 8),
            vec![0.25, 0.25, 0.25, 0.375, 0.5, 0.5, 0.5, 0.5]
        );
        assert_eq!(control.poll_applied().unwrap().effective_frame, 3);
        assert!(control.poll_applied().is_none());
        engine.render(1).unwrap();
        assert_eq!(values(&samples, 9)[8], 0.0);
        assert_eq!(control.poll_applied().unwrap().effective_frame, 8);
        let pre_gain = meter.latest().unwrap();
        assert_eq!(pre_gain.peak, 0.25);
        assert_eq!(pre_gain.rms, 0.25);
    }
}

#[test]
fn ingress_rejects_stale_bad_typed_unordered_and_full_requests() {
    let (mut engine, mut control, _, _) = pipeline(true, 2, 2);
    let base = request(1, ApplyAt::Frame(4), 1.0, 0);
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    plan_revision: REV + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleRevision
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    timeline_epoch: EPOCH + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleEpoch
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Bool(true),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Float(f64::NAN),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(request(1, ApplyAt::Frame(2048), 1.0, 0), 0)
            .code,
        ReplyCode::InvalidTime
    );
    assert_eq!(control.submit(base, 0).code, ReplyCode::Accepted);
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(3), 1.0, 0), 0)
            .code,
        ReplyCode::OutOfOrder
    );
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 2.0, 0), 0)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(
        control
            .submit(request(3, ApplyAt::Frame(4), 3.0, 0), 0)
            .code,
        ReplyCode::QueueFull
    );
    engine.render(8).unwrap();
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    assert_eq!(control.poll_applied().unwrap().request_id, 2);
}

#[test]
fn reply_backpressure_never_stalls_audio_or_loses_an_applied_ack() {
    let (mut engine, mut control, _, samples) = pipeline(true, 4, 1);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    engine.render(4).unwrap();
    // Applied reply is still queued. A second command is accepted but deferred.
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 0.0, 0), 4)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(engine.render(4).unwrap().applied_events, 0);
    assert_eq!(engine.timeline(), 8);
    assert_eq!(values(&samples, 8), vec![0.5; 8]);
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    engine.render(1).unwrap();
    let ack = control.poll_applied().unwrap();
    assert_eq!(ack.request_id, 2);
    assert_eq!(ack.code, ReplyCode::AppliedLate);
    assert_eq!(ack.effective_frame, 8);
    assert_eq!(values(&samples, 9)[8], 0.0);
}

#[test]
fn work_budget_bounds_segmentation_and_defers_excess_events() {
    let (mut engine, mut control, _, _) = pipeline(true, 1, 8);
    for frame in 1..=3 {
        control.submit(request(frame, ApplyAt::Frame(frame), frame as f64, 0), 0);
    }
    let report = engine.render(8).unwrap();
    assert_eq!(report.applied_events, 1);
    assert_eq!(report.segments, 2);
    engine.render(8).unwrap();
    control.poll_applied().unwrap();
    let late = control.poll_applied().unwrap();
    assert_eq!(late.code, ReplyCode::AppliedLate);
    assert_eq!(late.effective_frame, 8);
}

#[test]
fn builtins_render_without_allocation_or_deallocation_even_with_full_telemetry() {
    let (mut engine, mut control, mut meter, _) = pipeline(true, 16, 16);
    for frame in 1..=8 {
        control.submit(request(frame, ApplyAt::Frame(frame), 2.0, 16), 0);
    }
    COUNTS.with(|n| n.set((0, 0)));
    TRACK.with(|t| t.set(true));
    let result = engine.render(16);
    TRACK.with(|t| t.set(false));
    result.unwrap();
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert!(meter.latest().is_some());
    engine.render(16).unwrap();
    assert!(meter.latest().unwrap().dropped_snapshots > 0);
}

#[test]
fn prepared_tables_cannot_be_replaced_by_same_shaped_foreign_tables() {
    let (_, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let (_, foreign) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let resources = RtResources::<f64>::new(vec![ProcessorInstance::new(GAIN, Gain)]).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec {
            processor: GAIN,
            io,
        }],
        &resources,
        &params,
        config(8),
    )
    .unwrap();
    assert!(matches!(
        Engine::new(plan, resources, foreign),
        Err(RuntimeError::ForeignParameters)
    ));
}

#[test]
fn decoding_happens_before_spsc_and_applied_reply_roundtrips() {
    let (mut engine, mut control, _, _) = pipeline(false, 8, 8);
    let mut wire = Vec::new();
    write_parameter(&mut wire, request(10, ApplyAt::NextBlock, 0.5, 0)).unwrap();
    let decoded = read_parameter(&mut wire.as_slice()).unwrap();
    let accepted = control.submit(decoded, engine.timeline());
    assert_eq!(accepted.code, ReplyCode::Accepted);
    engine.render(1).unwrap();
    let applied = control.poll_applied().unwrap();
    let mut response = Vec::new();
    write_reply(&mut response, applied).unwrap();
    assert_eq!(
        read_reply(&mut response.as_slice()).unwrap().code,
        ReplyCode::Applied
    );
}

#[test]
fn engine_and_queue_endpoints_move_to_distinct_threads_without_shared_arena() {
    let (engine, mut control, _, samples) = pipeline(true, 8, 8);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut engine = engine;
        engine.render(8).unwrap();
        // Test synchronization occurs OUTSIDE render, not in a processor.
        ready_tx.send(()).unwrap();
        continue_rx.recv().unwrap();
        engine.render(8).unwrap();
        engine
    });
    ready_rx.recv().unwrap();
    assert_eq!(control.poll_applied().unwrap().effective_frame, 0);
    control.submit(request(2, ApplyAt::Frame(8), 0.0, 0), 8);
    continue_tx.send(()).unwrap();
    let engine = join.join().unwrap();
    assert_eq!(engine.timeline(), 16);
    assert_eq!(values(&samples, 16), [vec![0.5; 8], vec![0.0; 8]].concat());
    // Owned resources return to the non-RT owner before destruction.
    drop(engine.into_parts());
}
