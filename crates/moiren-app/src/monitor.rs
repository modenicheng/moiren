//! Compiler-prepared capture routing, independent of Windows device ownership.
use moiren_core::graph::*;
use moiren_engine::{
    boundary::{AudioReader, BridgeError, RtAudioSource, audio_bridge},
    compiler::{CompileConfig, CompileError, CompiledGraph, NodeBindings, compile},
    runtime::EngineConfig,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorLatencyProfile {
    Compatible,
    Balanced,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorLatencyConfig {
    pub profile: MonitorLatencyProfile,
    pub shared_period_ms: Option<f64>,
    pub input_target_frames: Option<u32>,
    pub input_jitter_frames: Option<u32>,
    pub render_target_frames: Option<u32>,
    pub wake_margin_frames: Option<u32>,
    pub callback_budget_frames: Option<u32>,
}

impl Default for MonitorLatencyConfig {
    fn default() -> Self {
        Self {
            profile: MonitorLatencyProfile::Compatible,
            shared_period_ms: Some(5.0),
            input_target_frames: None,
            input_jitter_frames: None,
            render_target_frames: None,
            wake_margin_frames: None,
            callback_budget_frames: None,
        }
    }
}

impl MonitorLatencyConfig {
    pub fn balanced() -> Self {
        Self {
            profile: MonitorLatencyProfile::Balanced,
            shared_period_ms: Some(5.0),
            input_target_frames: Some(1024),
            input_jitter_frames: Some(96),
            render_target_frames: Some(480),
            wake_margin_frames: Some(96),
            callback_budget_frames: Some(48),
        }
    }

    pub fn custom() -> Self {
        Self {
            profile: MonitorLatencyProfile::Custom,
            shared_period_ms: Some(5.0),
            input_target_frames: None,
            input_jitter_frames: None,
            render_target_frames: None,
            wake_margin_frames: None,
            callback_budget_frames: None,
        }
    }

    pub fn validate(self) -> Result<(), MonitorLatencyError> {
        if self
            .shared_period_ms
            .is_some_and(|value| !value.is_finite() || !(0.1..=100.0).contains(&value))
        {
            return Err(MonitorLatencyError::InvalidSharedPeriod);
        }
        if self
            .input_target_frames
            .is_some_and(|value| !(2..=8190).contains(&value))
        {
            return Err(MonitorLatencyError::InvalidInputTarget);
        }
        if self.input_jitter_frames.is_some_and(|value| value > 8192) {
            return Err(MonitorLatencyError::InvalidInputJitter);
        }
        if self
            .render_target_frames
            .is_some_and(|value| !(2..=8192).contains(&value))
        {
            return Err(MonitorLatencyError::InvalidRenderTarget);
        }
        if self.wake_margin_frames.is_some_and(|value| value > 8192) {
            return Err(MonitorLatencyError::InvalidWakeMargin);
        }
        if self
            .callback_budget_frames
            .is_some_and(|value| !(1..=8192).contains(&value))
        {
            return Err(MonitorLatencyError::InvalidCallbackBudget);
        }
        match self.profile {
            MonitorLatencyProfile::Compatible => {
                if self.input_target_frames.is_some()
                    || self.input_jitter_frames.is_some()
                    || self.render_target_frames.is_some()
                    || self.wake_margin_frames.is_some()
                    || self.callback_budget_frames.is_some()
                {
                    return Err(MonitorLatencyError::BudgetOverridesRequireProfile);
                }
            }
            MonitorLatencyProfile::Balanced => {
                if self.input_target_frames.is_none()
                    || self.input_jitter_frames.is_none()
                    || self.render_target_frames.is_none()
                    || self.wake_margin_frames.is_none()
                    || self.callback_budget_frames.is_none()
                {
                    return Err(MonitorLatencyError::IncompleteProfile);
                }
            }
            MonitorLatencyProfile::Custom => {
                if self.input_target_frames.is_none() {
                    return Err(MonitorLatencyError::MissingInputTarget);
                }
                if self.render_target_frames.is_none() {
                    return Err(MonitorLatencyError::MissingRenderTarget);
                }
                if self.input_jitter_frames.is_none()
                    || self.wake_margin_frames.is_none()
                    || self.callback_budget_frames.is_none()
                {
                    return Err(MonitorLatencyError::IncompleteProfile);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MonitorLatencyError {
    #[error("shared period must be default or finite and within 0.1..=100 milliseconds")]
    InvalidSharedPeriod,
    #[error("input target must be within 2..=8190 frames")]
    InvalidInputTarget,
    #[error("input jitter must be within 0..=8192 frames")]
    InvalidInputJitter,
    #[error("render target must be within 2..=8192 frames")]
    InvalidRenderTarget,
    #[error("wake margin must be within 0..=8192 frames")]
    InvalidWakeMargin,
    #[error("callback budget must be within 1..=8192 frames")]
    InvalidCallbackBudget,
    #[error("queue budget overrides require --profile balanced or --profile custom")]
    BudgetOverridesRequireProfile,
    #[error("balanced profile is incomplete")]
    IncompleteProfile,
    #[error("custom profile requires --input-target-frames")]
    MissingInputTarget,
    #[error("custom profile requires --render-target-frames")]
    MissingRenderTarget,
    #[error("process loopback supports only the compatible latency profile")]
    ProcessProfileUnsupported,
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    MonitorCleanupExit, MonitorOptions, MonitorOwnerExit, MonitorReport, MonitorSession,
    MonitorStatus, PreparedMonitorSession, ProcessMonitorOptions, prepare_monitor_with_stop,
    prepare_process_monitor_with_stop, start_monitor, start_process_monitor,
    start_process_monitor_with_stop,
};
#[cfg(windows)]
pub(crate) use windows::{
    prepare_monitor_with_signal_host, prepare_process_monitor_with_signal_host,
};

#[derive(Debug, Clone, Copy)]
pub struct MonitorConfig {
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
    pub latency: MonitorLatencyConfig,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            gain: 0.05,
            pan: 0.0,
            max_block_frames: 256,
            latency: MonitorLatencyConfig::default(),
        }
    }
}
impl MonitorConfig {
    pub fn validate(self) -> Result<(), MonitorError> {
        if !self.gain.is_finite()
            || !(0.0..=1.0).contains(&self.gain)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || !(1..=4096).contains(&self.max_block_frames)
        {
            return Err(MonitorError::InvalidConfig);
        }
        self.latency.validate()?;
        Ok(())
    }
}
#[derive(Debug, Error)]
pub enum MonitorError {
    #[cfg(windows)]
    #[error(transparent)]
    LatencyBudget(#[from] moiren_windows_audio::latency::LatencyBudgetError),
    #[cfg(windows)]
    #[error(transparent)]
    Gate(#[from] moiren_windows_audio::GateError),
    #[error(transparent)]
    Latency(#[from] MonitorLatencyError),
    #[error(
        "monitor requires finite gain in [0,1], pan in [-1,1], a 1..4096 frame block and a stereo source"
    )]
    InvalidConfig,
    #[error("application timeline epoch space exhausted")]
    TimelineEpochExhausted,
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Signals(#[from] crate::service::SignalPreparationError),
    #[cfg(windows)]
    #[error(transparent)]
    Capture(#[from] moiren_windows_audio::capture::CaptureError),
    #[cfg(windows)]
    #[error(transparent)]
    Render(#[from] moiren_windows_audio::render::RenderError),
}
pub struct MonitorGraph {
    pub compiled: CompiledGraph<f32>,
    pub output: AudioReader<f32>,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
}
pub fn prepare_monitor(
    source: impl RtAudioSource<f32> + 'static,
    config: MonitorConfig,
) -> Result<MonitorGraph, MonitorError> {
    prepare_inner(source, config, None)
}
pub fn prepare_monitor_with_signals(
    source: impl RtAudioSource<f32> + 'static,
    config: MonitorConfig,
    signals: &mut crate::service::SignalPreparation,
) -> Result<MonitorGraph, MonitorError> {
    prepare_inner(source, config, Some(signals))
}
fn prepare_inner(
    source: impl RtAudioSource<f32> + 'static,
    config: MonitorConfig,
    signals: Option<&mut crate::service::SignalPreparation>,
) -> Result<MonitorGraph, MonitorError> {
    config.validate()?;
    if source.channel_count() != 2 {
        return Err(MonitorError::InvalidConfig);
    }
    let crate::session_graph::SessionGraph {
        graph,
        input,
        gain: gain_node,
        pan: pan_node,
        output: sink,
    } = crate::session_graph::SessionGraph::new()?;
    let (writer, output) = audio_bridge(2, config.max_block_frames, 8 * 1024 * 1024)?;
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source)?;
    bindings.bind_gain(gain_node, config.gain)?;
    bindings.bind_pan(pan_node, config.pan)?;
    bindings.bind_sink(sink, writer)?;
    let compile_config = CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: config.max_block_frames,
            max_events_per_block: 64,
        },
        audio_byte_budget: 8 * 1024 * 1024,
        plan_revision: 1,
        timeline_epoch: crate::timeline::next_epoch()
            .ok_or(MonitorError::TimelineEpochExhausted)?,
        control_capacity: 64,
        control_horizon_frames: 48_000,
    };
    let compiled = if let Some(signals) = signals {
        signals.compile(
            &graph,
            bindings,
            compile_config,
            graph.get_node(input)?.outputs()[0].id(),
            graph.get_node(pan_node)?.outputs()[0].id(),
        )?
    } else {
        compile(&graph, bindings, compile_config)?
    };
    Ok(MonitorGraph {
        compiled,
        output,
        gain_node,
        pan_node,
    })
}
