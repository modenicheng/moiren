//! Field-level Rack intent, shared by native previews and service transactions.
use super::CompressorParameter;
use moiren_engine::{
    processor::ChannelMapperSettings,
    rack::{RackProcessor, RackSettings, RackSlot},
};

mod parameter;
pub use parameter::RackParameter;

#[derive(Debug, Clone, PartialEq)]
pub enum RackEdit {
    SetParameter {
        slot: u32,
        parameter: RackParameter,
        value: f64,
    },
    SetMono {
        slot: u32,
        enabled: bool,
    },
    /// Invert the latest candidate value, not a UI snapshot's captured value.
    ToggleMono {
        slot: u32,
    },
    SetSwap {
        slot: u32,
        enabled: bool,
    },
    ToggleSwap {
        slot: u32,
    },
    SetMatrixWeight {
        slot: u32,
        input: usize,
        output: usize,
        value: f64,
    },
    MapperPreset {
        slot: u32,
        swap: bool,
    },
    Bypass {
        slot: u32,
        bypass: bool,
    },
    ToggleBypass {
        slot: u32,
    },
    Move {
        slot: u32,
        direction: i8,
    },
    Remove {
        slot: u32,
    },
    Add {
        route: usize,
        processor: RackProcessor,
    },
    Clear,
}

impl RackEdit {
    /// Control-side preview only: no mutation, runtime work or identity check.
    /// Submission repeats this against the latest candidate after checking epoch.
    pub fn preview(
        &self,
        settings: &RackSettings,
        inputs: &[usize],
        outputs: &[usize],
    ) -> Result<RackSettings, String> {
        let layout = settings
            .resolve_layout(inputs, outputs)
            .ok_or("Rack settings are invalid for the actual stream ports")?;
        let mut next = settings.clone();
        match *self {
            Self::SetParameter {
                slot,
                parameter,
                value,
            } => {
                parameter.set(&mut find_slot(&mut next, slot)?.processor, value)?;
            }
            Self::SetMono { slot, enabled } => {
                let RackProcessor::Pan { mono, .. } = &mut find_slot(&mut next, slot)?.processor
                else {
                    return Err("Mono only applies to a Rack Balance stage".into());
                };
                *mono = enabled;
            }
            Self::ToggleMono { slot } => {
                let RackProcessor::Pan { mono, .. } = &mut find_slot(&mut next, slot)?.processor
                else {
                    return Err("Mono only applies to a Rack Balance stage".into());
                };
                *mono = !*mono;
            }
            Self::SetSwap { slot, enabled } => {
                let RackProcessor::Pan { swap, .. } = &mut find_slot(&mut next, slot)?.processor
                else {
                    return Err("Swap only applies to a Rack Balance stage".into());
                };
                *swap = enabled;
            }
            Self::ToggleSwap { slot } => {
                let RackProcessor::Pan { swap, .. } = &mut find_slot(&mut next, slot)?.processor
                else {
                    return Err("Swap only applies to a Rack Balance stage".into());
                };
                *swap = !*swap;
            }
            Self::SetMatrixWeight {
                slot,
                input,
                output,
                value,
            } => {
                let width = layout
                    .slots
                    .get(&slot)
                    .ok_or("The Rack stage does not exist")?
                    .input_channels;
                let stage = find_slot(&mut next, slot)?;
                // Configured mapper width differs from effective width while bypassed.
                let height = stage
                    .processor
                    .output_channels(width)
                    .map_err(|e| e.to_string())?;
                let RackProcessor::ChannelMapper(mapper) = &mut stage.processor else {
                    return Err("Matrix weights only apply to a Rack Mapper stage".into());
                };
                if input >= width || output >= height || !value.is_finite() {
                    return Err("Rack matrix coordinates or weight are invalid".into());
                }
                if mapper.matrix.is_empty() {
                    mapper.matrix = (0..height)
                        .map(|row| {
                            let mut weights = vec![0.; width];
                            weights[mapper.mapping.get(row).copied().unwrap_or(row)] = 1.;
                            weights
                        })
                        .collect();
                    mapper.mapping.clear();
                }
                mapper.matrix[output][input] = value;
            }
            Self::MapperPreset { slot, swap } => {
                let width = layout
                    .slots
                    .get(&slot)
                    .ok_or("The Rack stage does not exist")?
                    .input_channels;
                let stage = find_slot(&mut next, slot)?;
                let height = stage
                    .processor
                    .output_channels(width)
                    .map_err(|e| e.to_string())?;
                let RackProcessor::ChannelMapper(mapper) = &mut stage.processor else {
                    return Err("Mapper presets only apply to a Rack Mapper stage".into());
                };
                if width != height || (swap && width < 2) {
                    return Err(
                        "Mapper presets need equal widths; Swap 1/2 needs at least two channels"
                            .into(),
                    );
                }
                *mapper = ChannelMapperSettings::identity(width);
                if swap {
                    mapper.mapping.swap(0, 1);
                }
            }
            Self::Bypass { slot, bypass } => find_slot(&mut next, slot)?.bypass = bypass,
            Self::ToggleBypass { slot } => {
                let stage = find_slot(&mut next, slot)?;
                stage.bypass = !stage.bypass;
            }
            Self::Move { slot, direction } => {
                let route = layout
                    .slots
                    .get(&slot)
                    .ok_or("The Rack stage does not exist")?
                    .route;
                let indices: Vec<_> = next
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, stage)| layout.slots[&stage.id].route == route)
                    .map(|(index, _)| index)
                    .collect();
                let position = indices
                    .iter()
                    .position(|&index| next.slots[index].id == slot)
                    .ok_or("The Rack stage does not exist")?;
                let target = match direction {
                    -1 => position.checked_sub(1),
                    1 => position.checked_add(1),
                    _ => None,
                }
                .and_then(|position| indices.get(position))
                .copied()
                .ok_or("The Rack stage cannot move in that direction")?;
                // Routes contain membership, not execution order. Keep stable IDs.
                next.slots.swap(indices[position], target);
            }
            Self::Remove { slot } => {
                let index = next
                    .slots
                    .iter()
                    .position(|stage| stage.id == slot)
                    .ok_or("The Rack stage does not exist")?;
                next.slots.remove(index);
                for route in &mut next.routes {
                    route.slots.retain(|id| *id != slot);
                }
            }
            Self::Add {
                route,
                ref processor,
            } => {
                if route >= layout.routes.len() || next.slots.len() >= RackSettings::MAX_SLOTS {
                    return Err("The Rack route is missing or its stage limit is reached".into());
                }
                let id = next
                    .slots
                    .iter()
                    .map(|slot| slot.id)
                    .max()
                    .map_or(Some(0), |id| id.checked_add(1))
                    .or_else(|| {
                        (0..=RackSettings::MAX_SLOTS as u32)
                            .find(|id| !next.slots.iter().any(|slot| slot.id == *id))
                    })
                    .ok_or("The Rack stage identity space is exhausted")?;
                next.slots.push(RackSlot {
                    id,
                    bypass: false,
                    processor: processor.clone(),
                });
                if !next.routes.is_empty() {
                    next.routes[route].slots.push(id);
                }
            }
            Self::Clear => {
                next.slots.clear();
                for route in &mut next.routes {
                    route.slots.clear();
                }
            }
        }
        if !next.valid_layout(inputs, outputs) {
            return Err(
                "Rack parameters or routing are invalid for the actual stream ports".into(),
            );
        }
        Ok(next)
    }

    pub(super) fn invalidates_epoch(&self) -> bool {
        matches!(self, Self::Add { .. } | Self::Remove { .. } | Self::Clear)
    }

    pub(super) fn undo_key(&self) -> Option<String> {
        match self {
            // Clicks are discrete actions, unlike drags or exact assignments.
            Self::ToggleBypass { .. } | Self::ToggleMono { .. } | Self::ToggleSwap { .. } => None,
            Self::SetParameter {
                slot, parameter, ..
            } => Some(format!("{slot}:{parameter:?}")),
            Self::SetMono { slot, .. } => Some(format!("{slot}:mono")),
            Self::SetSwap { slot, .. } => Some(format!("{slot}:swap")),
            Self::SetMatrixWeight {
                slot,
                input,
                output,
                ..
            } => Some(format!("{slot}:matrix:{input}:{output}")),
            _ => None,
        }
    }
}

fn find_slot(settings: &mut RackSettings, id: u32) -> Result<&mut RackSlot, String> {
    settings
        .slots
        .iter_mut()
        .find(|slot| slot.id == id)
        .ok_or_else(|| "The Rack stage does not exist".into())
}
