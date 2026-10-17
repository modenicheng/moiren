use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::ProcessParameters,
    sample::ProcessingSample,
};

/// Utility nodes use the same copy kernel until their control-side observer or
/// recorder binding is attached. Keeping this as a processor preserves graph
/// ordering and the one-in/one-out contract without doing side effects on RT.
pub struct PassThrough;

impl<S: ProcessingSample> RtProcessor<S> for PassThrough {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1
            && io.output_count() == 1
            && io.input_channels(0) == io.output_channels(0)
        {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        _params: ProcessParameters<'_>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated utility input");
                let mut output = outputs.get_mut(0).expect("validated utility output");
                for (source, destination) in input.channels().zip(output.channels_mut()) {
                    destination.copy_from_slice(source);
                }
            }
            ProcessIo::InPlace { .. } => {}
            ProcessIo::ReadOnly { .. } => unreachable!("utility requires an output"),
        }
    }
}
