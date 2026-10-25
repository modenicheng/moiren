//! Fused Send + Bus kernel. Parameter identity stays with each logical Send.
use super::*;
use crate::{
    buffer::{AudioBlockMut, ProcessIo, ReadPorts},
    compiler::send::Send,
    control::FloatRamp,
    processor::Pan,
};

use moiren_core::protocol::ParamValue;

const TILE_FRAMES: usize = 32;

pub(super) struct PreparedMixInput {
    pub(super) runtime: usize,
    pub(super) port: u16,
    pub(super) params: ParameterBindings,
    gain: FloatRamp,
    pan: FloatRamp,
    muted: bool,
}
impl PreparedMixInput {
    pub(super) fn new(runtime: usize, port: u16, params: ParameterBindings) -> Self {
        Self {
            runtime,
            port,
            params,
            gain: FloatRamp::constant(1.0),
            pan: FloatRamp::constant(0.0),
            muted: false,
        }
    }
}

pub(super) fn process_mix<S: ProcessingSample>(
    io: ProcessIo<'_, S>,
    inputs: &mut [PreparedMixInput],
    parameters: &ParameterRuntime,
) {
    // Refresh existing scratch fields once per event segment. No parameter
    // lookup or heap allocation belongs inside the per-sample loop.
    for input in inputs.iter_mut() {
        let params = parameters.view(&input.params);
        input.gain = params.float(Send::GAIN).expect("validated mix gain");
        input.pan = params.float(Send::PAN).expect("validated mix pan");
        input.muted = params.discrete(Send::MUTE) == Some(ParamValue::Bool(true));
    }
    let ProcessIo::Separate {
        inputs: sources,
        mut outputs,
    } = io
    else {
        unreachable!("fused mix requires separate IO")
    };
    let mut output = outputs.get_mut(0).expect("validated fused mix output");
    let channels = output.channel_count();
    // Tiles keep input metadata outside the sample loop and let the compiler
    // vectorize contiguous planar samples. The bounded stack accumulator avoids
    // per-Send audio storage while writing each output sample exactly once.
    if channels == 2 {
        process_stereo(&sources, &mut output, inputs);
        return;
    }
    for channel in 0..channels {
        for start in (0..output.frames()).step_by(TILE_FRAMES) {
            let end = (start + TILE_FRAMES).min(output.frames());
            let mut sums = [S::ZERO; TILE_FRAMES];
            let active = &mut sums[..end - start];
            for (input, (_, source)) in inputs.iter().zip(sources.iter()) {
                if input.muted {
                    continue;
                }
                let samples = &source.channel(channel)[start..end];
                if let Some(gain) = input.gain.constant_value() {
                    for (value, sample) in active.iter_mut().zip(samples) {
                        accumulate(value, *sample, gain);
                    }
                } else {
                    for (offset, (value, sample)) in active.iter_mut().zip(samples).enumerate() {
                        accumulate(value, *sample, input.gain.sample(start + offset));
                    }
                }
            }
            // Empty and fully muted mixes also assign every sample, so no
            // preliminary full-buffer clear is necessary.
            output.channel_mut(channel)[start..end].copy_from_slice(active);
        }
    }
}

fn process_stereo<S: ProcessingSample>(
    sources: &ReadPorts<'_, S>,
    output: &mut AudioBlockMut<'_, S>,
    inputs: &[PreparedMixInput],
) {
    for start in (0..output.frames()).step_by(TILE_FRAMES) {
        let end = (start + TILE_FRAMES).min(output.frames());
        let mut left = [S::ZERO; TILE_FRAMES];
        let mut right = [S::ZERO; TILE_FRAMES];
        for (input, (_, source)) in inputs.iter().zip(sources.iter()) {
            if input.muted {
                continue;
            }
            let samples = left[..end - start]
                .iter_mut()
                .zip(&mut right[..end - start])
                .zip(&source.channel(0)[start..end])
                .zip(&source.channel(1)[start..end]);
            if let (Some(gain), Some(pan)) =
                (input.gain.constant_value(), input.pan.constant_value())
            {
                let left_coefficient = gain * Pan::coefficient(0, pan);
                let right_coefficient = gain * Pan::coefficient(1, pan);
                for (((left, right), l), r) in samples {
                    accumulate(left, *l, left_coefficient);
                    accumulate(right, *r, right_coefficient);
                }
            } else {
                // Gain and position are shared by the stereo pair. Sampling
                // each ramp once avoids duplicating interpolation per channel.
                for (offset, (((left, right), l), r)) in samples.enumerate() {
                    let gain = input.gain.sample(start + offset);
                    let pan = input.pan.sample(start + offset);
                    accumulate(left, *l, gain * Pan::coefficient(0, pan));
                    accumulate(right, *r, gain * Pan::coefficient(1, pan));
                }
            }
        }
        output.channel_mut(0)[start..end].copy_from_slice(&left[..end - start]);
        output.channel_mut(1)[start..end].copy_from_slice(&right[..end - start]);
    }
}

#[inline]
fn accumulate<S: ProcessingSample>(value: &mut S, sample: S, coefficient: f64) {
    // Match both reference rounding points: the Send's store and each Bus
    // addition. Accumulating f32 in f64 would change the reference result.
    let sent = S::from_f64(sample.to_f64() * coefficient);
    *value = S::from_f64(value.to_f64() + sent.to_f64());
}
