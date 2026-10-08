//! Control-side IO intent. These values never open devices or own audio buffers.
use crate::boundary::{CaptureIntent, ClockRole};
use thiserror::Error;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDeviceRole {
    Console,
    Multimedia,
    Communications,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSelector {
    /// Preserve opaque IDs exactly; a missing pinned endpoint stays unresolved.
    PinnedEndpoint {
        endpoint_id: String,
        stable_id: Option<String>,
    },
    /// Capture/render direction is determined by the owning node.
    FollowDefault(DefaultDeviceRole),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceMode {
    #[default]
    Shared,
    Exclusive,
}

/// Desired hardware settings, not negotiated format or a promise of support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceOptions {
    pub mode: DeviceMode,
    pub hardware_sample_rate: Option<u32>,
    pub period_frames: Option<usize>,
    pub clock_role: ClockRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSource {
    Physical(EndpointSelector),
    /// Persistent selector; the backend resolves a live process identity.
    Application {
        executable: String,
        intent: CaptureIntent,
    },
    Software {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputTarget {
    Physical(EndpointSelector),
    Software { name: String },
}

/// One multichannel output port, no graph input ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputConfig {
    pub source: InputSource,
    pub channels: usize,
    pub device: DeviceOptions,
}

/// One multichannel input port, no graph output ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfig {
    pub target: OutputTarget,
    pub channels: usize,
    pub device: DeviceOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IoConfigError {
    #[error("an IO port must carry at least one channel")]
    InvalidChannels,
    #[error("an endpoint, executable or software selector is empty")]
    InvalidSelector,
    #[error("requested hardware sample rate or period is zero")]
    InvalidDeviceOptions,
}

fn validate_text(value: &str) -> Result<(), IoConfigError> {
    if value.trim().is_empty() {
        Err(IoConfigError::InvalidSelector)
    } else {
        Ok(())
    }
}

impl EndpointSelector {
    pub fn validate(&self) -> Result<(), IoConfigError> {
        if let Self::PinnedEndpoint {
            endpoint_id,
            stable_id,
        } = self
        {
            validate_text(endpoint_id)?;
            if let Some(id) = stable_id {
                validate_text(id)?;
            }
        }
        Ok(())
    }
}

impl DeviceOptions {
    pub fn validate(&self) -> Result<(), IoConfigError> {
        if self.hardware_sample_rate == Some(0) || self.period_frames == Some(0) {
            Err(IoConfigError::InvalidDeviceOptions)
        } else {
            Ok(())
        }
    }
}

impl InputConfig {
    pub fn validate(&self) -> Result<(), IoConfigError> {
        if self.channels == 0 {
            return Err(IoConfigError::InvalidChannels);
        }
        self.device.validate()?;
        match &self.source {
            InputSource::Physical(endpoint) => endpoint.validate(),
            InputSource::Application { executable, .. } => validate_text(executable),
            InputSource::Software { name } => validate_text(name),
        }
    }
}

impl OutputConfig {
    pub fn validate(&self) -> Result<(), IoConfigError> {
        if self.channels == 0 {
            return Err(IoConfigError::InvalidChannels);
        }
        self.device.validate()?;
        match &self.target {
            OutputTarget::Physical(endpoint) => endpoint.validate(),
            OutputTarget::Software { name } => validate_text(name),
        }
    }
}
