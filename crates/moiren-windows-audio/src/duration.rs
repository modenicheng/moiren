//! Explicit continuous sessions keep native stop events as the only deadline.
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDuration {
    UntilStopped,
    For(Duration),
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn native_timeout_is_infinite_or_rounded_remaining_time() {
        assert_eq!(
            SessionDuration::UntilStopped.timeout_ms(Duration::MAX),
            u32::MAX
        );
        let finite = SessionDuration::For(Duration::from_secs(1));
        assert_eq!(finite.timeout_ms(Duration::ZERO), 1000);
        assert_eq!(finite.timeout_ms(Duration::from_nanos(999_999_999)), 1);
        assert_eq!(finite.timeout_ms(Duration::from_secs(1)), 0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    OutOfRange,
}

impl SessionDuration {
    pub fn validate(self) -> Result<(), DurationError> {
        match self {
            Self::UntilStopped => Ok(()),
            Self::For(d) if (Duration::from_secs(1)..=Duration::from_secs(600)).contains(&d) => {
                Ok(())
            }
            Self::For(_) => Err(DurationError::OutOfRange),
        }
    }
    pub fn remaining(self, elapsed: Duration) -> Option<Duration> {
        match self {
            Self::UntilStopped => None,
            Self::For(d) => Some(d.saturating_sub(elapsed)),
        }
    }
    pub fn requested_seconds(self) -> Option<f64> {
        match self {
            Self::UntilStopped => None,
            Self::For(d) => Some(d.as_secs_f64()),
        }
    }
    #[cfg(windows)]
    pub(crate) fn timeout_ms(self, elapsed: Duration) -> u32 {
        // Round up so a sub-millisecond remainder never becomes a busy wait.
        self.remaining(elapsed).map_or(u32::MAX, |d| {
            d.as_millis()
                .saturating_add(u128::from(d.subsec_nanos() % 1_000_000 != 0))
                .min(u128::from(u32::MAX - 1)) as u32
        })
    }
}
