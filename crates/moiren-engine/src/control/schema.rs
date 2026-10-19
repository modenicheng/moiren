use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamDomain {
    Float { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    Bool,
    Enum { variants: u32 },
}
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub key: ParameterKey,
    pub domain: ParamDomain,
    pub initial: ParamValue,
}
impl ParamSpec {
    pub(crate) fn accepts(&self, value: ParamValue, ramp: u32) -> bool {
        match (self.domain, value) {
            (ParamDomain::Float { min, max }, ParamValue::Float(v)) => {
                min.is_finite()
                    && max.is_finite()
                    && min <= max
                    && v.is_finite()
                    && v >= min
                    && v <= max
            }
            (ParamDomain::Int { min, max }, ParamValue::Int(v)) => {
                ramp == 0 && min <= v && v <= max
            }
            (ParamDomain::Bool, ParamValue::Bool(_)) => ramp == 0,
            (ParamDomain::Enum { variants }, ParamValue::Enum(v)) => ramp == 0 && v < variants,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ControlError {
    #[error("zero capacity/horizon or an initial value outside its domain")]
    InvalidConfiguration,
    #[error("two parameters declare the same processor/parameter key")]
    DuplicateParameter,
}

/// Single source for the "a kernel's own settings are valid" idiom: every
/// spec must accept its initial value at zero ramp horizon. Project
/// validation and rack expansion share it so the two cannot drift.
pub(crate) fn specs_accept_initials(specs: &[ParamSpec]) -> bool {
    specs.iter().all(|spec| spec.accepts(spec.initial, 0))
}

/// Single source for the "one recorded value targets a valid parameter"
/// idiom: some spec must carry this parameter id and accept the value.
pub(crate) fn accepts_parameter(
    specs: &[ParamSpec],
    parameter: ParameterId,
    value: ParamValue,
) -> bool {
    specs
        .iter()
        .any(|spec| spec.key.parameter == parameter && spec.accepts(value, 0))
}
