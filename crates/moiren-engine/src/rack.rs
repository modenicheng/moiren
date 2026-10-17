//! Control-side configuration for ordered processor chains across Rack streams.
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
use std::collections::{BTreeMap, BTreeSet};

mod prepare;
pub use prepare::{PreparedRackSlot, prepare_slot};
mod routing;
pub(crate) use routing::RackRouting;

/// Slots always execute in vector order. Explicit routes partition them into
/// nonbranching chains; complex split/merge routing belongs in LogicalGraph.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RackSettings {
    pub slots: Vec<RackSlot>,
    /// Empty retains the legacy input 0 -> all slots -> output 0 chain.
    /// Other outputs are silent, not implicitly copied from the main stream.
    pub routes: Vec<RackRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackRoute {
    /// Indices into the current Rack port lists, not persistent PortIds.
    pub input_port: u16,
    pub output_port: u16,
    /// Stable slot membership only; order is defined by RackSettings::slots.
    pub slots: Vec<u32>,
}

/// Allocating control-side resolution; never queried on the audio thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RackLayout {
    pub slots: BTreeMap<u32, RackSlotLayout>,
    pub routes: Vec<RackRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RackSlotLayout {
    pub input_channels: usize,
    /// Bypassed slots preserve input width even for a width-changing mapper.
    pub output_channels: usize,
    pub route: usize,
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
        self.valid_layout(&[channels], &[channels])
    }

    pub fn valid_layout(&self, inputs: &[usize], outputs: &[usize]) -> bool {
        self.resolve_layout(inputs, outputs).is_some()
    }

    /// Resolve each stable slot's actual IO for schema queries and reuse checks.
    /// A caller must compare both widths before reusing a prepared kernel: an
    /// upstream mapper edit can change IO without changing this slot's settings.
    /// Bypassed settings are still checked at their actual incoming width.
    pub fn resolve_layout(&self, inputs: &[usize], outputs: &[usize]) -> Option<RackLayout> {
        if inputs.is_empty()
            || outputs.is_empty()
            || inputs.len() > u16::MAX as usize + 1
            || outputs.len() > u16::MAX as usize + 1
            || !inputs
                .iter()
                .chain(outputs)
                .all(|&width| valid_channels(width))
            || self.slots.len() > Self::MAX_SLOTS
        {
            return None;
        }
        let ids: BTreeSet<_> = self.slots.iter().map(|slot| slot.id).collect();
        if ids.len() != self.slots.len() {
            return None;
        }
        let routes = if self.routes.is_empty() {
            vec![RackRoute {
                input_port: 0,
                output_port: 0,
                slots: self.slots.iter().map(|slot| slot.id).collect(),
            }]
        } else {
            self.routes.clone()
        };
        let mut used_inputs = BTreeSet::new();
        let mut used_outputs = BTreeSet::new();
        let mut assigned = BTreeSet::new();
        let mut layout = RackLayout {
            slots: BTreeMap::new(),
            routes: Vec::new(),
        };
        for (index, route) in routes.into_iter().enumerate() {
            let mut channels = *inputs.get(usize::from(route.input_port))?;
            let expected = *outputs.get(usize::from(route.output_port))?;
            if !used_inputs.insert(route.input_port) || !used_outputs.insert(route.output_port) {
                return None;
            }
            for id in &route.slots {
                if !ids.contains(id) || !assigned.insert(*id) {
                    return None;
                }
            }
            let mut ordered = Vec::new();
            for slot in self
                .slots
                .iter()
                .filter(|slot| route.slots.contains(&slot.id))
            {
                slot.processor
                    .checked_parameters(channels, ProcessorId(0))
                    .ok()?;
                let output_channels = if slot.bypass {
                    channels
                } else {
                    slot.processor.output_channels(channels).ok()?
                };
                layout.slots.insert(
                    slot.id,
                    RackSlotLayout {
                        input_channels: channels,
                        output_channels,
                        route: index,
                    },
                );
                channels = output_channels;
                ordered.push(slot.id);
            }
            if channels != expected {
                return None;
            }
            layout.routes.push(RackRoute {
                slots: ordered,
                ..route
            });
        }
        if assigned.len() != self.slots.len() {
            return None;
        }
        Some(layout)
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
    /// Infer a leaf's output width from its static settings and actual input.
    /// Only an explicit ChannelMapper can change width; no implicit conversion.
    pub fn output_channels(&self, inputs: usize) -> Result<usize, ProcessorError> {
        if !valid_channels(inputs) {
            return Err(ProcessorError::InvalidIo);
        }
        let outputs = if let Self::ChannelMapper(settings) = self {
            let outputs = if !settings.matrix.is_empty() {
                settings.matrix.len()
            } else if !settings.mapping.is_empty() {
                settings.mapping.len()
            } else {
                inputs
            };
            if !settings.valid_layout(inputs, outputs) {
                return Err(ProcessorError::InvalidIo);
            }
            outputs
        } else {
            inputs
        };
        Ok(outputs)
    }

    /// Non-RT schema query: validates settings without preparing DSP storage.
    /// This allocates parameter metadata; `id` belongs to the caller's plan,
    /// not to the stable rack slot identity.
    pub fn checked_parameters(
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
                if !settings.valid_layout(channels, self.output_channels(channels)?) {
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
