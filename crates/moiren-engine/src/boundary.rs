//! Device-independent streaming contracts; no COM object or driver buffer lease
//! crosses into the graph. Prepare/stop/destruction belong to backend owners.
use crate::{buffer::{AudioBlock, AudioBlockMut, IoMode, PreparedIo, ProcessIo},
    control::ProcessParameters, processor::{ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureIntent { PassiveTap, RoutedInput }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockRole { Master, Follower }
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundaryReport {
    pub transferred_frames: usize,
    pub discontinuity: bool,
    pub xruns: u64,
}
/// Must write a valid prefix and return immediately. The adapter supplies
/// initialized silence, including any unfilled tail. No external pointers escape.
pub trait RtAudioSource<S: ProcessingSample>: Send {
    fn channel_count(&self) -> usize;
    fn read(&mut self, ctx: &ProcessContext, output: AudioBlockMut<'_, S>) -> BoundaryReport;
}
/// Must consume/copy synchronously or enqueue into its OWN preallocated bridge.
/// It cannot enqueue AudioBlock references into an asynchronous recorder/device.
pub trait RtAudioSink<S: ProcessingSample>: Send {
    fn channel_count(&self) -> usize;
    fn write(&mut self, ctx: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport;
}
pub struct SourceAdapter<T>(pub T);
pub struct SinkAdapter<T>(pub T);
impl<S: ProcessingSample, T: RtAudioSource<S>> RtProcessor<S> for SourceAdapter<T> {
    fn role(&self) -> ProcessorRole { ProcessorRole::Source }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::Separate && io.input_count() == 0 && io.output_count() == 1
            && io.output_channels(0) == Some(self.0.channel_count()) { Ok(()) }
        else { Err(ProcessorError::InvalidIo) }
    }
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, _params: ProcessParameters<'_>) {
        if let ProcessIo::Separate { mut outputs, .. } = io {
            let output = outputs.get_mut(0).expect("validated source output");
            let _report = self.0.read(ctx, output);
        }
    }
}
impl<S: ProcessingSample, T: RtAudioSink<S>> RtProcessor<S> for SinkAdapter<T> {
    fn role(&self) -> ProcessorRole { ProcessorRole::Sink }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::ReadOnly && io.input_count() == 1
            && io.input_channels(0) == Some(self.0.channel_count()) { Ok(()) }
        else { Err(ProcessorError::InvalidIo) }
    }
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, _params: ProcessParameters<'_>) {
        if let ProcessIo::ReadOnly { inputs } = io {
            let input = inputs.get(0).expect("validated sink input");
            let _report = self.0.write(ctx, input);
        }
    }
}

/// Deterministic fake source for offline examples and backend-independent tests.
pub struct ConstantSource { pub channels: usize, pub value: f64 }
impl<S: ProcessingSample> RtAudioSource<S> for ConstantSource {
    fn channel_count(&self) -> usize { self.channels }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        for channel in output.channels_mut() { channel.fill(S::from_f64(self.value)); }
        BoundaryReport { transferred_frames: ctx.frames, ..BoundaryReport::default() }
    }
}
