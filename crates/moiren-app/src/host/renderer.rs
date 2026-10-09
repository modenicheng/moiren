use super::*;
use moiren_engine::{boundary::BoundaryReport, buffer::AudioBlockMut, processor::ProcessContext};
use std::sync::atomic::Ordering;

/// Portable render owner. Creation and destruction belong on control; render
/// performs no allocation, deallocation, locks or control-side graph work.
pub struct HostRenderer {
    engine: Engine<f32>,
    output: AudioReader<f32>,
    telemetry: Arc<Telemetry>,
}
impl HostRenderer {
    pub(super) fn new(
        engine: Engine<f32>,
        output: AudioReader<f32>,
        telemetry: Arc<Telemetry>,
    ) -> Self {
        Self {
            engine,
            output,
            telemetry,
        }
    }
    pub fn render_interleaved(
        &mut self,
        output: &mut [f32],
    ) -> Result<crate::ProcessReport, HostError> {
        let channels = self.output.channel_count();
        if !output.len().is_multiple_of(channels) {
            return Err(HostError::InvalidSamples);
        }
        let frames = output.len() / channels;
        self.engine
            .timeline()
            .checked_add(frames as u64)
            .ok_or(moiren_engine::runtime::RuntimeError::TimelineOverflow)?;
        let mut report = crate::ProcessReport {
            frames,
            ..Default::default()
        };
        let mut peak = 0.0_f32;
        for block in output.chunks_mut(channels * self.engine.config().max_block_frames) {
            let rendered = self.engine.render(block.len() / channels)?;
            if self.output.read_interleaved(block)?.transferred_frames != block.len() / channels {
                return Err(HostError::IncompleteTransfer);
            }
            for &value in block.iter() {
                peak = peak.max(value.abs());
            }
            report.blocks += 1;
            report.segments += rendered.segments;
        }
        self.telemetry
            .timeline
            .store(self.engine.timeline(), Ordering::Release);
        self.telemetry.peak.store(peak.to_bits(), Ordering::Release);
        self.telemetry
            .blocks
            .fetch_add(report.blocks as u64, Ordering::Release);
        Ok(report)
    }
    /// Non-RT handoff to a backend renderer; return these parts to
    /// `AudioHost::finish_parts` after that renderer joins.
    pub fn into_parts(self) -> (Engine<f32>, AudioReader<f32>) {
        (self.engine, self.output)
    }
}

/// A single concrete type for both real sources and reuse placeholders is
/// essential: the engine validates concrete TypeId before transferring state.
pub(super) struct HostSource {
    pub source: Option<Box<dyn RtAudioSource<f32>>>,
    pub gate: SourceGate,
    pub channels: usize,
}
impl RtAudioSource<f32> for HostSource {
    fn channel_count(&self) -> usize {
        self.channels
    }
    fn read(&mut self, ctx: &ProcessContext, output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        let frames = output.frames();
        if self.gate.is_available()
            && let Some(source) = &mut self.source
        {
            return source.read(ctx, output);
        }
        // SourceAdapter has cleared the entire destination already.
        BoundaryReport {
            transferred_frames: frames,
            ..Default::default()
        }
    }
}
