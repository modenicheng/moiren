use moiren_windows_audio::{DurationError, SessionDuration};
use std::time::Duration;

#[test]
fn unlimited_run_has_no_artificial_deadline() {
    let limit = SessionDuration::UntilStopped;
    assert_eq!(limit.validate(), Ok(()));
    assert_eq!(limit.remaining(Duration::from_secs(3600)), None);
    assert_eq!(limit.requested_seconds(), None);
    for duration in [
        Duration::ZERO,
        Duration::from_millis(999),
        Duration::from_secs(601),
    ] {
        assert_eq!(
            SessionDuration::For(duration).validate(),
            Err(DurationError::OutOfRange)
        );
    }
}

#[test]
fn finite_duration_preserves_boundaries_and_saturates() {
    for seconds in [1, 600] {
        let limit = SessionDuration::For(Duration::from_secs(seconds));
        assert_eq!(limit.validate(), Ok(()));
        assert_eq!(limit.requested_seconds(), Some(seconds as f64));
        assert_eq!(
            limit.remaining(Duration::ZERO),
            Some(Duration::from_secs(seconds))
        );
        assert_eq!(
            limit.remaining(Duration::from_secs(601)),
            Some(Duration::ZERO)
        );
    }
}
