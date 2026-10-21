use std::fmt;

/// One due wake of the virtual realtime schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTick {
    pub frames: usize,
    /// Periods due at this wake including the one about to render. Late
    /// periods stay due, so the owner repays them by rendering one block per
    /// immediate redue instead of discarding the missed work.
    pub backlog_periods: u64,
    /// Periods dropped to keep the repayable debt within
    /// [`DEBT_CAP_PERIODS`]: permanently unrendered frames. An honest,
    /// counted loss — never a silent rate bias on the boundary carriers.
    pub clamped_periods: u64,
}

/// Repayable debt ceiling in periods. A late wake repays its missed periods
/// by rendering them back-to-back; debt beyond this cap is discarded and
/// reported so the owner can escalate. The cap must stay below the boundary
/// carriers' servo target headroom (2255 frames at the VAD shape) so a full
/// repayment burst cannot itself starve or flood a carrier: 8 Graph256
/// periods are 2048 frames (~42.7 ms).
pub const DEBT_CAP_PERIODS: u64 = 8;

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

    /// One due wake consumes exactly one period and reports the backlog.
    /// Late periods stay due: calling again at the same instant returns the
    /// next overdue period until the debt is repaid or clamped, so rendered
    /// frames keep pace with the wall clock that paces the native side. The
    /// previous design jumped `next` past every missed period — each skip
    /// was a permanent mic-side production deficit and playback-side surplus
    /// at once, which drained both carriers monotonically.
    pub fn due(&mut self, now: u64) -> Result<Option<ClockTick>, ClockError> {
        self.observe(now)?;
        if now < self.deadline()? {
            return Ok(None);
        }
        let elapsed = self.elapsed_periods(now)?;
        let mut backlog = elapsed.saturating_sub(self.next - 1);
        let mut clamped = 0;
        if backlog > DEBT_CAP_PERIODS {
            clamped = backlog - DEBT_CAP_PERIODS;
            self.next = self.next.checked_add(clamped).ok_or(ClockError::Overflow)?;
            backlog = DEBT_CAP_PERIODS;
        }
        self.next = self.next.checked_add(1).ok_or(ClockError::Overflow)?;
        Ok(Some(ClockTick {
            frames: self.frames,
            backlog_periods: backlog,
            clamped_periods: clamped,
        }))
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
    fn long_stall_clamps_debt_to_the_cap_and_renders_it_back_to_back() {
        let mut schedule = ClockSchedule::new(10_000_000, 480, 48_000, 0).unwrap();
        assert_eq!(schedule.due(99_999).unwrap(), None);
        assert_eq!(
            schedule.due(100_000).unwrap(),
            Some(ClockTick {
                frames: 480,
                backlog_periods: 1,
                clamped_periods: 0
            })
        );
        let tick = schedule.due(10_000_000).unwrap().unwrap();
        assert_eq!(tick.backlog_periods, DEBT_CAP_PERIODS);
        assert_eq!(tick.clamped_periods, 91);
        for backlog in (1..DEBT_CAP_PERIODS).rev() {
            assert_eq!(
                schedule.due(10_000_000).unwrap(),
                Some(ClockTick {
                    frames: 480,
                    backlog_periods: backlog,
                    clamped_periods: 0
                })
            );
        }
        assert_eq!(schedule.due(10_000_000).unwrap(), None);
        assert!(schedule.remaining_ticks(10_000_000).unwrap() > 0);
    }

    /// The anti-bias invariant: periodic late wakes must repay in full, so
    /// the rendered timeline stays exactly on wall clock. The discarded-skip
    /// design lost every skipped period permanently (−1791 ppm on the
    /// failed target run) and drained both carriers monotonically.
    #[test]
    fn periodic_late_wakes_repay_debt_with_zero_long_term_drift() {
        let frequency = 10_000_000_u64;
        let frames = 256_u64;
        let rate = 48_000_u64;
        let deadline = |period: u64| -> u64 {
            u64::try_from(
                (u128::from(period) * u128::from(frames) * u128::from(frequency))
                    .div_ceil(u128::from(rate)),
            )
            .unwrap()
        };
        let mut schedule = ClockSchedule::new(frequency, frames as usize, rate, 0).unwrap();
        let total_periods = 3_000_u64;
        let mut rendered_periods = 0_u64;
        let mut clamped_periods = 0_u64;
        let mut period = 1_u64;
        while period <= total_periods {
            let late = u64::from(period.is_multiple_of(64)) * 2;
            let now = deadline(period + late);
            while let Some(tick) = schedule.due(now).unwrap() {
                rendered_periods += 1;
                clamped_periods += tick.clamped_periods;
            }
            period += 1 + late;
        }
        assert_eq!(clamped_periods, 0, "two-period jitter never clamps");
        assert_eq!(
            rendered_periods, total_periods,
            "every wall-clock period must render exactly once"
        );
        assert!(schedule.remaining_ticks(deadline(total_periods)).unwrap() > 0);
    }

    #[test]
    fn fractional_periods_do_not_accumulate_rounding_drift() {
        let mut schedule = ClockSchedule::new(10_000_000, 128, 48_000, 7).unwrap();
        assert_eq!(schedule.remaining_ticks(7).unwrap(), 26_667);
        for block in 1..=3000_u64 {
            let now = 7 + (u128::from(block) * 128 * 10_000_000).div_ceil(48_000) as u64;
            assert_eq!(schedule.due(now - 1).unwrap(), None);
            let tick = schedule.due(now).unwrap().unwrap();
            assert_eq!((tick.backlog_periods, tick.clamped_periods), (1, 0));
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
