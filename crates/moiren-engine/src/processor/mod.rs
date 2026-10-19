//! Safe DSP contracts. Rust memory safety does not enforce realtime deadlines.
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo, ReadPorts},
    control::ProcessParameters,
    sample::ProcessingSample,
};
use moiren_signal::ErasedBinding;
use thiserror::Error;

pub mod builtin;

pub(crate) use builtin::PassThrough;
pub use builtin::{
    Bus, ChannelMapper, ChannelMapperSettings, Compressor, CompressorSettings, Delay,
    DelaySettings, Eq, EqSettings, Gain, Gate, GateSettings, Limiter, LimiterSettings, Pan, Sum,
};
pub use builtin::{ChannelMode, StripConfig};

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
        signals: crate::signal::ProcessSignals<'_>,
    );
    fn signal_bindings(&self) -> &[ErasedBinding] {
        &[]
    }
}

/// An observer has no writable audio capability, not even an unused output.
pub trait RtObserver<S: ProcessingSample>: Send {
    fn state_type_id(&self) -> std::any::TypeId
    where
        Self: 'static,
    {
        std::any::TypeId::of::<Self>()
    }
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError>;
    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        signals: crate::signal::ProcessSignals<'_>,
    );
    fn signal_bindings(&self) -> &[ErasedBinding] {
        &[]
    }
}

impl<S: ProcessingSample, O: RtObserver<S> + ?Sized> RtObserver<S> for Box<O> {
    fn state_type_id(&self) -> std::any::TypeId
    where
        Self: 'static,
    {
        (**self).state_type_id()
    }
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        (**self).validate_inputs(io)
    }
    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        signals: crate::signal::ProcessSignals<'_>,
    ) {
        (**self).observe(ctx, inputs, signals)
    }
    fn signal_bindings(&self) -> &[ErasedBinding] {
        (**self).signal_bindings()
    }
}

pub struct Observer<O>(pub O);
impl<S: ProcessingSample, O: RtObserver<S>> RtProcessor<S> for Observer<O> {
    fn state_type_id(&self) -> std::any::TypeId
    where
        Self: 'static,
    {
        self.0.state_type_id()
    }
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
        signals: crate::signal::ProcessSignals<'_>,
    ) {
        if let ProcessIo::ReadOnly { inputs } = io {
            self.0.observe(ctx, inputs, signals);
        }
    }
    fn signal_bindings(&self) -> &[ErasedBinding] {
        self.0.signal_bindings()
    }
}

/// Shared input contract of the single-input observers (level meter,
/// spectrum analyzer): exactly one prepared input of the channel width the
/// observer was prepared for.
pub(crate) fn validate_single_input(
    channels: usize,
    io: &PreparedIo,
) -> Result<(), ProcessorError> {
    if io.input_count() == 1 && io.input_channels(0) == Some(channels) {
        Ok(())
    } else {
        Err(ProcessorError::InvalidIo)
    }
}

/// The frame-timeline prefix every exact-window observer publishes. The
/// window types keep these as their leading public fields; `From` impls
/// construct them from a header so the field set is written once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimelineHeader {
    pub timeline_epoch: u64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub sample_rate: f64,
}

/// Latched timeline facts an exact-window observer carries between blocks.
/// Any change — a new publisher activation generation, a timeline epoch
/// change, a frame gap (rewind or skip), or a sample-rate change — means the
/// accumulated history no longer describes contiguous source frames and the
/// partial window must restart. Shared by the demand-gated level meter and
/// spectrum analyzer so the reset predicate cannot drift between them.
pub(crate) struct WindowContinuity {
    generation: u64,
    timeline_epoch: u64,
    next_frame: u64,
    sample_rate: f64,
}

impl WindowContinuity {
    pub(crate) const fn new() -> Self {
        Self {
            generation: 0,
            timeline_epoch: 0,
            next_frame: 0,
            sample_rate: 0.0,
        }
    }

    /// Demand gate plus continuity check, in the order every exact-window
    /// observer needs: `None` skips the block (no scope, inactive publisher
    /// or no subscriber), `Some(true)` reports a continuity break so the
    /// caller restarts its accumulator, `Some(false)` continues the partial
    /// window. The latched facts are updated on every measured block, so
    /// inactive blocks do not fabricate a gap.
    pub(crate) fn begin_window<P: moiren_signal::Shape>(
        &mut self,
        signals: &mut crate::signal::ProcessSignals<'_>,
        publisher: &moiren_signal::PublisherHandle<P>,
        ctx: &ProcessContext,
    ) -> Option<bool> {
        let demand = signals.demand(publisher).ok()?;
        if !demand.active {
            return None;
        }
        let restart = self.generation != demand.generation
            || self.timeline_epoch != ctx.timeline_epoch
            || self.next_frame != ctx.timeline_start
            || self.sample_rate != ctx.processing_sr;
        self.generation = demand.generation;
        self.timeline_epoch = ctx.timeline_epoch;
        self.sample_rate = ctx.processing_sr;
        Some(restart)
    }

    /// Latched activation generation; test seam for asserting that a
    /// resubscribe advanced it without an intervening inactive render.
    #[cfg(test)]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Header for a window ending at `frame_end` (exclusive), stamped with
    /// the latched epoch and rate of the block that measured it.
    pub(crate) fn window(&self, frame_start: u64, frame_end: u64) -> TimelineHeader {
        TimelineHeader {
            timeline_epoch: self.timeline_epoch,
            frame_start,
            frame_end,
            sample_rate: self.sample_rate,
        }
    }

    /// Latch the block end; the next measured block must start exactly here.
    pub(crate) fn finish_block(&mut self, ctx: &ProcessContext) {
        self.next_frame = ctx.timeline_start + ctx.frames as u64;
    }
}
