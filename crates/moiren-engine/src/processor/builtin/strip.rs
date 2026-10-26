use super::Pan;
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    processor::{ProcessContext, ProcessorError, RtProcessor},
    sample::ProcessingSample,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChannelMode {
    #[default]
    Normal,
    Mono,
    Solo(usize),
    Swap,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripConfig {
    pub gain: f64,
    /// Stereo balance after the channel transform; retained but ignored for
    /// other layouts so changing a node's width does not lose its setting.
    pub balance: f64,
    pub muted: bool,
    pub mode: ChannelMode,
}

impl Default for StripConfig {
    fn default() -> Self {
        Self {
            gain: 1.,
            balance: 0.,
            muted: false,
            mode: ChannelMode::Normal,
        }
    }
}

impl StripConfig {
    pub fn valid(self, channels: usize) -> bool {
        self.gain.is_finite()
            && (0. ..=16.).contains(&self.gain)
            && self.balance.is_finite()
            && (-1. ..=1.).contains(&self.balance)
            && !matches!(self.mode, ChannelMode::Solo(channel) if channel >= channels)
    }
}

pub(crate) struct ChannelStrip {
    pub channels: usize,
    pub config: StripConfig,
}

impl<S: ProcessingSample> RtProcessor<S> for ChannelStrip {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.channels)
            || io.output_channels(0) != Some(self.channels)
            || !self.config.valid(self.channels)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn process(
        &mut self,
        _: &ProcessContext,
        io: ProcessIo<'_, S>,
        _: crate::control::ProcessParameters<'_>,
        _: crate::signal::ProcessSignals<'_>,
    ) {
        if let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        {
            let input = inputs.get(0).expect("prepared strip input");
            let mut output = outputs.get_mut(0).expect("prepared strip output");
            for (channel, samples) in output.channels_mut().enumerate() {
                // Share the send/Pan law, but apply it after this strip's
                // monitoring transform. PRE uses the same kernel at unity gain.
                let balance = if self.channels == 2 {
                    Pan::coefficient(channel, self.config.balance)
                } else {
                    1.
                };
                for (frame, sample) in samples.iter_mut().enumerate() {
                    let value = if self.config.muted {
                        0.
                    } else {
                        match self.config.mode {
                            ChannelMode::Normal => input.channel(channel)[frame].to_f64(),
                            ChannelMode::Swap => {
                                input.channel(self.channels - channel - 1)[frame].to_f64()
                            }
                            ChannelMode::Solo(selected) => input.channel(selected)[frame].to_f64(),
                            ChannelMode::Mono => input
                                .channels()
                                .map(|samples| samples[frame].to_f64() / self.channels as f64)
                                .sum(),
                        }
                    };
                    *sample = S::from_f64(value * balance * self.config.gain);
                }
            }
        }
    }
}
