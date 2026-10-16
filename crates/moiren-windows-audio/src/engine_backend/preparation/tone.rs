use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    processor::ProcessContext,
};
pub(super) struct Tone {
    phase: f64,
    frequency: f64,
}
impl Tone {
    pub(super) fn new(frequency: f64) -> Self {
        Self {
            phase: 0.0,
            frequency,
        }
    }
}
impl RtAudioSource<f32> for Tone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        context: &ProcessContext,
        mut output: AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        let advance = std::f64::consts::TAU * self.frequency / context.processing_sr;
        for frame in 0..context.frames {
            let value = self.phase.sin() as f32;
            output.channel_mut(0)[frame] = value;
            output.channel_mut(1)[frame] = value;
            self.phase = (self.phase + advance) % std::f64::consts::TAU;
        }
        BoundaryReport {
            transferred_frames: context.frames,
            ..BoundaryReport::default()
        }
    }
}
