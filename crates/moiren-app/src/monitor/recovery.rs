use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MonitorFaults {
    /// Processed output timeline, including source priming and silence.
    pub output_frames: u64,
    pub priming_frames: u64,
    pub underrun_frames: u64,
    pub dropped_frames: u64,
    pub resets: u64,
    pub backlog_recoveries: u64,
    pub age_recoveries: u64,
    pub discontinuities: u64,
}
impl MonitorFaults {
    fn delta(self, previous: Self) -> Option<Self> {
        Some(Self {
            output_frames: self.output_frames.checked_sub(previous.output_frames)?,
            priming_frames: self.priming_frames.checked_sub(previous.priming_frames)?,
            underrun_frames: self.underrun_frames.checked_sub(previous.underrun_frames)?,
            dropped_frames: self.dropped_frames.checked_sub(previous.dropped_frames)?,
            resets: self.resets.checked_sub(previous.resets)?,
            backlog_recoveries: self
                .backlog_recoveries
                .checked_sub(previous.backlog_recoveries)?,
            age_recoveries: self.age_recoveries.checked_sub(previous.age_recoveries)?,
            discontinuities: self.discontinuities.checked_sub(previous.discontinuities)?,
        })
    }
    fn has_fault(self) -> bool {
        self.priming_frames != 0
            || self.underrun_frames != 0
            || self.dropped_frames != 0
            || self.resets != 0
            || self.backlog_recoveries != 0
            || self.age_recoveries != 0
            || self.discontinuities != 0
    }
    fn accumulate(&mut self, delta: Self) {
        self.output_frames = self.output_frames.saturating_add(delta.output_frames);
        self.priming_frames = self.priming_frames.saturating_add(delta.priming_frames);
        self.underrun_frames = self.underrun_frames.saturating_add(delta.underrun_frames);
        self.dropped_frames = self.dropped_frames.saturating_add(delta.dropped_frames);
        self.resets = self.resets.saturating_add(delta.resets);
        self.backlog_recoveries = self
            .backlog_recoveries
            .saturating_add(delta.backlog_recoveries);
        self.age_recoveries = self.age_recoveries.saturating_add(delta.age_recoveries);
        self.discontinuities = self.discontinuities.saturating_add(delta.discontinuities);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRecoveryPolicy {
    pub startup_grace: Duration,
    pub window: Duration,
    pub max_observation_gap: Duration,
    pub consecutive_windows: u32,
    pub fade_frames: u32,
    pub fade_timeout: Duration,
}
impl Default for MonitorRecoveryPolicy {
    fn default() -> Self {
        Self {
            startup_grace: Duration::from_secs(1),
            window: Duration::from_millis(250),
            max_observation_gap: Duration::from_secs(1),
            consecutive_windows: 3,
            fade_frames: 240,
            fade_timeout: Duration::from_secs(1),
        }
    }
}
impl MonitorRecoveryPolicy {
    pub fn validate(self) -> Result<(), MonitorRecoveryError> {
        if self.window.is_zero()
            || self.window > Duration::from_secs(60)
            || self.startup_grace > Duration::from_secs(600)
            || self.max_observation_gap < self.window
            || self.max_observation_gap > Duration::from_secs(600)
            || !(1..=1000).contains(&self.consecutive_windows)
            || !(1..=48_000).contains(&self.fade_frames)
            || self.fade_timeout.is_zero()
            || self.fade_timeout > Duration::from_secs(10)
        {
            return Err(MonitorRecoveryError::InvalidPolicy);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MonitorRecoveryError {
    #[error("invalid or unbounded monitor recovery policy")]
    InvalidPolicy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRecoveryReason {
    pub consecutive_windows: u32,
    pub faults: MonitorFaults,
}

pub struct MonitorRecovery {
    policy: MonitorRecoveryPolicy,
    baseline: Option<(Duration, MonitorFaults)>,
    last_observation: Option<Duration>,
    streak: u32,
    faults: MonitorFaults,
    triggered: bool,
}
impl MonitorRecovery {
    pub fn new(policy: MonitorRecoveryPolicy) -> Result<Self, MonitorRecoveryError> {
        policy.validate()?;
        Ok(Self {
            policy,
            baseline: None,
            last_observation: None,
            streak: 0,
            faults: MonitorFaults::default(),
            triggered: false,
        })
    }
    fn reset(&mut self, now: Duration, faults: MonitorFaults) {
        self.baseline = Some((now, faults));
        self.streak = 0;
        self.faults = MonitorFaults::default();
    }
    pub fn observe(
        &mut self,
        now: Duration,
        faults: MonitorFaults,
    ) -> Option<MonitorRecoveryReason> {
        if self.triggered {
            return None;
        }
        let gap = self
            .last_observation
            .and_then(|previous| now.checked_sub(previous));
        let invalid_gap = self.last_observation.is_some()
            && gap.is_none_or(|gap| gap > self.policy.max_observation_gap);
        self.last_observation = Some(now);
        if now < self.policy.startup_grace || invalid_gap || self.baseline.is_none() {
            self.reset(now, faults);
            return None;
        }
        let (started, previous) = self.baseline?;
        if started < self.policy.startup_grace {
            self.reset(now, faults);
            return None;
        }
        let Some(delta) = faults.delta(previous) else {
            self.reset(now, faults);
            return None;
        };
        if now.saturating_sub(started) < self.policy.window {
            return None;
        }
        self.baseline = Some((now, faults));
        if delta.output_frames == 0 || !delta.has_fault() {
            self.reset(now, faults);
            return None;
        }
        self.streak += 1;
        self.faults.accumulate(delta);
        if self.streak < self.policy.consecutive_windows {
            return None;
        }
        self.triggered = true;
        Some(MonitorRecoveryReason {
            consecutive_windows: self.streak,
            faults: self.faults,
        })
    }
}
