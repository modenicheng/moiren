use moiren_core::protocol::{ApplyAt, ParamValue, ParameterKey, ParameterRequest, ProcessorId};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSink, SinkAdapter, SourceAdapter},
    buffer::{AudioBlock, BufferArena, BufferSlotLayout, PortAccess},
    control::{ControlPort, parameter_channel},
    meter::{MeterReader, level_meter},
    processor::{Gain, Observer, ProcessContext},
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(super) const REV: u64 = 7;
pub(super) const EPOCH: u64 = 3;
pub(super) const SOURCE: ProcessorId = ProcessorId(10);
pub(super) const GAIN: ProcessorId = ProcessorId(20);
pub(super) const METER: ProcessorId = ProcessorId(30);
pub(super) const SINK: ProcessorId = ProcessorId(40);

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
pub(super) fn request(id: u64, at: ApplyAt, value: f64, ramp_frames: u32) -> ParameterRequest {
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
pub(super) fn config(events: usize) -> EngineConfig {
    EngineConfig {
        processing_sr: 48000.0,
        max_block_frames: 16,
        max_events_per_block: events,
    }
}
pub(super) fn pipeline(
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
pub(super) fn values(samples: &[AtomicU64], frames: usize) -> Vec<f64> {
    samples[..frames]
        .iter()
        .map(|v| f64::from_bits(v.load(Ordering::Relaxed)))
        .collect()
}
