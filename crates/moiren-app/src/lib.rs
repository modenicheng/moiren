//! Application ownership and offline IO orchestration.
#![forbid(unsafe_code)]

pub mod monitor;
pub mod monitor_cli;
pub mod render_cli;
pub mod service;
pub mod tone;

use moiren_core::protocol::{
    ApplyAt, ControlReply, ParamValue, ParameterKey, ParameterRequest, ProcessorId, ReplyCode,
};
use moiren_engine::{boundary::*, buffer::*, control::*, node::*, processor::Gain, runtime::*};
use thiserror::Error;

const INPUT: ProcessorId = ProcessorId(1);
const GAIN: ProcessorId = ProcessorId(2);
const OUTPUT: ProcessorId = ProcessorId(3);
const AUDIO_BYTE_BUDGET: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct AppConfig {
    pub channels: usize,
    pub processing_sr: f64,
    pub max_block_frames: usize,
    pub initial_gain: f64,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            channels: 2,
            processing_sr: 48_000.0,
            max_block_frames: 256,
            initial_gain: 0.5,
        }
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid app channels, sample rate, maximum block size or initial gain")]
    InvalidConfig,
    #[error("input/output slices must have equal lengths and contain complete frames")]
    InvalidSamples,
    #[error("gain request was rejected: {0:?}")]
    ParameterRejected(ReplyCode),
    #[error("parameter request counter overflowed")]
    RequestIdOverflow,
    #[error("offline audio bridge failed to transfer a complete requested block")]
    IncompleteTransfer,
    #[error(transparent)]
    Buffer(#[from] BufferError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessReport {
    pub frames: usize,
    pub blocks: usize,
    pub segments: usize,
}

/// First application session: software input -> InputNode -> in-place Gain ->
/// OutputNode -> software output. One slab and two preallocated audio bridges.
/// Construction, control calls and stop belong to the non-RT application owner.
/// This offline owner runs both ends synchronously; it is not a device scheduler.
pub struct OfflineApp {
    engine: Engine<f32>,
    control: ControlPort,
    input: AudioWriter<f32>,
    output: AudioReader<f32>,
    input_reports: BoundaryReader,
    output_reports: BoundaryReader,
    config: AppConfig,
    next_request: u64,
}
impl OfflineApp {
    pub fn new(config: AppConfig) -> Result<Self, AppError> {
        if config.channels == 0
            || !config.processing_sr.is_finite()
            || config.processing_sr <= 0.0
            || config.max_block_frames == 0
            || config.max_block_frames > u32::MAX as usize
            || !config.initial_gain.is_finite()
            || !(0.0..=16.0).contains(&config.initial_gain)
        {
            return Err(AppError::InvalidConfig);
        }
        let arena = BufferArena::<f32>::new(
            &[BufferSlotLayout {
                channels: config.channels,
                capacity_frames: config.max_block_frames,
            }],
            AUDIO_BYTE_BUDGET,
        )?;
        let slot = arena.slot(0).expect("one prepared audio slot");
        let specs = vec![
            OpSpec {
                processor: INPUT,
                io: arena.prepare_io(&[PortAccess::Write { port: 0, slot }])?,
            },
            OpSpec {
                processor: GAIN,
                io: arena.prepare_io(&[PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot,
                }])?,
            },
            OpSpec {
                processor: OUTPUT,
                io: arena.prepare_io(&[PortAccess::Read { port: 0, slot }])?,
            },
        ];
        let (input, source) =
            audio_bridge(config.channels, config.max_block_frames, AUDIO_BYTE_BUDGET)?;
        let (sink, output) =
            audio_bridge(config.channels, config.max_block_frames, AUDIO_BYTE_BUDGET)?;
        let (input_node, input_reports) = InputNode::new(
            InputConfig {
                source: InputSource::Software {
                    name: "offline input".into(),
                },
                channels: config.channels,
                device: DeviceOptions::default(),
            },
            source,
            16,
        )?;
        let (output_node, output_reports) = OutputNode::new(
            OutputConfig {
                target: OutputTarget::Software {
                    name: "offline output".into(),
                },
                channels: config.channels,
                device: DeviceOptions::default(),
            },
            sink,
            16,
        )?;
        let resources = RtResources::new(vec![
            ProcessorInstance::new(INPUT, input_node),
            ProcessorInstance::new(GAIN, Gain),
            ProcessorInstance::new(OUTPUT, output_node),
        ])?;
        let (control, parameters) = parameter_channel(
            vec![Gain::parameter(GAIN, config.initial_gain)],
            1,
            1,
            16,
            48_000,
        )?;
        let plan = ExecutionPlan::prepare(
            arena,
            specs,
            &resources,
            &parameters,
            EngineConfig {
                processing_sr: config.processing_sr,
                max_block_frames: config.max_block_frames,
                max_events_per_block: 16,
            },
        )?;
        Ok(Self {
            engine: Engine::new(plan, resources, parameters)?,
            control,
            input,
            output,
            input_reports,
            output_reports,
            config,
            next_request: 1,
        })
    }

    pub fn timeline(&self) -> u64 {
        self.engine.timeline()
    }

    /// Complete worker buffers are split into bounded variable engine blocks.
    /// All shape/timeline checks happen before samples are queued or consumed.
    pub fn process_interleaved(
        &mut self,
        input: &[f32],
        output: &mut [f32],
    ) -> Result<ProcessReport, AppError> {
        if input.len() != output.len() || !input.len().is_multiple_of(self.config.channels) {
            return Err(AppError::InvalidSamples);
        }
        let frames = input.len() / self.config.channels;
        self.timeline()
            .checked_add(frames as u64)
            .ok_or(RuntimeError::TimelineOverflow)?;
        let chunk_samples = self.config.max_block_frames * self.config.channels;
        let mut report = ProcessReport {
            frames,
            ..ProcessReport::default()
        };
        for (src, dst) in input
            .chunks(chunk_samples)
            .zip(output.chunks_mut(chunk_samples))
        {
            let frames = src.len() / self.config.channels;
            if self.input.write_interleaved(src)?.transferred_frames != frames {
                return Err(AppError::IncompleteTransfer);
            }
            let rendered = self.engine.render(frames)?;
            if self.output.read_interleaved(dst)?.transferred_frames != frames {
                return Err(AppError::IncompleteTransfer);
            }
            report.blocks += 1;
            report.segments += rendered.segments;
        }
        Ok(report)
    }

    pub fn set_gain(&mut self, value: f64, ramp_frames: u32) -> Result<ControlReply, AppError> {
        let request_id = self.next_request;
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or(AppError::RequestIdOverflow)?;
        let reply = self.control.submit(
            ParameterRequest {
                request_id,
                plan_revision: 1,
                timeline_epoch: self.engine.epoch(),
                target: ParameterKey {
                    processor: GAIN,
                    parameter: Gain::LEVEL,
                },
                at: ApplyAt::NextBlock,
                value: ParamValue::Float(value),
                ramp_frames,
            },
            self.engine.timeline(),
        );
        if reply.code != ReplyCode::Accepted {
            return Err(AppError::ParameterRejected(reply.code));
        }
        Ok(reply)
    }
    pub fn poll_applied(&mut self) -> Option<ControlReply> {
        self.control.poll_applied()
    }
    pub fn input_status(&mut self) -> Option<BoundarySnapshot> {
        self.input_reports.latest()
    }
    pub fn output_status(&mut self) -> Option<BoundarySnapshot> {
        self.output_reports.latest()
    }

    /// No streaming worker may still access these resources. Offline execution
    /// has returned synchronously, so the app owner can reclaim everything here.
    pub fn stop(self) {
        let Self {
            engine,
            control,
            input,
            output,
            input_reports,
            output_reports,
            ..
        } = self;
        drop(engine.into_parts());
        drop((control, input, output, input_reports, output_reports));
    }
}
