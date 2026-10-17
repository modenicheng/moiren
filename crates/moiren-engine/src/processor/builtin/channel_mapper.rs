use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::ProcessParameters,
    sample::ProcessingSample,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMapperSettings {
    /// One source channel index for each output channel. The current graph
    /// model keeps input/output width equal, but mappings may reorder or copy.
    pub mapping: Vec<usize>,
}

impl ChannelMapperSettings {
    pub fn identity(channels: usize) -> Self {
        Self {
            mapping: (0..channels).collect(),
        }
    }

    pub fn valid_for(&self, channels: usize) -> bool {
        self.mapping.len() == channels && self.mapping.iter().all(|&source| source < channels)
    }
}

impl Default for ChannelMapperSettings {
    fn default() -> Self {
        Self {
            mapping: Vec::new(),
        }
    }
}

/// Static channel reorder/copy. It deliberately requires separate buffers:
/// arbitrary in-place permutations can overwrite a source still needed later.
pub struct ChannelMapper {
    mapping: Box<[usize]>,
}

impl ChannelMapper {
    pub fn new(channels: usize, settings: ChannelMapperSettings) -> Result<Self, ProcessorError> {
        let mapping = if settings.mapping.is_empty() {
            ChannelMapperSettings::identity(channels).mapping
        } else {
            settings.mapping
        };
        let valid = mapping.len() == channels && mapping.iter().all(|&source| source < channels);
        if !valid {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(Self {
            mapping: mapping.into_boxed_slice(),
        })
    }
}

impl<S: ProcessingSample> RtProcessor<S> for ChannelMapper {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.mapping.len())
            || io.output_channels(0) != Some(self.mapping.len())
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
        for (destination, &source) in self.mapping.iter().enumerate() {
            output
                .channel_mut(destination)
                .copy_from_slice(input.channel(source));
        }
    }
}
