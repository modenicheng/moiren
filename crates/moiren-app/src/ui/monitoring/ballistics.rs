//! Elapsed-time peak/RMS presentation in the decibel domain.
//!
//! Every layer is driven by the user's meter preferences: the peak bar rises
//! instantly and falls at the selected dB/s, the marker holds the recent
//! maximum for the selected duration before falling at the same rate, and the
//! RMS body integrates mean square across every ingested window (so the
//! reading is a true running average instead of a point sample) and then
//! obeys the same fall-off preference. Peaks accumulate between display
//! reads, so a transient that has already decayed before the next frame is
//! still shown at full height.
use super::{ChannelLevel, DisplayLevel, Duration, Instant};
use crate::project::MeterDisplayPreferences;

/// One-pole mean-square integration across ingested windows. The natural
/// decay in silence is ~58 dB/s, above every fall-off preference, so the
/// visible body fall always follows the preference instead of the integrator.
pub(super) const RMS_INTEGRATION: Duration = Duration::from_millis(150);
#[cfg(test)]
pub(super) const PEAK_HOLD: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub(super) struct ChannelBallistics {
    bar_db: f32,
    marker_db: f32,
    body_db: f32,
    mean_square: f64,
    peak_since_read: f32,
    held_at: Option<Instant>,
    ticked_at: Option<Instant>,
    pub settings: MeterDisplayPreferences,
}

impl ChannelBallistics {
    pub fn new() -> Self {
        Self {
            bar_db: f32::NEG_INFINITY,
            marker_db: f32::NEG_INFINITY,
            body_db: f32::NEG_INFINITY,
            mean_square: 0.,
            peak_since_read: 0.,
            held_at: None,
            ticked_at: None,
            settings: MeterDisplayPreferences::default(),
        }
    }

    /// Consume one measurement window, weighted by its true duration.
    pub fn ingest(&mut self, raw: ChannelLevel, window: Duration, now: Instant) {
        self.peak_since_read = self.peak_since_read.max(raw.peak);
        let fraction = 1. - (-window.as_secs_f64() / RMS_INTEGRATION.as_secs_f64()).exp();
        let square = f64::from(raw.rms) * f64::from(raw.rms);
        self.mean_square += (square - self.mean_square) * fraction;
        let peak_db = amplitude_db(raw.peak);
        if peak_db >= self.marker_db {
            self.marker_db = peak_db;
            self.held_at = Some(now);
        }
    }

    pub fn advance(&mut self, now: Instant) {
        let Some(previous) = self.ticked_at else {
            // First data after (re)subscription: snap every layer to it.
            self.bar_db = amplitude_db(self.peak_since_read);
            self.marker_db = self.marker_db.max(self.bar_db);
            self.body_db = mean_square_db(self.mean_square);
            self.peak_since_read = 0.;
            self.ticked_at = Some(now);
            return;
        };
        let dt = now.saturating_duration_since(previous).as_secs_f32();
        let rate = self.settings.falloff.db_per_second();
        let release = rate * dt;
        let target = amplitude_db(self.peak_since_read);
        self.bar_db = target.max(self.bar_db - release);
        // Resolve the deadline from the current preference so shortening or
        // disabling hold affects an already visible marker on the next frame.
        let anchor = self
            .held_at
            .map(|held_at| held_at + self.settings.peak_hold.duration())
            .unwrap_or(previous)
            .max(previous);
        let held = now.saturating_duration_since(anchor).as_secs_f32();
        self.marker_db = (self.marker_db - rate * held).max(self.bar_db);
        let integrated = mean_square_db(self.mean_square);
        self.body_db = integrated.max(self.body_db - release);
        self.peak_since_read = 0.;
        self.ticked_at = Some(now);
    }

    pub fn level(&self) -> DisplayLevel {
        DisplayLevel {
            bar: db_to_amplitude(self.bar_db),
            marker: db_to_amplitude(self.marker_db),
            rms: db_to_amplitude(self.body_db),
        }
    }
}

fn amplitude_db(amplitude: f32) -> f32 {
    if amplitude > 0. {
        20. * amplitude.log10()
    } else {
        f32::NEG_INFINITY
    }
}

fn mean_square_db(mean_square: f64) -> f32 {
    if mean_square > 0. {
        10. * (mean_square.log10() as f32)
    } else {
        f32::NEG_INFINITY
    }
}

fn db_to_amplitude(db: f32) -> f32 {
    if db.is_finite() {
        10f32.powf(db / 20.)
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelBallistics, ChannelLevel, Duration, Instant, PEAK_HOLD};
    use crate::project::{MeterFalloff, PeakHold};

    const WINDOW: Duration = Duration::from_millis(4);

    #[test]
    fn cached_frames_do_not_renew_hold_and_deadlines_use_partial_intervals() {
        let now = Instant::now();
        let rate = MeterFalloff::Fast.db_per_second();
        let mut meter = ChannelBallistics::new();
        meter.ingest(ChannelLevel { peak: 1., rms: 0. }, WINDOW, now);
        meter.advance(now);
        meter.ingest(ChannelLevel { peak: 0., rms: 0. }, WINDOW, now + WINDOW);
        for ms in [100, 250, 490, 600] {
            meter.advance(now + Duration::from_millis(ms));
        }
        // The marker holds until PEAK_HOLD, then falls 100 ms at the rate.
        let marker_db = 20. * meter.level().marker.log10();
        assert!((marker_db - (-rate * 0.1)).abs() < 0.001);
        // The bar has fallen for the full 600 ms without any hold.
        let bar_db = 20. * meter.level().bar.log10();
        assert!((bar_db - (-rate * 0.6)).abs() < 0.001);
    }

    #[test]
    fn disabling_hold_releases_an_existing_marker_on_the_next_frame() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.ingest(ChannelLevel { peak: 1., rms: 0. }, WINDOW, now);
        meter.advance(now);
        meter.settings.peak_hold = PeakHold::Off;
        meter.advance(now + Duration::from_millis(100));
        let level = meter.level();
        assert!(
            (level.marker - level.bar).abs() < 0.00001,
            "marker {level:?} tracks the bar once hold is off"
        );
    }

    #[test]
    fn decay_is_independent_of_refresh_rate_and_irregular_frames() {
        let now = Instant::now();
        let run = |times: Vec<u64>| {
            let mut meter = ChannelBallistics::new();
            meter.ingest(ChannelLevel { peak: 1., rms: 0.5 }, WINDOW, now);
            meter.advance(now);
            meter.ingest(ChannelLevel { peak: 0., rms: 0. }, WINDOW, now);
            for ns in times {
                meter.advance(now + Duration::from_nanos(ns));
            }
            meter.level()
        };
        let reference = run(vec![1_000_000_000]);
        for hz in [60, 120, 144, 240] {
            let actual = run((1..=hz).map(|frame| frame * 1_000_000_000 / hz).collect());
            assert!((actual.bar - reference.bar).abs() < 0.00001);
            assert!((actual.marker - reference.marker).abs() < 0.00001);
            assert!((actual.rms - reference.rms).abs() < 0.00001);
        }
        let irregular = run(vec![
            7_000_000,
            93_000_000,
            493_000_000,
            590_000_000,
            1_000_000_000,
        ]);
        assert!((irregular.marker - reference.marker).abs() < 0.00001);
    }

    #[test]
    fn peaks_between_reads_accumulate_so_no_transient_is_lost() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.ingest(ChannelLevel { peak: 1., rms: 0. }, WINDOW, now);
        meter.advance(now);
        // A full-scale transient that has already decayed by the next display
        // read still drives the bar and marker to its full height.
        meter.ingest(ChannelLevel { peak: 1., rms: 0. }, WINDOW, now);
        meter.ingest(ChannelLevel { peak: 0.1, rms: 0. }, WINDOW, now + WINDOW);
        meter.advance(now + Duration::from_millis(16));
        assert!((meter.level().bar - 1.).abs() < 0.00001);
        assert!((meter.level().marker - 1.).abs() < 0.00001);
    }

    #[test]
    fn rms_integrates_across_ingested_windows_like_a_moving_average() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        // One integration constant of constant full-scale RMS: the integrated
        // mean square covers 1 - 1/e of the step.
        meter.ingest(
            ChannelLevel { peak: 0., rms: 1. },
            super::RMS_INTEGRATION,
            now,
        );
        meter.advance(now);
        let expected = (1. - std::f64::consts::E.recip()).sqrt() as f32;
        assert!((meter.level().rms - expected).abs() < 0.0001);
    }

    #[test]
    fn rms_body_falls_at_the_selected_rate_in_silence() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        // Below the integrator's natural ~58 dB/s silence decay, so the
        // preference decides the visible fall.
        meter.settings.falloff = MeterFalloff::Moderate;
        for _ in 0..100 {
            meter.ingest(ChannelLevel { peak: 0., rms: 1. }, WINDOW, now);
        }
        meter.advance(now);
        let settled_db = 20. * meter.level().rms.log10();
        for _ in 0..100 {
            meter.ingest(
                ChannelLevel { peak: 0., rms: 0. },
                WINDOW,
                now + Duration::from_millis(250),
            );
        }
        meter.advance(now + Duration::from_millis(250));
        let db = 20. * meter.level().rms.log10();
        assert!(
            (db - (settled_db - 20. * 0.25)).abs() < 0.05,
            "fell to {db} dB from {settled_db} dB instead of 5 dB at 20 dB/s"
        );
    }

    #[test]
    fn bar_rises_instantly_while_the_marker_keeps_the_recent_maximum() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.ingest(ChannelLevel { peak: 0.5, rms: 0. }, WINDOW, now);
        meter.advance(now);
        meter.ingest(ChannelLevel { peak: 0.1, rms: 0. }, WINDOW, now + WINDOW);
        meter.advance(now + Duration::from_millis(100));
        let level = meter.level();
        assert!((level.marker - 0.5).abs() < 0.00001, "marker still holds");
        // No hold on the bar: attack is instant, but a quieter reading falls at
        // the preference falloff rate rather than dropping in one step.
        let rate = MeterFalloff::Fast.db_per_second();
        let expected_db = (-6.0206 - rate * 0.1).max(20. * 0.1f32.log10());
        assert!(
            (20. * level.bar.log10() - expected_db).abs() < 0.001,
            "bar = {:?}",
            level.bar
        );
        meter.ingest(ChannelLevel { peak: 0.8, rms: 0. }, WINDOW, now + WINDOW);
        meter.advance(now + Duration::from_millis(200));
        assert!((meter.level().bar - 0.8).abs() < 0.00001);
    }

    #[test]
    fn first_window_snaps_every_layer_then_falls() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.ingest(
            ChannelLevel {
                peak: 0.5,
                rms: 0.3,
            },
            WINDOW,
            now,
        );
        meter.advance(now);
        let level = meter.level();
        assert!((level.bar - 0.5).abs() < 0.000001);
        assert!((level.marker - 0.5).abs() < 0.000001);
        // The body integrates from silence: one 4 ms window covers only a
        // fraction of the integration constant, so it starts below the raw
        // window RMS and climbs as further windows arrive.
        assert!(level.rms > 0. && level.rms < 0.3);
        meter.advance(now + PEAK_HOLD);
        assert!(meter.level().bar < 0.5, "silence falls after the snap");
    }
}
