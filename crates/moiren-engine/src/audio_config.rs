//! Saved audio intent, independent of platform discovery and negotiated state.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    boundary::ClockRole,
    node::{DeviceMode, DeviceOptions},
};

pub const SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const BUFFER_SIZES: [u32; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    #[default]
    Wasapi,
    Asio,
}

impl AudioBackend {
    pub const ALL: [Self; 2] = [Self::Wasapi, Self::Asio];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Wasapi => "WASAPI",
            Self::Asio => "ASIO",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingPrecision {
    #[default]
    F32,
    F64,
}

impl ProcessingPrecision {
    pub const ALL: [Self; 2] = [Self::F32, Self::F64];

    pub const fn title(self) -> &'static str {
        match self {
            Self::F32 => "32-bit float",
            Self::F64 => "64-bit float",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    #[default]
    Native,
    Pcm16,
    Pcm24,
    Pcm32,
    Float32,
    Float64,
}

impl SampleFormat {
    pub const ALL: [Self; 6] = [
        Self::Native,
        Self::Pcm16,
        Self::Pcm24,
        Self::Pcm32,
        Self::Float32,
        Self::Float64,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Native => "Device native",
            Self::Pcm16 => "16-bit PCM",
            Self::Pcm24 => "24-bit PCM",
            Self::Pcm32 => "32-bit PCM",
            Self::Float32 => "32-bit float",
            Self::Float64 => "64-bit float",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessingSettings {
    pub sample_rate: u32,
    pub precision: ProcessingPrecision,
    pub block_frames: u32,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            precision: ProcessingPrecision::F32,
            block_frames: 256,
        }
    }
}

impl ProcessingSettings {
    pub fn validate(self) -> Result<(), AudioConfigError> {
        validate_rate(self.sample_rate)?;
        validate_buffer(self.block_frames)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamSettings {
    /// None follows processing SR; it never means silently change a system mix.
    pub sample_rate: Option<u32>,
    pub format: SampleFormat,
    /// None lets the driver choose. Explicit choices are always frame counts.
    pub buffer_frames: Option<u32>,
    pub mode: DeviceMode,
}

impl StreamSettings {
    pub fn validate(self) -> Result<(), AudioConfigError> {
        if let Some(rate) = self.sample_rate {
            validate_rate(rate)?;
        }
        if let Some(frames) = self.buffer_frames {
            validate_buffer(frames)?;
        }
        Ok(())
    }

    /// Resolve inherited rate before crossing the native preparation boundary.
    pub fn device_options(
        self,
        backend: AudioBackend,
        processing_rate: u32,
        clock_role: ClockRole,
    ) -> DeviceOptions {
        DeviceOptions {
            backend,
            mode: self.mode,
            hardware_sample_rate: Some(self.sample_rate.unwrap_or(processing_rate)),
            // IASIO exposes driver-owned channel encodings. Keep a saved
            // WASAPI format when switching APIs, but never impose it on ASIO.
            sample_format: if backend == AudioBackend::Asio {
                SampleFormat::Native
            } else {
                self.format
            },
            period_frames: self.buffer_frames.map(|frames| frames as usize),
            clock_role,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioSettings {
    pub backend: AudioBackend,
    pub processing: ProcessingSettings,
    pub input: StreamSettings,
    pub output: StreamSettings,
    /// Full device snapshots deliberately keep custom devices independent of
    /// later default edits. Removing an entry restores live inheritance.
    pub input_overrides: BTreeMap<String, StreamSettings>,
    pub output_overrides: BTreeMap<String, StreamSettings>,
}

impl AudioSettings {
    pub fn validate(&self) -> Result<(), AudioConfigError> {
        self.processing.validate()?;
        self.input.validate()?;
        self.output.validate()?;
        for overrides in [&self.input_overrides, &self.output_overrides] {
            if overrides.len() > 256 {
                return Err(AudioConfigError::TooManyDevices);
            }
            for (identity, config) in overrides {
                if identity.trim().is_empty() || identity.contains('\0') || identity.len() > 32768 {
                    return Err(AudioConfigError::InvalidDeviceIdentity);
                }
                config.validate()?;
            }
        }
        Ok(())
    }

    pub fn effective_input(&self, identity: &str) -> StreamSettings {
        self.input_overrides
            .get(identity)
            .copied()
            .unwrap_or(self.input)
    }

    pub fn effective_output(&self, identity: &str) -> StreamSettings {
        self.output_overrides
            .get(identity)
            .copied()
            .unwrap_or(self.output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AudioConfigError {
    #[error("sample rate must be 44.1, 48, 88.2 or 96 kHz")]
    InvalidSampleRate,
    #[error("buffer must be 64, 128, 256, 512, 1024, 2048 or 4096 frames")]
    InvalidBufferSize,
    #[error("device identity must contain 1 to 32768 bytes without NUL")]
    InvalidDeviceIdentity,
    #[error("at most 256 device overrides can be saved per direction")]
    TooManyDevices,
}

fn validate_rate(rate: u32) -> Result<(), AudioConfigError> {
    if SAMPLE_RATES.contains(&rate) {
        Ok(())
    } else {
        Err(AudioConfigError::InvalidSampleRate)
    }
}

fn validate_buffer(frames: u32) -> Result<(), AudioConfigError> {
    if BUFFER_SIZES.contains(&frames) {
        Ok(())
    } else {
        Err(AudioConfigError::InvalidBufferSize)
    }
}
