//! EngineHost adapter. Native preparation, cancellation and ownership stay here.
use crate::process_loopback::ProcessIdentity;
use moiren_engine::{
    boundary::{CaptureIntent, ClockRole},
    host::BackendError,
    node::{DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig, OutputTarget},
};
use std::time::Duration;

#[cfg(windows)]
mod owner;
#[cfg(windows)]
mod preparation;
#[cfg(windows)]
mod resources;
mod selectors;
mod validation;
pub use validation::validate_native_project;
mod observations;
mod profile;
pub use observations::{
    NativeBridgeObservation, NativeObservationSnapshot, NativeObservations, NativeStreamKind,
    NativeStreamObservation,
};
pub use profile::NativeLatencyProfile;

#[cfg(windows)]
pub use preparation::WindowsBackend;

#[derive(Debug, Clone, Copy)]
pub struct WindowsBackendConfig {
    pub latency_profile: NativeLatencyProfile,
    pub probe_interval: Duration,
    pub max_resources: usize,
    pub render_queue: crate::latency::RenderQueuePolicy,
    pub latency_budget: crate::latency::LatencyBudgetPolicy,
    pub telemetry: crate::latency::TelemetryMode,
}

impl Default for WindowsBackendConfig {
    fn default() -> Self {
        Self {
            latency_profile: NativeLatencyProfile::Compatible,
            probe_interval: Duration::from_millis(500),
            max_resources: 64,
            render_queue: crate::latency::RenderQueuePolicy::default(),
            latency_budget: crate::latency::LatencyBudgetPolicy::Compatible,
            telemetry: crate::latency::TelemetryMode::Summary,
        }
    }
}

impl WindowsBackendConfig {
    pub fn validate(self) -> Result<(), BackendError> {
        if self.probe_interval < Duration::from_millis(100)
            || self.probe_interval > Duration::from_secs(60)
            || !(1..=256).contains(&self.max_resources)
        {
            return Err(BackendError::Unsupported(
                "probe interval must be 100ms..60s and resource capacity 1..256".into(),
            ));
        }
        self.render_queue
            .validate()
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        if self.latency_profile != NativeLatencyProfile::Compatible
            && (self.render_queue != crate::latency::RenderQueuePolicy::FullCapacity
                || self.latency_budget != crate::latency::LatencyBudgetPolicy::Compatible)
        {
            return Err(BackendError::Unsupported(
                "relative latency profiles cannot be combined with explicit queue budgets".into(),
            ));
        }
        if let crate::latency::LatencyBudgetPolicy::Custom { render, .. } = self.latency_budget
            && render != self.render_queue
        {
            return Err(BackendError::Unsupported(
                "latency budget and renderer must use the same render queue policy".into(),
            ));
        }
        Ok(())
    }
}

pub fn physical_input(endpoint: EndpointSelector) -> InputConfig {
    InputConfig {
        source: InputSource::Physical(endpoint),
        channels: 2,
        device: DeviceOptions::default(),
    }
}

pub fn physical_output(endpoint: EndpointSelector) -> OutputConfig {
    OutputConfig {
        target: OutputTarget::Physical(endpoint),
        channels: 2,
        device: DeviceOptions {
            clock_role: ClockRole::Master,
            ..DeviceOptions::default()
        },
    }
}

pub fn tone_input(frequency_hz: f64) -> Result<InputConfig, BackendError> {
    selectors::validate_frequency(frequency_hz)?;
    Ok(InputConfig {
        source: InputSource::Software {
            name: format!("moiren:tone:{frequency_hz}"),
        },
        channels: 2,
        device: DeviceOptions::default(),
    })
}

/// Preserve the creation time in project intent: a recycled PID never selects
/// another application, even when the executable basename is unchanged.
pub fn process_input(identity: &ProcessIdentity) -> Result<InputConfig, BackendError> {
    if identity.pid == 0 || identity.creation_time_100ns == 0 {
        return Err(BackendError::Unsupported(
            "process identity requires PID and creation time".into(),
        ));
    }
    Ok(InputConfig {
        source: InputSource::Application {
            executable: format!(
                "moiren:process:{}:{}",
                identity.pid, identity.creation_time_100ns
            ),
            intent: CaptureIntent::PassiveTap,
        },
        channels: 2,
        device: DeviceOptions::default(),
    })
}
