//! Control-side configuration for a linear, channel-preserving processor rack.
//!
//! The compiler expands active slots in vector order into its existing execution
//! plan. Slot IDs survive reordering and bypass edits; they are configuration
//! identities, not runtime [`ProcessorId`]s. A rack owns neither a second graph
//! nor an opaque realtime processor.

use crate::{
    control::ParamSpec,
    processor::{
        ChannelMapperSettings, Compressor, CompressorSettings, Delay, DelaySettings, Eq,
        EqSettings, Gain, Gate, GateSettings, Limiter, LimiterSettings, Pan, ProcessorError,
        builtin::MAX_CHANNELS,
    },
};
use moiren_core::protocol::ProcessorId;
use serde::{Deserialize, Serialize};

mod prepare;
pub use prepare::prepare_slot;

/// An empty rack is a direct connection; otherwise each active slot consumes
/// the preceding slot's output with the same channel width.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RackSettings {
    pub slots: Vec<RackSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackSlot {
    /// Stable within this rack, including while the slot is bypassed.
    pub id: u32,
    /// The compiler omits this slot's operation and parameter specifications.
    #[serde(default)]
    pub bypass: bool,
    pub processor: RackProcessor,
}

/// Only leaf transforms are representable, so racks cannot contain racks,
/// sources, sinks, observers or a second editable graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// Tuple settings variants use Serde's external representation consistently,
// e.g. `{ "compressor": { ... } }`; internal tagging only supports struct
// variants and would force a less useful public enum shape.
#[serde(rename_all = "snake_case")]
pub enum RackProcessor {
    Gain {
        initial: f64,
    },
    Pan {
        initial: f64,
        mono: bool,
        swap: bool,
    },
    Compressor(CompressorSettings),
    Eq(EqSettings),
    Gate(GateSettings),
    Limiter(LimiterSettings),
    Delay(DelaySettings),
    ChannelMapper(ChannelMapperSettings),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackPreset {
    pub version: u32,
    pub name: String,
    pub rack: RackSettings,
}

impl RackSettings {
    pub const MAX_SLOTS: usize = 64;

    /// Validate on the control side before expansion. Bypassed settings are
    /// checked too, so enabling a saved slot cannot reveal invalid settings.
    pub fn valid_for(&self, channels: usize) -> bool {
        valid_channels(channels)
            && self.slots.len() <= Self::MAX_SLOTS
            && self.slots.iter().enumerate().all(|(index, slot)| {
                !self.slots[..index]
                    .iter()
                    .any(|previous| previous.id == slot.id)
                    && slot
                        .processor
                        .checked_parameters(channels, ProcessorId(0))
                        .is_ok()
            })
    }
}

impl RackPreset {
    /// The version describes the serialized configuration, not runtime state.
    pub const VERSION: u32 = 1;
    pub const MAX_NAME_BYTES: usize = 256;

    /// Deserialization alone does not validate a preset for a target layout.
    pub fn valid_for(&self, channels: usize) -> bool {
        self.version == Self::VERSION
            && !self.name.trim().is_empty()
            && self.name.len() <= Self::MAX_NAME_BYTES
            && !self.name.chars().any(char::is_control)
            && self.rack.valid_for(channels)
    }
}

fn valid_channels(channels: usize) -> bool {
    (1..=MAX_CHANNELS).contains(&channels)
}

impl RackProcessor {
    fn checked_parameters(
        &self,
        channels: usize,
        id: ProcessorId,
    ) -> Result<Vec<ParamSpec>, ProcessorError> {
        if !valid_channels(channels) {
            return Err(ProcessorError::InvalidIo);
        }
        let parameters = match self {
            Self::Gain { initial } => vec![Gain::parameter(id, *initial)],
            Self::Pan {
                initial,
                mono,
                swap,
            } => {
                if channels != 2 {
                    return Err(ProcessorError::InvalidIo);
                }
                Pan::parameters(id, *initial, *mono, *swap).to_vec()
            }
            Self::Compressor(settings) => Compressor::parameters(id, *settings).to_vec(),
            Self::Eq(settings) => Eq::parameters(id, *settings).to_vec(),
            Self::Gate(settings) => Gate::parameters(id, *settings).to_vec(),
            Self::Limiter(settings) => Limiter::parameters(id, *settings).to_vec(),
            Self::Delay(settings) => {
                // This is the same prepared length limit enforced by Delay.
                // Checking it here avoids allocating a ring just to validate.
                if !settings.max_delay_ms.is_finite()
                    || !(0.0..=60_000.0).contains(&settings.max_delay_ms)
                {
                    return Err(ProcessorError::InvalidParameterDomain);
                }
                Delay::parameters(id, *settings).to_vec()
            }
            Self::ChannelMapper(settings) => {
                if !settings.valid_for(channels) {
                    return Err(ProcessorError::InvalidIo);
                }
                Vec::new()
            }
        };
        // Use the kernels' own schemas to keep rack validation and parameter
        // preparation aligned when a builtin's supported ranges change.
        if parameters.iter().any(|spec| !spec.accepts(spec.initial, 0)) {
            return Err(ProcessorError::InvalidParameterDomain);
        }
        Ok(parameters)
    }
}

#[cfg(test)]
mod tests;
