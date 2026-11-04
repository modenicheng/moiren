use moiren_core::protocol::{ApplyAt, ParamValue, ParameterKey, ParameterRequest, ProcessorId};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSink, SinkAdapter, SourceAdapter},
    buffer::{AudioBlock, BufferArena, BufferSlotLayout, PortAccess, PreparedIo, ReadPorts},
    control::{ControlPort, parameter_channel},
    processor::{Gain, Observer, ProcessContext, ProcessorError, RtObserver},
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
    signal::ProcessSignals,
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

/// Shared readout of [`PeakProbe`]: the peak magnitude and frame count the
/// observer saw, readable without touching the render thread.
pub(super) struct ProbeState {
    peak_bits: AtomicU64,
    frames: AtomicU64,
}
impl ProbeState {
    pub(super) fn peak(&self) -> f64 {
        f64::from_bits(self.peak_bits.load(Ordering::Relaxed))
    }
    pub(super) fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

/// Allocation-free single-input test observer: records the observed peak
/// magnitude and frame count through atomics, with no queue or signal bus.
pub(super) struct PeakProbe {
    state: Arc<ProbeState>,
}
impl PeakProbe {
    pub(super) fn new() -> (Self, Arc<ProbeState>) {
        let state = Arc::new(ProbeState {
            peak_bits: AtomicU64::new(0.0_f64.to_bits()),
            frames: AtomicU64::new(0),
        });
        (
            Self {
                state: Arc::clone(&state),
            },
            state,
        )
    }
}
impl RtObserver<f64> for PeakProbe {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0).is_some() {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, f64>,
        _signals: ProcessSignals<'_>,
    ) {
        let input = inputs.get(0).expect("validated observer input");
        let mut peak = 0.0_f64;
        for frame in 0..ctx.frames {
            for channel in 0..input.channel_count() {
                peak = peak.max(input.channel(channel)[frame].abs());
            }
        }
        // Bit patterns of non-negative finite f64 are monotonic, so fetch_max
        // over to_bits keeps the running peak without locks on the render path.
        self.state
            .peak_bits
            .fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.state
            .frames
            .fetch_add(ctx.frames as u64, Ordering::Relaxed);
    }
}

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
        max_block_frames: 16,
        max_events_per_block: events,
        ..super::common::engine_config()
    }
}
pub(super) fn pipeline(
    in_place: bool,
    events: usize,
    queue: usize,
) -> (Engine<f64>, ControlPort, Arc<ProbeState>, Arc<[AtomicU64]>) {
    let (control, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, queue, 1024).unwrap();
    let (probe, state) = PeakProbe::new();
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
        ProcessorInstance::new(METER, Observer(probe)),
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
        state,
        samples,
    )
}
pub(super) fn values(samples: &[AtomicU64], frames: usize) -> Vec<f64> {
    samples[..frames]
        .iter()
        .map(|v| f64::from_bits(v.load(Ordering::Relaxed)))
        .collect()
}
