//! Meter-node settings and implicit UI window sizing. The demand-gated
//! sample-peak/RMS measurement kernel lives in [`signal`]; it is NOT a
//! BS.1770/LUFS or true-peak meter. The integration window delays the
//! display, never the audio signal.

mod polling;
pub mod signal;
pub use polling::MeterPollingTimer;

/// Default explicit Meter-node integration window: 480 frames is 10 ms at
/// 48 kHz. Implicit UI observers use [`ui_window_frames`] instead. Analysis
/// windows delay observations, never audio.
pub const DEFAULT_WINDOW_FRAMES: usize = 480;

/// Implicit UI observers target at least 240 analysis windows per audio second.
/// Derive this off the audio thread from the validated processing rate; explicit
/// Meter-node settings keep their chosen integration interval instead.
pub fn ui_window_frames(processing_sample_rate: f64) -> usize {
    ((processing_sample_rate / 240.0).floor() as usize).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeterSettings {
    pub window_frames: usize,
}
impl Default for MeterSettings {
    fn default() -> Self {
        Self {
            window_frames: DEFAULT_WINDOW_FRAMES,
        }
    }
}
impl MeterSettings {
    pub fn valid(self) -> bool {
        (1..=1_048_576).contains(&self.window_frames)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn implicit_ui_windows_floor_the_rate_with_at_least_one_frame() {
        for (rate, frames) in [
            (1.0, 1),
            (239.0, 1),
            (240.0, 1),
            (44_100.0, 183),
            (48_000.0, 200),
            (96_000.0, 400),
        ] {
            assert_eq!(super::ui_window_frames(rate), frames);
        }
        assert_eq!(super::MeterSettings::default().window_frames, 480);
    }
}
