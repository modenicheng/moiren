use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTick {
    pub frames: usize,
    pub skipped_periods: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    InvalidConfiguration,
    CounterRegressed,
    Overflow,
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "virtual realtime clock: {self:?}")
    }
}
impl std::error::Error for ClockError {}

pub struct ClockSchedule {
    frequency: u64,
    sample_rate: u64,
    frames: usize,
    origin: u64,
    next: u64,
    last: u64,
}

impl ClockSchedule {
    pub fn new(
        frequency: u64,
        frames: usize,
        sample_rate: u64,
        now: u64,
    ) -> Result<Self, ClockError> {
        if frequency == 0 || frames == 0 || sample_rate == 0 {
            return Err(ClockError::InvalidConfiguration);
        }
        Ok(Self {
            frequency,
            sample_rate,
            frames,
            origin: now,
            next: 1,
            last: now,
        })
    }

    pub fn due(&mut self, now: u64) -> Result<Option<ClockTick>, ClockError> {
        self.observe(now)?;
        if now < self.deadline()? {
            return Ok(None);
        }
        let elapsed = self.elapsed_periods(now)?;
        let skipped_periods = elapsed.saturating_sub(self.next);
        self.next = elapsed.checked_add(1).ok_or(ClockError::Overflow)?;
        Ok(Some(ClockTick {
            frames: self.frames,
            skipped_periods,
        }))
    }

    /// Recheck after DSP finishes. Missed wall-clock periods are discarded,
    /// never recursively executed. Engine frames advance only for actual work.
    pub fn skip_past(&mut self, now: u64) -> Result<u64, ClockError> {
        self.observe(now)?;
        let previous = self.next;
        self.next = self.next.max(
            self.elapsed_periods(now)?
                .checked_add(1)
                .ok_or(ClockError::Overflow)?,
        );
        Ok(self.next - previous)
    }

    pub fn remaining_ticks(&mut self, now: u64) -> Result<u64, ClockError> {
        self.observe(now)?;
        Ok(self.deadline()?.saturating_sub(now))
    }

    fn observe(&mut self, now: u64) -> Result<(), ClockError> {
        if now < self.last {
            return Err(ClockError::CounterRegressed);
        }
        self.last = now;
        Ok(())
    }

    fn period_numerator(&self) -> Result<u128, ClockError> {
        (self.frames as u128)
            .checked_mul(self.frequency.into())
            .ok_or(ClockError::Overflow)
    }

    fn deadline(&self) -> Result<u64, ClockError> {
        let offset = u128::from(self.next)
            .checked_mul(self.period_numerator()?)
            .ok_or(ClockError::Overflow)?
            .div_ceil(self.sample_rate.into());
        self.origin
            .checked_add(u64::try_from(offset).map_err(|_| ClockError::Overflow)?)
            .ok_or(ClockError::Overflow)
    }

    fn elapsed_periods(&self, now: u64) -> Result<u64, ClockError> {
        let elapsed = u128::from(now - self.origin) * u128::from(self.sample_rate);
        u64::try_from(elapsed / self.period_numerator()?).map_err(|_| ClockError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_due_wake_renders_one_block_even_after_a_long_stall() {
        let mut schedule = ClockSchedule::new(10_000_000, 480, 48_000, 0).unwrap();
        assert_eq!(schedule.due(99_999).unwrap(), None);
        assert_eq!(
            schedule.due(100_000).unwrap(),
            Some(ClockTick {
                frames: 480,
                skipped_periods: 0
            })
        );
        assert_eq!(
            schedule.due(10_000_000).unwrap(),
            Some(ClockTick {
                frames: 480,
                skipped_periods: 98
            })
        );
        assert_eq!(schedule.due(10_000_000).unwrap(), None);
    }

    #[test]
    fn render_stalls_skip_wall_clock_work_and_always_leave_a_future_deadline() {
        let mut schedule = ClockSchedule::new(10_000_000, 480, 48_000, 0).unwrap();
        schedule.due(100_000).unwrap().unwrap();
        assert_eq!(schedule.skip_past(9_000_001).unwrap(), 89);
        assert_eq!(schedule.due(9_000_001).unwrap(), None);
        assert_eq!(schedule.remaining_ticks(9_000_001).unwrap(), 99_999);
        assert_eq!(schedule.due(9_100_000).unwrap().unwrap().frames, 480);
    }

    #[test]
    fn fractional_periods_do_not_accumulate_rounding_drift() {
        let mut schedule = ClockSchedule::new(10_000_000, 128, 48_000, 7).unwrap();
        assert_eq!(schedule.remaining_ticks(7).unwrap(), 26_667);
        for block in 1..=3000_u64 {
            let now = 7 + (u128::from(block) * 128 * 10_000_000).div_ceil(48_000) as u64;
            assert_eq!(schedule.due(now - 1).unwrap(), None);
            assert_eq!(schedule.due(now).unwrap().unwrap().skipped_periods, 0);
        }
        assert_eq!(schedule.remaining_ticks(80_000_007).unwrap(), 26_667);
    }

    #[test]
    fn invalid_and_regressing_counters_fail_instead_of_selecting_another_clock() {
        assert!(ClockSchedule::new(0, 480, 48_000, 0).is_err());
        assert!(ClockSchedule::new(10_000_000, 0, 48_000, 0).is_err());
        assert!(ClockSchedule::new(10_000_000, 480, 0, 0).is_err());
        let mut schedule = ClockSchedule::new(10_000_000, 480, 48_000, 100).unwrap();
        assert_eq!(schedule.due(99), Err(ClockError::CounterRegressed));
        let mut schedule = ClockSchedule::new(u64::MAX, usize::MAX, 48_000, u64::MAX).unwrap();
        assert_eq!(
            schedule.remaining_ticks(u64::MAX),
            Err(ClockError::Overflow)
        );
    }

    #[test]
    fn deadlines_follow_engine_rate_instead_of_assuming_48khz() {
        let mut schedule = ClockSchedule::new(10_000_000, 441, 44_100, 0).unwrap();
        assert_eq!(schedule.remaining_ticks(0).unwrap(), 100_000);
        assert_eq!(schedule.due(99_999).unwrap(), None);
        assert_eq!(schedule.due(100_000).unwrap().unwrap().frames, 441);
        assert_eq!(schedule.remaining_ticks(100_000).unwrap(), 100_000);
    }
}
