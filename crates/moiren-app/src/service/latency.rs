//! Saved latency intent. Native preparation admits the requested policy against
//! actual device timing; choosing a profile is never a latency measurement.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioLatencyProfile {
    #[default]
    Compatible,
    Balanced,
    Low,
}

impl AudioLatencyProfile {
    pub const ALL: [Self; 3] = [Self::Compatible, Self::Balanced, Self::Low];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Compatible => "Compatible",
            Self::Balanced => "Balanced",
            Self::Low => "Low",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Compatible => "Use each device's default period.",
            Self::Balanced => "Request ~10 ms device buffers (512 frames at 48 kHz).",
            Self::Low => "Request ~5 ms device buffers (256 frames at 48 kHz).",
        }
    }

    /// Buffer request a profile maps to when no explicit per-device or global
    /// buffer is configured. Professional native admission only accepts
    /// power-of-two frames, so the mapping stays on those presets.
    pub const fn requested_buffer_frames(self) -> Option<u32> {
        match self {
            Self::Compatible => None,
            Self::Balanced => Some(512),
            Self::Low => Some(256),
        }
    }

    #[cfg(windows)]
    pub fn windows_backend_config(
        self,
    ) -> moiren_windows_audio::engine_backend::WindowsBackendConfig {
        use moiren_windows_audio::engine_backend::{NativeLatencyProfile, WindowsBackendConfig};
        WindowsBackendConfig {
            latency_profile: match self {
                Self::Compatible => NativeLatencyProfile::Compatible,
                Self::Balanced => NativeLatencyProfile::Balanced,
                Self::Low => NativeLatencyProfile::Low,
            },
            ..WindowsBackendConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_have_stable_saved_names_and_compatible_default() {
        assert_eq!(
            AudioLatencyProfile::default(),
            AudioLatencyProfile::Compatible
        );
        for (profile, saved) in
            AudioLatencyProfile::ALL
                .into_iter()
                .zip(["\"compatible\"", "\"balanced\"", "\"low\""])
        {
            assert_eq!(serde_json::to_string(&profile).unwrap(), saved);
            assert_eq!(
                serde_json::from_str::<AudioLatencyProfile>(saved).unwrap(),
                profile
            );
        }
        assert!(serde_json::from_str::<AudioLatencyProfile>("\"turbo\"").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn saved_profiles_configure_the_matching_native_preparation_policy() {
        use moiren_windows_audio::engine_backend::NativeLatencyProfile;
        for (profile, native) in AudioLatencyProfile::ALL.into_iter().zip([
            NativeLatencyProfile::Compatible,
            NativeLatencyProfile::Balanced,
            NativeLatencyProfile::Low,
        ]) {
            let config = profile.windows_backend_config();
            assert_eq!(config.latency_profile, native);
            config.validate().unwrap();
        }
    }

    #[test]
    fn profiles_map_to_power_of_two_buffer_requests() {
        assert_eq!(
            AudioLatencyProfile::Compatible.requested_buffer_frames(),
            None
        );
        assert_eq!(
            AudioLatencyProfile::Balanced.requested_buffer_frames(),
            Some(512)
        );
        assert_eq!(
            AudioLatencyProfile::Low.requested_buffer_frames(),
            Some(256)
        );
    }
}
