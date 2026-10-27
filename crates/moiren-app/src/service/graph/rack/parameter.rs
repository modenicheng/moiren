use super::{CompressorParameter, RackProcessor};

/// Shared Rack/independent processor field identity, always in engine units:
/// Gain is linear, Pan is -1..1, EqFrequency is Hz, EqQ is dimensionless,
/// EqGain/GateThreshold/GateRange/LimitInput/LimitThreshold are dB (GateRange
/// is nonpositive attenuation). Timing fields are ms; DelayFeedback is a signed
/// ratio and DelayMix is a 0..1 ratio, not percent. Compressor fields retain
/// their existing engine units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RackParameter {
    Gain,
    Pan,
    Compressor(CompressorParameter),
    EqFrequency,
    EqGain,
    EqQ,
    GateThreshold,
    GateRange,
    GateAttack,
    GateHold,
    GateRelease,
    LimitInput,
    LimitThreshold,
    LimitRelease,
    DelayTime,
    DelayFeedback,
    DelayMix,
}

impl RackParameter {
    pub(in crate::service::graph) fn set(
        self,
        processor: &mut RackProcessor,
        value: f64,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("Rack parameter values must be finite".into());
        }
        let target = match (self, processor) {
            (Self::Gain, RackProcessor::Gain { initial }) => initial,
            (Self::Pan, RackProcessor::Pan { initial, .. }) => initial,
            (Self::EqFrequency, RackProcessor::Eq(s)) => &mut s.frequency_hz,
            (Self::EqGain, RackProcessor::Eq(s)) => &mut s.gain_db,
            (Self::EqQ, RackProcessor::Eq(s)) => &mut s.q,
            (Self::GateThreshold, RackProcessor::Gate(s)) => &mut s.threshold_db,
            (Self::GateRange, RackProcessor::Gate(s)) => &mut s.range_db,
            (Self::GateAttack, RackProcessor::Gate(s)) => &mut s.attack_ms,
            (Self::GateHold, RackProcessor::Gate(s)) => &mut s.hold_ms,
            (Self::GateRelease, RackProcessor::Gate(s)) => &mut s.release_ms,
            (Self::LimitInput, RackProcessor::Limiter(s)) => &mut s.input_gain_db,
            (Self::LimitThreshold, RackProcessor::Limiter(s)) => &mut s.threshold_db,
            (Self::LimitRelease, RackProcessor::Limiter(s)) => &mut s.release_ms,
            (Self::DelayTime, RackProcessor::Delay(s)) => &mut s.delay_ms,
            (Self::DelayFeedback, RackProcessor::Delay(s)) => &mut s.feedback,
            (Self::DelayMix, RackProcessor::Delay(s)) => &mut s.mix,
            (Self::Compressor(parameter), RackProcessor::Compressor(s)) => match parameter {
                CompressorParameter::InputGain => &mut s.input_gain_db,
                CompressorParameter::Threshold => &mut s.threshold_db,
                CompressorParameter::Ratio => &mut s.ratio,
                CompressorParameter::Attack => &mut s.attack_ms,
                CompressorParameter::Release => &mut s.release_ms,
                CompressorParameter::Hold => &mut s.hold_ms,
                CompressorParameter::Knee => &mut s.knee_db,
                CompressorParameter::MakeupGain => &mut s.makeup_gain_db,
                CompressorParameter::OutputGain => &mut s.output_gain_db,
                CompressorParameter::Mix => &mut s.mix,
            },
            _ => return Err("The Rack parameter does not match the stage processor".into()),
        };
        *target = value;
        Ok(())
    }
}
