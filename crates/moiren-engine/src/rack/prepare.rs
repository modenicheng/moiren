use super::RackProcessor;
use crate::{
    control::ParamSpec,
    processor::{
        ChannelMapper, Compressor, Delay, DelaySettings, Eq, Gain, Gate, Limiter, Pan,
        ProcessorError, RtProcessor,
    },
    sample::ProcessingSample,
};
use moiren_core::protocol::ProcessorId;

/// A prepared slot owns its kernel and the schema used for plan-local control.
pub type PreparedRackSlot<S> = (Box<dyn RtProcessor<S>>, Vec<ParamSpec>);

/// Prepare one leaf kernel and its parameter schema on the control side.
///
/// The caller assigns a unique runtime ID, then binds each active slot's input
/// to the previous output in the existing execution plan. Skip bypassed slots
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
    let parameters = processor.checked_parameters(channels, id)?;
    let kernel: Box<dyn RtProcessor<S>> = match processor {
        RackProcessor::Gain { .. } => Box::new(Gain),
        RackProcessor::Pan { .. } => Box::new(Pan),
        RackProcessor::Compressor(_) => Box::new(Compressor::new()),
        RackProcessor::Eq(_) => Box::new(Eq::new()),
        RackProcessor::Gate(_) => Box::new(Gate::new()),
        RackProcessor::Limiter(_) => Box::new(Limiter::new()),
        RackProcessor::Delay(settings) => {
            validate_delay_storage(*settings, channels, sample_rate)?;
            Box::new(Delay::new(channels, *settings, sample_rate)?)
        }
        RackProcessor::ChannelMapper(settings) => {
            Box::new(ChannelMapper::new(channels, settings.clone())?)
        }
    };
    Ok((kernel, parameters))
}

fn validate_delay_storage(
    settings: DelaySettings,
    channels: usize,
    sample_rate: f64,
) -> Result<(), ProcessorError> {
    // Match Delay's capacity calculation before its float-to-usize cast and
    // allocation. Finite sample rates can still overflow frame or byte counts.
    let frames = (settings.max_delay_ms * sample_rate / 1_000.0).ceil();
    if !frames.is_finite() || frames >= usize::MAX as f64 {
        return Err(ProcessorError::InvalidIo);
    }
    let bytes = (frames as usize)
        .checked_add(1)
        .and_then(|capacity| capacity.checked_mul(channels))
        .and_then(|samples| samples.checked_mul(std::mem::size_of::<f64>()));
    if bytes.is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(ProcessorError::InvalidIo);
    }
    Ok(())
}
