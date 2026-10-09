//! Safe DSP contracts. Rust memory safety does not enforce realtime deadlines.
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo, ReadPorts},
    control::ProcessParameters,
    sample::ProcessingSample,
};
use thiserror::Error;

pub mod builtin;

pub use builtin::{Bus, Compressor, CompressorSettings, Gain, Pan, Sum};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy)]
pub struct ProcessContext {
    pub timeline_epoch: u64,
    pub timeline_start: u64,
    pub frames: usize,
    pub processing_sr: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProcessorError {
    #[error("prepared IO does not match the processor's port contract")]
    InvalidIo,
    #[error("a required parameter is missing from the runtime table")]
    MissingParameter,
    #[error("parameter domain exceeds the processor's supported range")]
    InvalidParameterDomain,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorRole {
    Transform,
    Source,
    Sink,
    Observer,
}

pub trait RtProcessor<S: ProcessingSample>: Send {
    /// Concrete persistent DSP identity, forwarded by type-erasing wrappers.
    fn state_type_id(&self) -> std::any::TypeId
    where
        Self: 'static,
    {
        std::any::TypeId::of::<Self>()
    }
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Transform
    }
    fn latency_frames(&self) -> u32 {
        0
    }
    /// Non-RT prepare hook; process never queries or changes its port schema.
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError>;
    fn validate_parameters(&self, _params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        Ok(())
    }
    /// No allocation, locks, syscalls, logging, IPC or unbounded work. Do not
    /// retain views/pointers. This is a realtime contract, not an unsafe trait.
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
    );
}

/// An observer has no writable audio capability, not even an unused output.
pub trait RtObserver<S: ProcessingSample>: Send {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError>;
    fn observe(&mut self, ctx: &ProcessContext, inputs: ReadPorts<'_, S>);
}
pub struct Observer<O>(pub O);
impl<S: ProcessingSample, O: RtObserver<S>> RtProcessor<S> for Observer<O> {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Observer
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::ReadOnly || io.output_count() != 0 {
            return Err(ProcessorError::InvalidIo);
        }
        self.0.validate_inputs(io)
    }
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        _params: ProcessParameters<'_>,
    ) {
        if let ProcessIo::ReadOnly { inputs } = io {
            self.0.observe(ctx, inputs);
        }
    }
}
