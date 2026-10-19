//! Shared support for the fixed-schema float builtin kernels (compressor,
//! gate, limiter, EQ). Every helper is a pure computation with no allocation
//! or locking, so each is equally safe on the control and render sides.
use super::super::ProcessorError;
use super::MAX_CHANNELS;
use crate::{
    buffer::PreparedIo,
    control::{ParamDomain, ParamSpec, ProcessParameters},
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

/// dB to linear amplitude for the linked detectors and gain stages.
pub(crate) fn db_to_gain(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// Guard against non-finite or non-positive rates reaching the DSP kernels;
/// the fallback keeps smoothing and filter math defined.
pub(crate) fn sanitize_sample_rate(sample_rate: f64) -> f64 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        48_000.0
    }
}

/// One-pole step from `current` toward `target` with time constant `ms`.
///
/// The coefficient is formed with `exp_m1` because `1 - exp(-x)` suffers
/// catastrophic cancellation for small `x`: the subtraction discards the low
/// bits of `exp`, so slow ramps lose precision and freeze entirely once
/// `exp(-x)` rounds to `1.0`. `-exp_m1(-x)` keeps full relative accuracy for
/// every representable smoothing constant.
pub(crate) fn smooth(current: f64, target: f64, ms: f64, sample_rate: f64) -> f64 {
    if ms == 0.0 {
        target
    } else {
        current + (target - current) * -(-1000.0 / (ms * sample_rate)).exp_m1()
    }
}

/// Positional float parameter table: parameter `i` pairs `DOMAINS[i]` with
/// `values[i]`, which is the layout every fixed-schema kernel publishes.
pub(crate) fn float_specs<const N: usize>(
    id: ProcessorId,
    domains: &[(f64, f64); N],
    values: [f64; N],
) -> [ParamSpec; N] {
    std::array::from_fn(|index| {
        let (min, max) = domains[index];
        ParamSpec {
            key: ParameterKey {
                processor: id,
                parameter: ParameterId(index as u32),
            },
            domain: ParamDomain::Float { min, max },
            initial: ParamValue::Float(values[index]),
        }
    })
}

/// Each parameter must be a Float domain that fits inside the kernel's
/// supported range, so runtime ramps can never leave the DSP contract.
pub(crate) fn validate_float_domains(
    params: &ProcessParameters<'_>,
    domains: &[(f64, f64)],
) -> Result<(), ProcessorError> {
    for (index, &(supported_min, supported_max)) in domains.iter().enumerate() {
        match params.domain(ParameterId(index as u32)) {
            Some(ParamDomain::Float { min, max })
                if min.is_finite()
                    && max.is_finite()
                    && min >= supported_min
                    && max <= supported_max
                    && min <= max => {}
            _ => return Err(ProcessorError::InvalidParameterDomain),
        }
    }
    Ok(())
}

/// Shared IO contract of the linked mono-width kernels (gate, limiter, EQ):
/// one input and one output of the same nonzero width within the fixed
/// built-in channel budget.
pub(crate) fn validate_linked_io(io: &PreparedIo) -> Result<(), ProcessorError> {
    let channels = io.input_channels(0).ok_or(ProcessorError::InvalidIo)?;
    if io.input_count() != 1
        || io.output_count() != 1
        || io.output_channels(0) != Some(channels)
        || channels == 0
        || channels > MAX_CHANNELS
    {
        return Err(ProcessorError::InvalidIo);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Across the typical smoothing constants the two coefficient forms agree:
    /// `-exp_m1(-x)` and `1 - exp(-x)` compute the same one-pole fraction.
    #[test]
    fn exp_m1_coefficient_equals_one_minus_exp_for_typical_time_constants() {
        for &ms in &[0.01, 0.1, 1.0, 10.0, 100.0, 1_000.0, 5_000.0] {
            for &sr in &[1_000.0, 48_000.0, 192_000.0] {
                let x = 1000.0_f64 / (ms * sr);
                let legacy = 1.0 - (-x).exp();
                let unified = -(-x).exp_m1();
                assert!(
                    (legacy - unified).abs() <= 1e-15,
                    "{legacy} != {unified} for {ms} ms at {sr} Hz"
                );
            }
        }
    }

    /// Beyond ~2^53 samples of time constant the subtracted form rounds the
    /// coefficient to exactly zero and the ramp freezes; `exp_m1` keeps every
    /// step representable. This is the drift the unified form fixes.
    #[test]
    fn exp_m1_coefficient_preserves_steps_the_subtraction_rounds_away() {
        let x = 2.0_f64.powi(-60);
        assert_eq!(1.0 - (-x).exp(), 0.0);
        assert!((-(-x).exp_m1() - x).abs() <= x * f64::EPSILON);
    }

    /// A slow ramp must track the exact geometric decay `1 - exp(-steps/samples)`
    /// instead of drifting as the per-step coefficient loses digits.
    #[test]
    fn slow_ramps_stay_on_the_geometric_decay() {
        let ms = 5_000.0;
        let sr = 192_000.0;
        let samples = ms * sr / 1_000.0;
        let steps = 240_000_u64;
        let mut current = 0.0;
        for _ in 0..steps {
            current = smooth(current, 1.0, ms, sr);
        }
        let expected = -(-(steps as f64) / samples).exp_m1();
        assert!((current - expected).abs() <= expected * 1e-9);
    }
}
