use moiren_core::protocol::{ParamValue, ParameterId, ProcessorId};
use moiren_engine::{
    control::ParamDomain,
    processor::{Compressor, CompressorSettings},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressorState {
    pub input_gain_db: f64,
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub hold_ms: f64,
    pub knee_db: f64,
    pub makeup_gain_db: f64,
    pub output_gain_db: f64,
    pub mix: f64,
}

impl Default for CompressorState {
    fn default() -> Self {
        let settings = CompressorSettings::default();
        Self {
            input_gain_db: settings.input_gain_db,
            threshold_db: settings.threshold_db,
            ratio: settings.ratio,
            attack_ms: settings.attack_ms,
            release_ms: settings.release_ms,
            hold_ms: settings.hold_ms,
            knee_db: settings.knee_db,
            makeup_gain_db: settings.makeup_gain_db,
            output_gain_db: settings.output_gain_db,
            mix: settings.mix,
        }
    }
}

impl CompressorState {
    pub fn settings(self) -> CompressorSettings {
        CompressorSettings {
            input_gain_db: self.input_gain_db,
            threshold_db: self.threshold_db,
            ratio: self.ratio,
            attack_ms: self.attack_ms,
            release_ms: self.release_ms,
            hold_ms: self.hold_ms,
            knee_db: self.knee_db,
            makeup_gain_db: self.makeup_gain_db,
            output_gain_db: self.output_gain_db,
            mix: self.mix,
        }
    }

    pub fn valid(self) -> bool {
        CompressorParameter::ALL
            .into_iter()
            .all(|parameter| parameter.accepts(self.value(parameter)))
    }

    pub fn value(self, parameter: CompressorParameter) -> f64 {
        match parameter {
            CompressorParameter::InputGain => self.input_gain_db,
            CompressorParameter::Threshold => self.threshold_db,
            CompressorParameter::Ratio => self.ratio,
            CompressorParameter::Attack => self.attack_ms,
            CompressorParameter::Release => self.release_ms,
            CompressorParameter::Hold => self.hold_ms,
            CompressorParameter::Knee => self.knee_db,
            CompressorParameter::MakeupGain => self.makeup_gain_db,
            CompressorParameter::OutputGain => self.output_gain_db,
            CompressorParameter::Mix => self.mix,
        }
    }

    pub fn set(&mut self, parameter: CompressorParameter, value: f64) {
        match parameter {
            CompressorParameter::InputGain => self.input_gain_db = value,
            CompressorParameter::Threshold => self.threshold_db = value,
            CompressorParameter::Ratio => self.ratio = value,
            CompressorParameter::Attack => self.attack_ms = value,
            CompressorParameter::Release => self.release_ms = value,
            CompressorParameter::Hold => self.hold_ms = value,
            CompressorParameter::Knee => self.knee_db = value,
            CompressorParameter::MakeupGain => self.makeup_gain_db = value,
            CompressorParameter::OutputGain => self.output_gain_db = value,
            CompressorParameter::Mix => self.mix = value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompressorParameter {
    InputGain,
    Threshold,
    Ratio,
    Attack,
    Release,
    Hold,
    Knee,
    MakeupGain,
    OutputGain,
    Mix,
}

impl CompressorParameter {
    pub const ALL: [Self; 10] = [
        Self::InputGain,
        Self::Threshold,
        Self::Ratio,
        Self::Attack,
        Self::Release,
        Self::Hold,
        Self::Knee,
        Self::MakeupGain,
        Self::OutputGain,
        Self::Mix,
    ];

    pub fn id(self) -> ParameterId {
        match self {
            Self::InputGain => Compressor::INPUT_GAIN,
            Self::Threshold => Compressor::THRESHOLD,
            Self::Ratio => Compressor::RATIO,
            Self::Attack => Compressor::ATTACK,
            Self::Release => Compressor::RELEASE,
            Self::Hold => Compressor::HOLD,
            Self::Knee => Compressor::KNEE,
            Self::MakeupGain => Compressor::MAKEUP_GAIN,
            Self::OutputGain => Compressor::OUTPUT_GAIN,
            Self::Mix => Compressor::MIX,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::InputGain => "Input gain",
            Self::Threshold => "Threshold",
            Self::Ratio => "Ratio",
            Self::Attack => "Attack",
            Self::Release => "Release",
            Self::Hold => "Hold",
            Self::Knee => "Knee",
            Self::MakeupGain => "Makeup gain",
            Self::OutputGain => "Output gain",
            Self::Mix => "Dry / wet",
        }
    }

    pub fn display(self, value: f64) -> String {
        match self {
            Self::Ratio => format!("{value:.2}:1"),
            Self::Attack | Self::Release | Self::Hold => format!("{value:.1} ms"),
            Self::Mix => format!("{:.1}%", value * 100.),
            _ => format!("{value:.1} dB"),
        }
    }

    pub fn range(self) -> (f64, f64) {
        match Compressor::parameters(ProcessorId(0), CompressorSettings::default())
            [self.id().0 as usize]
            .domain
        {
            ParamDomain::Float { min, max } => (min, max),
            _ => unreachable!("compressor parameters have float domains"),
        }
    }

    pub fn accepts(self, value: f64) -> bool {
        let (min, max) = self.range();
        value.is_finite() && (min..=max).contains(&value)
    }

    pub fn desired(self, state: CompressorState) -> (ParameterId, ParamValue) {
        (self.id(), ParamValue::Float(state.value(self)))
    }
}
