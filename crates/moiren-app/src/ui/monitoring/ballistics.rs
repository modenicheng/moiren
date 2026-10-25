//! Elapsed-time sample-peak presentation. Only received windows renew hold.
use super::{ChannelLevel, DisplayLevel, Duration, Instant};
use crate::project::MeterDisplayPreferences;

pub(super) const RMS_ATTACK: Duration = Duration::from_millis(150);
pub(super) const RMS_RELEASE: Duration = Duration::from_millis(700);
#[cfg(test)]
pub(super) const PEAK_HOLD: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub(super) struct ChannelBallistics {
    bar: f32,
    rms: f32,
    marker: f32,
    target: ChannelLevel,
    held_at: Option<Instant>,
    ticked_at: Option<Instant>,
    pub settings: MeterDisplayPreferences,
}

impl ChannelBallistics {
    pub fn new() -> Self {
        Self {
            bar: 0.,
            rms: 0.,
            marker: 0.,
            target: ChannelLevel { peak: 0., rms: 0. },
            held_at: None,
            ticked_at: None,
            settings: MeterDisplayPreferences::default(),
        }
    }

    pub fn observe(&mut self, raw: ChannelLevel, now: Instant) {
        self.target = raw;
        if self.ticked_at.is_none() {
            self.bar = raw.peak;
            self.rms = raw.rms;
            self.marker = raw.peak;
            self.ticked_at = Some(now);
        } else {
            self.advance(now);
        }
        if raw.peak >= self.marker {
            self.marker = raw.peak;
            self.held_at = Some(now);
        }
    }

    pub fn advance(&mut self, now: Instant) {
        let Some(previous) = self.ticked_at else {
            return;
        };
        let dt = now.saturating_duration_since(previous);
        let tau = if self.target.rms >= self.rms {
            RMS_ATTACK
        } else {
            RMS_RELEASE
        };
        self.rms +=
            (self.target.rms - self.rms) * (1. - (-dt.as_secs_f32() / tau.as_secs_f32()).exp());
        let release = self.settings.falloff.db_per_second();
        self.bar = self.target.peak.max(fall(self.bar, dt, release));
        // Resolve the deadline from the current preference so shortening or
        // disabling hold affects an already visible marker on the next frame.
        let anchor = self
            .held_at
            .map(|held_at| held_at + self.settings.peak_hold.duration())
            .unwrap_or(previous)
            .max(previous);
        self.marker =
            fall(self.marker, now.saturating_duration_since(anchor), release).max(self.bar);
        self.ticked_at = Some(now);
    }

    pub fn level(&self) -> DisplayLevel {
        DisplayLevel {
            bar: self.bar,
            marker: self.marker,
            rms: self.rms,
        }
    }
}

fn fall(value: f32, dt: Duration, db_per_second: f32) -> f32 {
    let value = value * 10f32.powf(-db_per_second * dt.as_secs_f32() / 20.);
    if value < 1e-6 { 0. } else { value }
}

#[cfg(test)]
mod tests {
    use super::{ChannelBallistics, ChannelLevel, Duration, Instant};

    #[test]
    fn cached_frames_do_not_renew_hold_and_deadlines_use_partial_intervals() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.observe(ChannelLevel { peak: 1., rms: 0. }, now);
        meter.observe(
            ChannelLevel { peak: 0., rms: 0. },
            now + Duration::from_millis(10),
        );
        for ms in [100, 250, 490, 600] {
            meter.advance(now + Duration::from_millis(ms));
        }
        assert!((20. * meter.level().marker.log10() + 2.).abs() < 0.001);
        assert!((20. * meter.level().bar.log10() + 12.).abs() < 0.001);
    }

    #[test]
    fn disabling_hold_releases_an_existing_marker_on_the_next_frame() {
        let now = Instant::now();
        let mut meter = ChannelBallistics::new();
        meter.observe(ChannelLevel { peak: 1., rms: 0. }, now);
        meter.observe(ChannelLevel { peak: 0., rms: 0. }, now);
        meter.settings.peak_hold = crate::project::PeakHold::Off;
        meter.advance(now + Duration::from_millis(100));
        assert!((meter.level().marker - meter.level().bar).abs() < 0.00001);
    }

    #[test]
    fn decay_is_independent_of_refresh_rate_and_irregular_frames() {
        let now = Instant::now();
        let run = |times: Vec<u64>| {
            let mut meter = ChannelBallistics::new();
            meter.observe(ChannelLevel { peak: 1., rms: 0.5 }, now);
            meter.observe(ChannelLevel { peak: 0., rms: 0. }, now);
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
}
