//! Capture-only sine IO over the existing no-device transport and real engine.
use crate::service::test_backend::FakeBackend;
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    host::{
        BackendError, CancellationToken, EngineBackend, NodeConfig, PrepareRequest,
        PreparedBackend, ResourceEvent,
    },
    node::InputSource,
    processor::ProcessContext,
};

#[derive(Clone)]
pub(in crate::ui::window::capture::node_workstations) struct ToneBackend(pub FakeBackend);

impl EngineBackend<f32> for ToneBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f32>, BackendError> {
        // Delegate only sink/transport preparation: FakeBackend would otherwise
        // bind every input to DC, including a software Tone selector. The engine
        // still compiles the original, unmodified project supplied by its host.
        let mut delegated = request.project.clone();
        delegated
            .nodes
            .retain(|_, config| !matches!(config, NodeConfig::Input(_)));
        let mut prepared = self.0.prepare(
            PrepareRequest {
                project: &delegated,
                ..request
            },
            cancel,
        )?;
        for (&node, config) in &request.project.nodes {
            let NodeConfig::Input(config) = config else {
                continue;
            };
            if request.reuse.iter().any(|&(reused, _)| reused == node) {
                continue;
            }
            let InputSource::Software { name } = &config.source else {
                return Err(BackendError::Unsupported(
                    "spectrum capture requires an explicit software tone".into(),
                ));
            };
            let source = ToneSource::new(
                name,
                config.channels,
                request.project.processing.engine.processing_sr,
            )?;
            prepared
                .bindings
                .bind_source(node, source)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
        }
        Ok(prepared)
    }

    fn poll_resource(&self) -> Option<ResourceEvent> {
        <FakeBackend as EngineBackend<f32>>::poll_resource(&self.0)
    }
}

pub(super) struct ToneSource {
    frequency: f64,
    phase: f64,
    channels: usize,
}

impl ToneSource {
    pub(super) fn new(name: &str, channels: usize, rate: f64) -> Result<Self, BackendError> {
        let frequency = name
            .strip_prefix("moiren:tone:")
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|hz| hz.is_finite() && *hz > 0. && *hz < rate * 0.5)
            .filter(|_| rate.is_finite() && (1..=256).contains(&channels))
            .ok_or_else(|| {
                BackendError::Unsupported("invalid capture tone selector/rate/channels".into())
            })?;
        Ok(Self {
            frequency,
            phase: 0.,
            channels,
        })
    }
}

impl RtAudioSource<f32> for ToneSource {
    fn channel_count(&self) -> usize {
        self.channels
    }

    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        // Same sine/amplitude convention as native software Tone. No allocation
        // or locks in the callback; phase persists across render blocks/swaps.
        let step = self.frequency / ctx.processing_sr;
        for frame in 0..output.frames() {
            let value = ((self.phase * std::f64::consts::TAU).sin() * 0.1) as f32;
            for channel in 0..self.channels {
                output.channel_mut(channel)[frame] = value;
            }
            self.phase = (self.phase + step).fract();
        }
        BoundaryReport {
            transferred_frames: output.frames(),
            ..Default::default()
        }
    }
}
