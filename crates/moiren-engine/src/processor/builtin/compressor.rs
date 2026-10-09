use super::super::{ProcessContext, ProcessorError, RtProcessor};
use crate::{
    buffer::{PreparedIo, ProcessIo},
    control::{ParamDomain, ParamSpec, ProcessParameters},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

/// Initial control-side values. Gains, threshold and knee are dB; times are ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorSettings {
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

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            input_gain_db: 0.0,
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            hold_ms: 0.0,
            knee_db: 6.0,
            makeup_gain_db: 0.0,
            output_gain_db: 0.0,
            mix: 1.0,
        }
    }
}

/// Feedforward peak compressor with one detector shared by all channels.
/// Persistent state occupies fixed storage independent of block/channel count.
#[derive(Debug, Default)]
pub struct Compressor {
    reduction_db: f64,
    hold_elapsed_frames: u64,
}

impl Compressor {
    pub const INPUT_GAIN: ParameterId = ParameterId(0);
    pub const THRESHOLD: ParameterId = ParameterId(1);
    pub const RATIO: ParameterId = ParameterId(2);
    pub const ATTACK: ParameterId = ParameterId(3);
    pub const RELEASE: ParameterId = ParameterId(4);
    pub const HOLD: ParameterId = ParameterId(5);
    pub const KNEE: ParameterId = ParameterId(6);
    pub const MAKEUP_GAIN: ParameterId = ParameterId(7);
    pub const OUTPUTGAIN: ParameterId = ParameterId(8);
    pub const OUTPUT_GAIN: ParameterId = Self::OUTPUTGAIN;
    pub const MIX: ParameterId = ParameterId(9);

    const DOMAINS: [(f64, f64); 10] = [
        (-60.0, 60.0),
        (-120.0, 24.0),
        (1.0, 100.0),
        (0.0, 60_000.0),
        (0.0, 60_000.0),
        (0.0, 60_000.0),
        (0.0, 60.0),
        (-60.0, 60.0),
        (-60.0, 60.0),
        (0.0, 1.0),
    ];

    pub fn new() -> Self {
        Self::default()
    }

    /// All ten values have bounded float domains; invalid initial settings are
    /// rejected by parameter_channel before a processor can enter render.
    pub fn parameters(id: ProcessorId, settings: CompressorSettings) -> [ParamSpec; 10] {
        let values = [
            settings.input_gain_db,
            settings.threshold_db,
            settings.ratio,
            settings.attack_ms,
            settings.release_ms,
            settings.hold_ms,
            settings.knee_db,
            settings.makeup_gain_db,
            settings.output_gain_db,
            settings.mix,
        ];
        std::array::from_fn(|index| {
            let (min, max) = Self::DOMAINS[index];
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

    fn gain(db: f64) -> f64 {
        10.0_f64.powf(db / 20.0)
    }

    // Giannoulis / Massberg / Reiss, JAES 2012, Eq. (4):
    // https://joshreiss.github.io/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf
    fn target_reduction(level_db: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
        let above = level_db - threshold;
        let slope = 1.0 - 1.0 / ratio;
        if above <= -knee * 0.5 {
            0.0
        } else if knee > 0.0 && above < knee * 0.5 {
            slope * (above + knee * 0.5).powi(2) / (2.0 * knee)
        } else {
            slope * above
        }
    }

    fn smooth(current: f64, target: f64, ms: f64, sr: f64) -> f64 {
        if ms == 0.0 {
            target
        } else {
            // exp_m1 avoids losing slow ramp steps to cancellation.
            current + (target - current) * -(-1000.0 / (ms * sr)).exp_m1()
        }
    }

    fn coefficient(&mut self, peak: f64, values: [f64; 10], sr: f64) -> f64 {
        // Adding input gain in dB avoids overflowing the detector's amplitude.
        let level_db = if peak == 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * peak.log10() + values[0]
        };
        let target = Self::target_reduction(level_db, values[1], values[2], values[6]);
        if target >= self.reduction_db {
            self.reduction_db = Self::smooth(self.reduction_db, target, values[3], sr);
            self.hold_elapsed_frames = 0;
        } else if self.hold_elapsed_frames < (values[5] * (sr / 1000.0)).ceil() as u64 {
            self.hold_elapsed_frames = self.hold_elapsed_frames.saturating_add(1);
        } else {
            self.reduction_db = Self::smooth(self.reduction_db, target, values[4], sr);
        }
        let wet_gain = Self::gain(values[0] - self.reduction_db + values[7]);
        ((1.0 - values[9]) + values[9] * wet_gain) * Self::gain(values[8])
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Compressor {
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0).is_none()
            || io.input_channels(0) != io.output_channels(0)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        // Check presence first, so missing/type-mismatched schema has one error.
        for index in 0..10 {
            params
                .float(ParameterId(index))
                .ok_or(ProcessorError::MissingParameter)?;
        }
        for (index, &(supported_min, supported_max)) in Self::DOMAINS.iter().enumerate() {
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

    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
    ) {
        let ramps: [_; 10] = std::array::from_fn(|index| {
            params
                .float(ParameterId(index as u32))
                .expect("validated compressor parameter")
        });
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let input = inputs.get(0).expect("validated compressor input");
                let mut output = outputs.get_mut(0).expect("validated compressor output");
                for frame in 0..input.frames() {
                    let peak = input
                        .channels()
                        .fold(0.0_f64, |peak, ch| peak.max(ch[frame].to_f64().abs()));
                    let values = ramps.map(|ramp| ramp.sample(frame));
                    let gain = self.coefficient(peak, values, ctx.processing_sr);
                    for channel in 0..input.channel_count() {
                        output.channel_mut(channel)[frame] =
                            S::from_f64(input.channel(channel)[frame].to_f64() * gain);
                    }
                }
            }
            ProcessIo::InPlace { mut pairs, .. } => {
                let mut block = pairs.get_mut(0, 0).expect("validated compressor pair");
                for frame in 0..block.frames() {
                    // Complete detection before overwriting any linked channel.
                    let peak = (0..block.channel_count()).fold(0.0_f64, |peak, ch| {
                        peak.max(block.channel(ch)[frame].to_f64().abs())
                    });
                    let values = ramps.map(|ramp| ramp.sample(frame));
                    let gain = self.coefficient(peak, values, ctx.processing_sr);
                    for channel in block.channels_mut() {
                        channel[frame] = S::from_f64(channel[frame].to_f64() * gain);
                    }
                }
            }
            ProcessIo::ReadOnly { .. } => unreachable!("compressor requires an output"),
        }
    }
}
