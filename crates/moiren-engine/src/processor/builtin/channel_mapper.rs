use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::ProcessParameters,
    sample::ProcessingSample,
};

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChannelMapperSettings {
    /// One source index per output. Empty settings mean equal-width identity.
    pub mapping: Vec<usize>,
    /// Explicit weighted downmix/upmix, one output row per source-width vector.
    /// Mutually exclusive with mapping; zero rows create silent channels.
    pub matrix: Vec<Vec<f64>>,
}

impl ChannelMapperSettings {
    pub fn identity(channels: usize) -> Self {
        Self {
            mapping: (0..channels).collect(),
            matrix: Vec::new(),
        }
    }

    pub fn valid_for(&self, channels: usize) -> bool {
        self.valid_layout(channels, channels)
    }
    pub fn valid_layout(&self, inputs: usize, outputs: usize) -> bool {
        if inputs == 0 || outputs == 0 || inputs > 256 || outputs > 256 {
            return false;
        }
        if !self.matrix.is_empty() {
            self.mapping.is_empty()
                && self.matrix.len() == outputs
                && self
                    .matrix
                    .iter()
                    .all(|row| row.len() == inputs && row.iter().all(|weight| weight.is_finite()))
        } else if self.mapping.is_empty() {
            inputs == outputs
        } else {
            self.mapping.len() == outputs && self.mapping.iter().all(|&source| source < inputs)
        }
    }
}

/// Static channel reorder/copy. It deliberately requires separate buffers:
/// arbitrary in-place permutations can overwrite a source still needed later.
pub struct ChannelMapper {
    inputs: usize,
    outputs: usize,
    matrix: Box<[f64]>,
}

impl ChannelMapper {
    pub fn new(channels: usize, settings: ChannelMapperSettings) -> Result<Self, ProcessorError> {
        Self::with_layout(channels, channels, settings)
    }

    pub fn with_layout(
        inputs: usize,
        outputs: usize,
        settings: ChannelMapperSettings,
    ) -> Result<Self, ProcessorError> {
        if !settings.valid_layout(inputs, outputs) {
            return Err(ProcessorError::InvalidIo);
        }
        let mut matrix = vec![0.0; inputs * outputs];
        if settings.matrix.is_empty() {
            for output in 0..outputs {
                let source = settings.mapping.get(output).copied().unwrap_or(output);
                matrix[output * inputs + source] = 1.0;
            }
        } else {
            for (row, weights) in settings.matrix.iter().enumerate() {
                matrix[row * inputs..(row + 1) * inputs].copy_from_slice(weights);
            }
        }
        Ok(Self {
            inputs,
            outputs,
            matrix: matrix.into_boxed_slice(),
        })
    }
}

impl<S: ProcessingSample> RtProcessor<S> for ChannelMapper {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.inputs)
            || io.output_channels(0) != Some(self.outputs)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        _params: ProcessParameters<'_>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        else {
            unreachable!("channel mapper requires separate IO")
        };
        let input = inputs.get(0).expect("validated mapper input");
        let mut output = outputs.get_mut(0).expect("validated mapper output");
        for destination in 0..self.outputs {
            let target = output.channel_mut(destination);
            target.fill(S::ZERO);
            for source in 0..self.inputs {
                let weight = self.matrix[destination * self.inputs + source];
                if weight == 0.0 {
                    continue;
                }
                for (target, sample) in target.iter_mut().zip(input.channel(source)) {
                    *target = S::from_f64(target.to_f64() + sample.to_f64() * weight);
                }
            }
        }
    }
}
