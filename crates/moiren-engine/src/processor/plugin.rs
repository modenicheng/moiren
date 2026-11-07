//! Engine adapter driving a third-party plugin instance as an `RtProcessor`.
//!
//! The plugin runs at f32 planar precision (every format's floor); the
//! engine's processing sample is bridged through preallocated f32 staging
//! buffers so both f32 and f64 graphs can host plugins. Parameter automation
//! is block-granular: each block, every parameter's effective value is
//! compared against the last pushed one and only changes are forwarded.

use moiren_plugins::{PluginInstance, PluginParamEvent};

use crate::{
    buffer::{AudioBlockMut, PreparedIo, ProcessIo},
    control::{FloatRamp, ParamDomain, ParamSpec, ProcessParameters},
    processor::{ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample,
};
use moiren_core::protocol::{ParamValue, ParameterId, ParameterKey, ProcessorId};

/// Persisted configuration of one plugin node: what to load and the
/// parameter table the scan collected by probing an instance.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSettings {
    pub descriptor: moiren_plugins::spec::PluginDescriptor,
    /// Parameter snapshot from the scanning probe; the engine's automation
    /// table is derived from it, so it must match what a fresh instance
    /// reports (the probe guarantees that at scan time).
    pub parameters: Vec<moiren_plugins::spec::PluginParameterSpec>,
}

impl PluginSettings {
    /// The engine-side automation schema of the plugin's parameters.
    /// Plugin automation values use the format's own unit (CLAP: plain
    /// values, VST2/VST3: normalized 0..1), mapped one-to-one onto
    /// `ParamDomain::Float`.
    pub fn specs(&self, id: ProcessorId) -> Vec<ParamSpec> {
        self.parameters
            .iter()
            .map(|parameter| ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: ParameterId(parameter.id),
                },
                domain: ParamDomain::Float {
                    min: parameter.min,
                    max: parameter.max,
                },
                initial: ParamValue::Float(parameter.default),
            })
            .collect()
    }

    /// Whether this plugin can serve a node of `channels`: equal main-bus
    /// width and a finite, self-consistent parameter table.
    pub fn valid_for_channels(&self, channels: usize) -> bool {
        self.descriptor.channels() as usize == channels
            && self.parameters.iter().all(|parameter| {
                parameter.min.is_finite()
                    && parameter.max.is_finite()
                    && parameter.min <= parameter.max
                    && parameter.default >= parameter.min
                    && parameter.default <= parameter.max
            })
    }
}

/// One live plugin behind the engine's processor contract. Construction
/// (control side) instantiates and activates; `Drop` deactivates and
/// destroys. `process` is the realtime path and allocates nothing.
pub struct PluginProcessor<S> {
    instance: Box<dyn PluginInstance>,
    parameters: Vec<moiren_plugins::spec::PluginParameterSpec>,
    channels: usize,
    /// f32 staging planes for the plugin's buses, [channel][frame]; the
    /// plugin always renders stage-to-stage with distinct buffers.
    input_stage: Vec<Vec<f32>>,
    output_stage: Vec<Vec<f32>>,
    /// Per parameter, the value last sent to the plugin; the diff drives
    /// the event list. Initialized to the reported defaults.
    last_pushed: Vec<f64>,
    events: Vec<PluginParamEvent>,
    _sample: std::marker::PhantomData<S>,
}

impl<S: ProcessingSample> PluginProcessor<S> {
    /// Instantiate for the descriptor's bus width at the given engine
    /// bounds. Control side only; activation happens in the format backend.
    pub fn new(
        settings: &PluginSettings,
        sample_rate: f64,
        max_block_frames: usize,
    ) -> Result<Self, moiren_plugins::PluginError> {
        let instance = moiren_plugins::instantiate(
            &settings.descriptor,
            sample_rate,
            max_block_frames.max(1) as u32,
        )?;
        let channels = instance.descriptor().channels() as usize;
        let capacity = max_block_frames.max(1);
        let parameters = instance.parameters().to_vec();
        Ok(Self {
            last_pushed: parameters
                .iter()
                .map(|parameter| parameter.default)
                .collect(),
            instance,
            parameters,
            channels,
            input_stage: vec![vec![0.0; capacity]; channels],
            output_stage: vec![vec![0.0; capacity]; channels],
            events: Vec::new(),
            _sample: std::marker::PhantomData,
        })
    }

    /// The automation schema this live instance serves.
    pub fn specs(&self, id: ProcessorId) -> Vec<ParamSpec> {
        self.parameters
            .iter()
            .map(|parameter| ParamSpec {
                key: ParameterKey {
                    processor: id,
                    parameter: ParameterId(parameter.id),
                },
                domain: ParamDomain::Float {
                    min: parameter.min,
                    max: parameter.max,
                },
                initial: ParamValue::Float(parameter.default),
            })
            .collect()
    }

    fn capacity(&self) -> usize {
        self.output_stage.first().map_or(0, Vec::len)
    }

    /// Copy an engine-side input block into the f32 input stage.
    fn stage_input<'a>(&mut self, channels: impl Iterator<Item = &'a [S]>, frames: usize) {
        for (stage, channel) in self
            .input_stage
            .iter_mut()
            .zip(channels.map(|channel| &channel[..frames]))
        {
            for (target, sample) in stage[..frames].iter_mut().zip(channel.iter()) {
                *target = sample.to_f64() as f32;
            }
        }
    }

    /// Copy the f32 output stage back into an engine-side output block.
    fn unstage_output(&self, output: &mut AudioBlockMut<'_, S>, frames: usize) {
        for (channel, stage) in output.channels_mut().zip(&self.output_stage) {
            for (target, sample) in channel[..frames].iter_mut().zip(stage[..frames].iter()) {
                *target = S::from_f64(f64::from(*sample));
            }
        }
    }

    /// Run the plugin block on the staging planes and zero outputs on
    /// failure so a broken plugin cannot emit stale buffers.
    fn render(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        let inputs: Vec<&[f32]> = self
            .input_stage
            .iter()
            .map(|stage| &stage[..frames])
            .collect();
        let mut outputs: Vec<&mut [f32]> = self
            .output_stage
            .iter_mut()
            .map(|stage| &mut stage[..frames])
            .collect();
        if self
            .instance
            .process(&inputs, &mut outputs, &self.events)
            .is_err()
        {
            for channel in &mut self.output_stage {
                channel[..frames].fill(0.0);
            }
        }
    }
}

impl<S: ProcessingSample> RtProcessor<S> for PluginProcessor<S> {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Transform
    }

    fn latency_frames(&self) -> u32 {
        self.instance.latency_frames()
    }

    /// Plugins keep internal state and assume distinct input/output
    /// buffers, so in-place folding is never sound for them.
    fn transport_interchangeable(&self) -> bool {
        false
    }

    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() != 1
            || io.output_count() != 1
            || io.input_channels(0) != Some(self.channels)
            || io.input_channels(0) != io.output_channels(0)
        {
            return Err(ProcessorError::InvalidIo);
        }
        Ok(())
    }

    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        let all_present = self
            .parameters
            .iter()
            .all(|parameter| params.float(ParameterId(parameter.id)).is_some());
        all_present
            .then_some(())
            .ok_or(ProcessorError::MissingParameter)
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        // Block-granular automation: forward only changed values, computed
        // from each parameter's effective value at this block's start.
        self.events.clear();
        for (index, parameter) in self.parameters.iter().enumerate() {
            let Some(ramp) = params.float(ParameterId(parameter.id)) else {
                continue;
            };
            let value = effective_value(ramp);
            if value != self.last_pushed[index] {
                self.last_pushed[index] = value;
                self.events.push(PluginParamEvent {
                    parameter: parameter.id,
                    value,
                });
            }
        }
        let capacity = self.capacity();
        match io {
            ProcessIo::Separate {
                inputs,
                mut outputs,
            } => {
                let (Some(input), Some(mut output)) = (inputs.get(0), outputs.get_mut(0)) else {
                    return;
                };
                let frames = output.frames().min(capacity);
                self.stage_input(input.channels(), frames);
                self.render(frames);
                self.unstage_output(&mut output, frames);
            }
            // Not negotiated for plugins (transport_interchangeable is
            // false), but a single in-place pair still renders correctly
            // through the staging planes.
            ProcessIo::InPlace { mut pairs, .. } => {
                let Some(mut block) = pairs.get_mut(0, 0) else {
                    return;
                };
                let frames = block.frames().min(capacity);
                let channel_count = block.channel_count();
                self.stage_input(
                    (0..channel_count).map(|channel| block.channel(channel)),
                    frames,
                );
                self.render(frames);
                self.unstage_output(&mut block, frames);
            }
            ProcessIo::ReadOnly { .. } => {}
        }
    }
}

/// A ramp's effective value for block-granular automation: the constant
/// target once settled, otherwise the value at the block's first frame.
fn effective_value(ramp: FloatRamp) -> f64 {
    ramp.constant_value().unwrap_or_else(|| ramp.sample(0))
}
