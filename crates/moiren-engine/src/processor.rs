//! Safe DSP contracts. Rust memory safety does not enforce realtime deadlines.
use moiren_core::protocol::{ParameterId, ParameterKey, ParamValue, ProcessorId};
use crate::{buffer::{IoMode, PreparedIo, ProcessIo, ReadPorts},
    control::{ParamDomain, ParamSpec, ProcessParameters}, sample::ProcessingSample};

#[derive(Debug, Clone, Copy)]
pub struct ProcessContext {
    pub timeline_epoch: u64,
    pub timeline_start: u64,
    pub frames: usize,
    pub processing_sr: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorError { InvalidIo, MissingParameter }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorRole { Transform, Source, Sink, Observer }

pub trait RtProcessor<S: ProcessingSample>: Send {
    fn role(&self) -> ProcessorRole { ProcessorRole::Transform }
    fn latency_frames(&self) -> u32 { 0 }
    /// Non-RT prepare hook; process never queries or changes its port schema.
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError>;
    fn validate_parameters(&self, _params: ProcessParameters<'_>) -> Result<(), ProcessorError> { Ok(()) }
    /// No allocation, locks, syscalls, logging, IPC or unbounded work. Do not
    /// retain views/pointers. This is a realtime contract, not an unsafe trait.
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, params: ProcessParameters<'_>);
}

/// An observer has no writable audio capability, not even an unused output.
pub trait RtObserver<S: ProcessingSample>: Send {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError>;
    fn observe(&mut self, ctx: &ProcessContext, inputs: ReadPorts<'_, S>);
}
pub struct Observer<O>(pub O);
impl<S: ProcessingSample, O: RtObserver<S>> RtProcessor<S> for Observer<O> {
    fn role(&self) -> ProcessorRole { ProcessorRole::Observer }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::ReadOnly || io.output_count() != 0 { return Err(ProcessorError::InvalidIo); }
        self.0.validate_inputs(io)
    }
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, _params: ProcessParameters<'_>) {
        if let ProcessIo::ReadOnly { inputs } = io { self.0.observe(ctx, inputs); }
    }
}

pub struct Gain;
impl Gain {
    pub const LEVEL: ParameterId = ParameterId(0);
    pub fn parameter(id: ProcessorId, initial: f64) -> ParamSpec {
        ParamSpec { key: ParameterKey { processor: id, parameter: Self::LEVEL },
            domain: ParamDomain::Float { min: 0.0, max: 16.0 }, initial: ParamValue::Float(initial) }
    }
}
impl<S: ProcessingSample> RtProcessor<S> for Gain {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() != 1 || io.output_count() != 1
            || io.input_channels(0).is_none() || io.input_channels(0) != io.output_channels(0) {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }
    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        params.float(Self::LEVEL).map(|_| ()).ok_or(ProcessorError::MissingParameter)
    }
    fn process(&mut self, _ctx: &ProcessContext, io: ProcessIo<'_, S>, params: ProcessParameters<'_>) {
        let gain = params.float(Self::LEVEL).expect("validated gain parameter");
        match io {
            ProcessIo::Separate { inputs, mut outputs } => {
                let input = inputs.get(0).expect("validated main input");
                let mut output = outputs.get_mut(0).expect("validated main output");
                for (src, dst) in input.channels().zip(output.channels_mut()) {
                    for (i, (src, dst)) in src.iter().zip(dst.iter_mut()).enumerate() {
                        *dst = S::from_f64(src.to_f64() * gain.sample(i));
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated main pair");
                for channel in block.channels_mut() {
                    for (i, sample) in channel.iter_mut().enumerate() {
                        *sample = S::from_f64(sample.to_f64() * gain.sample(i));
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("gain requires an output"),
        }
    }
}

/// Minimal bus kernel. Send matrices and gains can later be folded into this
/// loop; it never requires one audio allocation per logical input port.
pub struct Sum;
impl<S: ProcessingSample> RtProcessor<S> for Sum {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        let output = io.output_channels(0).ok_or(ProcessorError::InvalidIo)?;
        if io.mode() != IoMode::Separate || io.output_count() != 1
            || io.input_ports().any(|(_, channels)| channels != output) {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }
    fn process(&mut self, _ctx: &ProcessContext, io: ProcessIo<'_, S>, _params: ProcessParameters<'_>) {
        if let ProcessIo::Separate { inputs, mut outputs } = io {
            let mut output = outputs.get_mut(0).expect("validated bus output");
            for (_, input) in inputs.iter() {
                for (src, dst) in input.channels().zip(output.channels_mut()) {
                    for (src, dst) in src.iter().zip(dst.iter_mut()) {
                        *dst = S::from_f64(dst.to_f64() + src.to_f64());
                    }
                }
            }
        }
    }
}
