use super::runtime::ParamState;
use super::{FloatRamp, ParamDomain, ParamSpec, ParameterRuntime};
use moiren_core::protocol::{ParamValue, ParameterId, ProcessorId};
use std::sync::Arc;

pub(crate) struct ParameterBindings {
    specs: Arc<[ParamSpec]>,
    slots: Box<[(ParameterId, usize)]>,
}
#[derive(Clone, Copy)]
pub struct ProcessParameters<'a> {
    states: &'a [ParamState],
    slots: &'a [(ParameterId, usize)],
    specs: &'a [ParamSpec],
}
impl ProcessParameters<'_> {
    /// Schema metadata for non-RT processor preparation. Binding the correct
    /// type alone does not guarantee that later automation stays in DSP range.
    pub fn domain(&self, id: ParameterId) -> Option<ParamDomain> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        Some(self.specs[*slot].domain)
    }
    pub fn float(&self, id: ParameterId) -> Option<FloatRamp> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        match self.states[*slot] {
            ParamState::Float(ramp) => Some(ramp),
            _ => None,
        }
    }
    pub fn discrete(&self, id: ParameterId) -> Option<ParamValue> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        match self.states[*slot] {
            ParamState::Discrete(value) => Some(value),
            _ => None,
        }
    }
}
impl ParameterRuntime {
    pub(crate) fn bindings(&self, id: ProcessorId) -> ParameterBindings {
        let slots = self
            .specs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.key.processor == id)
            .map(|(slot, s)| (s.key.parameter, slot))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ParameterBindings {
            specs: Arc::clone(&self.specs),
            slots,
        }
    }
    pub(crate) fn owns(&self, bindings: &ParameterBindings) -> bool {
        Arc::ptr_eq(&self.specs, &bindings.specs)
    }
    pub(crate) fn view<'a>(&'a self, bindings: &'a ParameterBindings) -> ProcessParameters<'a> {
        ProcessParameters {
            states: &self.states,
            slots: &bindings.slots,
            specs: &self.specs,
        }
    }
}
