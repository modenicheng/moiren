//! Prepared external-port routing, after the compiler expands real slot kernels.
use super::RackLayout;
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::ProcessParameters,
    processor::{ProcessContext, ProcessorError, RtProcessor},
    sample::ProcessingSample,
    signal::ProcessSignals,
};

/// Owns only immutable routing metadata. Preparation/disposal stays control-side;
/// processing borrows port views and never allocates, drops storage or locks.
pub(crate) struct RackRouting {
    inputs: Box<[usize]>,
    outputs: Box<[usize]>,
    sources: Box<[Option<u16>]>,
}

impl RackRouting {
    pub(crate) fn new(layout: &RackLayout, outputs: &[usize]) -> Self {
        let mut sources = vec![None; outputs.len()];
        let inputs = layout
            .routes
            .iter()
            .enumerate()
            .map(|(index, route)| {
                sources[usize::from(route.output_port)] = Some(index as u16);
                outputs[usize::from(route.output_port)]
            })
            .collect();
        Self {
            inputs,
            outputs: outputs.into(),
            sources: sources.into_boxed_slice(),
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for RackRouting {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() != IoMode::Separate
            || io.input_count() != self.inputs.len()
            || io.output_count() != self.outputs.len()
            || self
                .inputs
                .iter()
                .enumerate()
                .any(|(port, &width)| io.input_channels(port as u16) != Some(width))
            || self
                .outputs
                .iter()
                .enumerate()
                .any(|(port, &width)| io.output_channels(port as u16) != Some(width))
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn process(
        &mut self,
        _: &ProcessContext,
        io: ProcessIo<'_, S>,
        _: ProcessParameters<'_>,
        _: ProcessSignals<'_>,
    ) {
        let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        else {
            unreachable!("rack routing requires separate IO")
        };
        for (port, mut output) in outputs.iter_mut() {
            if let Some(source) = self.sources[usize::from(port)] {
                let input = inputs.get(source).expect("validated rack route input");
                for (source, destination) in input.channels().zip(output.channels_mut()) {
                    destination.copy_from_slice(source);
                }
            } else {
                output.clear();
            }
        }
    }
}
