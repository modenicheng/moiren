use super::RackProcessor;
use crate::{
    control::ParamSpec,
    processor::{
        ChannelMapper, Compressor, Delay, Eq, Gain, Gate, Limiter, Pan, ProcessorError, RtProcessor,
    },
    sample::ProcessingSample,
};
use moiren_core::protocol::ProcessorId;

/// A prepared slot owns its kernel and the schema used for plan-local control.
pub type PreparedRackSlot<S> = (Box<dyn RtProcessor<S>>, Vec<ParamSpec>);

/// Prepare one leaf kernel and its parameter schema on the control side.
///
/// The caller assigns a unique runtime ID, then binds each active slot's input
/// to the previous output on its resolved route. `channels` is the actual slot
/// input width, not the enclosing node width. Skip bypassed slots
/// before calling this function; stable slot IDs remain configuration identity.
/// ChannelMapper requires separate buffers, so expansion must respect each
/// kernel's IO contract rather than forcing every slot to process in place.
///
/// Returns [`ProcessorError::InvalidIo`] for an unsupported layout, invalid
/// sample rate or unrepresentable delay storage, and
/// [`ProcessorError::InvalidParameterDomain`] for invalid numeric settings.
/// Persistent DSP state, including delay rings, is allocated here, never while
/// rendering. The caller owns runtime memory budgets and control-side disposal.
pub fn prepare_slot<S: ProcessingSample>(
    processor: &RackProcessor,
    channels: usize,
    sample_rate: f64,
    id: ProcessorId,
) -> Result<PreparedRackSlot<S>, ProcessorError> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(ProcessorError::InvalidIo);
    }
    // checked_parameters stays the single precheck (channel contracts and
    // initial-value domains) so this entry cannot accept what the control-side
    // schema query rejects; each fixed-schema kernel's `prepare` then builds
    // the kernel and its schema from the same source.
    let parameters = processor.checked_parameters(channels, id)?;
    let kernel: Box<dyn RtProcessor<S>> = match processor {
        RackProcessor::Gain { .. } => Box::new(Gain),
        RackProcessor::Pan { .. } => Box::new(Pan),
        RackProcessor::Compressor(settings) => Box::new(Compressor::prepare(id, *settings).0),
        RackProcessor::Eq(settings) => Box::new(Eq::prepare(id, *settings).0),
        RackProcessor::Gate(settings) => Box::new(Gate::prepare(id, *settings).0),
        RackProcessor::Limiter(settings) => Box::new(Limiter::prepare(id, *settings).0),
        RackProcessor::Delay(settings) => {
            Box::new(Delay::prepare(channels, *settings, sample_rate, id)?.0)
        }
        RackProcessor::ChannelMapper(settings) => Box::new(ChannelMapper::with_layout(
            channels,
            processor.output_channels(channels)?,
            settings.clone(),
        )?),
    };
    Ok((kernel, parameters))
}
