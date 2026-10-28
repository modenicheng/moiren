//! Field-level intent for independent processors. Only candidate settings change;
//! the drivers rebuild prepared plans through the existing structural edit path.
use super::{BuiltinState, GraphSnapshot, NodeId, NodeKind, RackParameter};
use moiren_core::graph::Node;
use moiren_engine::{
    processor::{ChannelMapperSettings, EqSettings},
    rack::RackProcessor,
};

pub(super) fn set_mapper_weight(
    next: &mut GraphSnapshot,
    node: NodeId,
    input: usize,
    output: usize,
    weight: f64,
) -> Result<(), String> {
    edit_mapper(next, node, |mapper, inputs, outputs| {
        if input >= inputs || output >= outputs || !weight.is_finite() {
            return Err("Mapper matrix coordinates or weight are invalid".into());
        }
        if mapper.matrix.is_empty() {
            // The current routing was validated against the actual ports before
            // indexing. Empty routing is identity only on equal-width ports.
            mapper.matrix = (0..outputs)
                .map(|row| {
                    let mut weights = vec![0.; inputs];
                    weights[mapper.mapping.get(row).copied().unwrap_or(row)] = 1.;
                    weights
                })
                .collect();
            mapper.mapping.clear();
        }
        mapper.matrix[output][input] = weight;
        Ok(())
    })
}

pub(super) fn mapper_preset(
    next: &mut GraphSnapshot,
    node: NodeId,
    swap: bool,
) -> Result<(), String> {
    edit_mapper(next, node, |mapper, inputs, outputs| {
        if inputs != outputs || (swap && inputs < 2) {
            return Err(
                "Mapper presets need equal widths; Swap 1/2 needs at least two channels".into(),
            );
        }
        *mapper = ChannelMapperSettings::identity(inputs);
        if swap {
            mapper.mapping.swap(0, 1);
        }
        Ok(())
    })
}

fn edit_mapper(
    next: &mut GraphSnapshot,
    node: NodeId,
    edit: impl FnOnce(&mut ChannelMapperSettings, usize, usize) -> Result<(), String>,
) -> Result<(), String> {
    edit_builtin(next, node, |builtin, topology| {
        if topology.kind() != NodeKind::ChannelMapper {
            return Err("Mapper edits only apply to Channel Mapper nodes".into());
        }
        let ([input], [output]) = (topology.inputs(), topology.outputs()) else {
            return Err("Mapper edits require one input and one output stream".into());
        };
        let BuiltinState::ChannelMapper(mut mapper) = builtin else {
            return Err("Channel Mapper settings are missing".into());
        };
        if !mapper.valid_layout(input.channels(), output.channels()) {
            return Err("Channel Mapper settings are invalid for the actual stream ports".into());
        }
        edit(&mut mapper, input.channels(), output.channels())?;
        Ok(BuiltinState::ChannelMapper(mapper))
    })
}

pub(super) fn set_builtin_parameter(
    next: &mut GraphSnapshot,
    node: NodeId,
    parameter: RackParameter,
    value: f64,
) -> Result<(), String> {
    edit_builtin(next, node, |builtin, _| {
        let mut processor = match builtin {
            BuiltinState::Eq(settings) => RackProcessor::Eq(settings),
            BuiltinState::Gate(settings) => RackProcessor::Gate(settings),
            BuiltinState::Limiter(settings) => RackProcessor::Limiter(settings),
            BuiltinState::Delay(settings) => RackProcessor::Delay(settings),
            _ => {
                return Err("Builtin scalar edits only apply to Eq, Gate, Limiter or Delay".into());
            }
        };
        // Reuse the Rack field mapping without reconstructing defaults (notably
        // Delay's prepared storage bound). Engine validation follows the edit.
        parameter.set(&mut processor, value)?;
        Ok(match processor {
            RackProcessor::Eq(settings) => BuiltinState::Eq(settings),
            RackProcessor::Gate(settings) => BuiltinState::Gate(settings),
            RackProcessor::Limiter(settings) => BuiltinState::Limiter(settings),
            RackProcessor::Delay(settings) => BuiltinState::Delay(settings),
            _ => unreachable!("setting a scalar does not change the processor kind"),
        })
    })
}

pub(super) fn set_eq_band(
    next: &mut GraphSnapshot,
    node: NodeId,
    frequency_hz: f64,
    gain_db: f64,
) -> Result<(), String> {
    edit_eq(next, node, |settings| {
        settings.frequency_hz = frequency_hz;
        settings.gain_db = gain_db;
    })
}

pub(super) fn adjust_eq_q(
    next: &mut GraphSnapshot,
    node: NodeId,
    factor: f64,
) -> Result<(), String> {
    if !factor.is_finite() || factor <= 0. {
        return Err("EQ Q adjustment factor must be finite and positive".into());
    }
    edit_eq(next, node, |settings| {
        // The current Q is validated first. A finite positive factor can still
        // overflow/underflow the product; saturate to the requested Q bounds.
        settings.q = (settings.q * factor).clamp(0.1, 20.);
    })
}

fn edit_eq(
    next: &mut GraphSnapshot,
    node: NodeId,
    edit: impl FnOnce(&mut EqSettings),
) -> Result<(), String> {
    edit_builtin(next, node, |builtin, _| {
        let BuiltinState::Eq(mut settings) = builtin else {
            return Err("EQ band and Q edits only apply to Eq nodes".into());
        };
        edit(&mut settings);
        Ok(BuiltinState::Eq(settings))
    })
}

fn edit_builtin(
    next: &mut GraphSnapshot,
    node: NodeId,
    edit: impl FnOnce(BuiltinState, &Node) -> Result<BuiltinState, String>,
) -> Result<(), String> {
    let topology = next
        .graph
        .get_node(node)
        .map_err(|error| error.to_string())?;
    let state = next
        .nodes
        .iter_mut()
        .find(|state| state.id == node)
        .ok_or("The node does not exist")?;
    state.builtin.validate_for_node(topology)?;
    let updated = edit(state.builtin.resolved(topology.kind()), topology)?;
    updated.validate_for_node(topology)?;
    state.builtin = updated;
    Ok(())
}

#[cfg(test)]
mod tests;
