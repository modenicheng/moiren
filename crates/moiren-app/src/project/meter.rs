//! Global meter presentation choices, independent of project audio processing.
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterFalloff {
    Moderate,
    #[default]
    Fast,
    VeryFast,
}

impl MeterFalloff {
    pub const ALL: [Self; 3] = [Self::Moderate, Self::Fast, Self::VeryFast];
    /// Broadcast-PPM-grade at the slow end; the default clears a full
    /// 60 dB fall in 1.5 s so bars read at modern DAW speed.
    pub fn db_per_second(self) -> f32 {
        match self {
            Self::Moderate => 20.,
            Self::Fast => 40.,
            Self::VeryFast => 60.,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeakHold {
    Off,
    #[default]
    HalfSecond,
    OneSecond,
    TwoSeconds,
}

impl PeakHold {
    pub const ALL: [Self; 4] = [
        Self::Off,
        Self::HalfSecond,
        Self::OneSecond,
        Self::TwoSeconds,
    ];
    pub fn duration(self) -> Duration {
        Duration::from_millis(match self {
            Self::Off => 0,
            Self::HalfSecond => 500,
            Self::OneSecond => 1000,
            Self::TwoSeconds => 2000,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeterDisplayPreferences {
    pub falloff: MeterFalloff,
    pub peak_hold: PeakHold,
}

#[cfg(test)]
mod tests {
    use super::{MeterDisplayPreferences, MeterFalloff, PeakHold};
    use crate::project::AppPreferences;

    #[test]
    fn response_presets_round_trip_and_missing_preferences_keep_defaults() {
        for falloff in MeterFalloff::ALL {
            for peak_hold in PeakHold::ALL {
                let preferences = AppPreferences {
                    meters: MeterDisplayPreferences { falloff, peak_hold },
                    ..AppPreferences::default()
                };
                let json = serde_json::to_string(&preferences).unwrap();
                assert_eq!(
                    serde_json::from_str::<AppPreferences>(&json).unwrap(),
                    preferences
                );
            }
        }
        let mut older = serde_json::to_value(AppPreferences::default()).unwrap();
        older.as_object_mut().unwrap().remove("meters");
        assert_eq!(
            serde_json::from_value::<AppPreferences>(older)
                .unwrap()
                .meters,
            MeterDisplayPreferences::default()
        );
    }
}
