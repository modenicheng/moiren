//! Finite control polling. Callers must not put sink IO in the poll closure.

use anyhow::Result;
use std::time::{Duration, Instant};

const CONTROL_POLL: Duration = Duration::from_millis(10);

pub(super) fn run_until(
    duration: Duration,
    mut poll: impl FnMut(Duration) -> Result<()>,
    stop: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    run_with_clock(
        duration,
        &mut poll,
        stop,
        || started.elapsed(),
        std::thread::sleep,
    )
}

fn run_with_clock(
    duration: Duration,
    mut poll: impl FnMut(Duration) -> Result<()>,
    stop: impl FnOnce() -> Result<()>,
    elapsed: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> Result<()> {
    let collection = (|| {
        while elapsed() < duration {
            poll(elapsed())?;
            // Poll may do bounded serialization/snapshot work. Never sleep
            // using the timestamp captured before that work.
            sleep(CONTROL_POLL.min(duration.saturating_sub(elapsed())));
        }
        Ok(())
    })();
    let stopped = stop();
    super::soak_cleanup::combine(collection, stopped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::mpsc, time::Instant};

    #[test]
    fn soak_control_uses_fresh_time_after_poll_crosses_deadline() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut polls = 0;
        let mut sleeps = Vec::new();
        run_with_clock(
            Duration::from_millis(20),
            |_| {
                polls += 1;
                elapsed.set(Duration::from_millis(40));
                Ok(())
            },
            || Ok(()),
            || elapsed.get(),
            |duration| sleeps.push(duration),
        )
        .unwrap();
        assert_eq!(polls, 1);
        assert_eq!(
            sleeps,
            [Duration::ZERO],
            "stale time added sleep after deadline"
        );
    }

    #[test]
    fn soak_control_waits_for_finite_deadline_and_cancels_after_poll_error() {
        let started = Instant::now();
        let duration = Duration::from_millis(40);
        run_until(duration, |_| Ok(()), || Ok(())).unwrap();
        assert!(
            started.elapsed() >= duration,
            "deadline must not stop immediately"
        );
        let (tx, rx) = mpsc::sync_channel(1);
        let error = run_until(
            duration,
            |_| anyhow::bail!("collection IO failure"),
            || {
                tx.send(()).unwrap();
                anyhow::bail!("cancellation failure")
            },
        )
        .unwrap_err();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("stop skipped after poll error");
        let message = format!("{error:#}");
        assert!(message.contains("collection IO failure"));
        assert!(message.contains("cancellation failure"));
    }
}
