use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{IoMode, PreparedIo, ProcessIo},
    control::ProcessParameters,
    sample::ProcessingSample,
};

/// Minimal bus kernel. Send matrices and gains can later be folded into this
/// loop; it never requires one audio allocation per logical input port.
pub struct Sum;
impl<S: ProcessingSample> RtProcessor<S> for Sum {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        let output = io.output_channels(0).ok_or(ProcessorError::InvalidIo)?;
        if io.mode() != IoMode::Separate
            || io.output_count() != 1
            || io.input_ports().any(|(_, channels)| channels != output)
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
    ) {
        if let ProcessIo::Separate {
            inputs,
            mut outputs,
        } = io
        {
            let mut output = outputs.get_mut(0).expect("validated bus output");
            for (_, input) in inputs.iter() {
                for (src, dst) in input.channels().zip(output.channels_mut()) {
                    for (src, dst) in src.iter().zip(dst.iter_mut()) {
                        *dst = S::from_f64(dst.to_f64() + src.to_f64());
                    }
                }
            }
        }
    }
}
