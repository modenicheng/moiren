use super::ActiveSession;
use crate::monitor::{MonitorRecovery, MonitorRecoveryPolicy, MonitorRecoveryReason};
use std::time::{Duration, Instant};

struct PendingFade {
    reason: MonitorRecoveryReason,
    requested: Instant,
    submitted: Option<Instant>,
}
pub(super) struct OwnerRecovery {
    detector: MonitorRecovery,
    policy: MonitorRecoveryPolicy,
    started: Option<Instant>,
    pending: Option<PendingFade>,
}
impl OwnerRecovery {
    pub fn new(policy: MonitorRecoveryPolicy) -> Self {
        Self {
            detector: MonitorRecovery::new(policy).expect("validated recovery policy"),
            policy,
            started: None,
            pending: None,
        }
    }
    pub fn poll(
        &mut self,
        session: &dyn ActiveSession,
    ) -> Result<Option<(MonitorRecoveryReason, bool)>, super::BackendError> {
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        if let Some(pending) = self.pending.as_mut() {
            if session.monitor_fade_out_submitted() {
                let submitted = *pending.submitted.get_or_insert(now);
                if now.duration_since(submitted) >= session.monitor_queue_drain_duration() {
                    return Ok(Some((pending.reason, false)));
                }
            }
            return Ok(
                (now.duration_since(pending.requested) >= self.policy.fade_timeout)
                    .then_some((pending.reason, true)),
            );
        }
        if let Some(faults) = session.monitor_faults()
            && let Some(reason) = self.detector.observe(now.duration_since(started), faults)
        {
            session.request_monitor_fade_out(self.policy.fade_frames)?;
            self.pending = Some(PendingFade {
                reason,
                requested: now,
                submitted: None,
            });
        }
        Ok(None)
    }
}

pub(super) fn remaining_limit(
    limit: super::RunLimit,
    elapsed: Duration,
) -> Option<super::RunLimit> {
    match limit {
        super::RunLimit::UntilStopped => Some(limit),
        super::RunLimit::Seconds(seconds) => {
            let remaining = Duration::from_secs(u64::from(seconds)).saturating_sub(elapsed);
            if remaining.is_zero() {
                None
            } else {
                Some(super::RunLimit::Seconds(
                    remaining
                        .as_secs()
                        .saturating_add(u64::from(remaining.subsec_nanos() != 0))
                        as u16,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::RunLimit;

    #[test]
    fn finite_fallback_keeps_only_remaining_seconds_and_never_restarts_an_expired_run() {
        assert_eq!(
            remaining_limit(RunLimit::Seconds(10), Duration::from_millis(6100)),
            Some(RunLimit::Seconds(4))
        );
        assert_eq!(
            remaining_limit(RunLimit::Seconds(10), Duration::from_secs(10)),
            None
        );
        assert_eq!(
            remaining_limit(RunLimit::Seconds(10), Duration::from_secs(11)),
            None
        );
        assert_eq!(
            remaining_limit(RunLimit::UntilStopped, Duration::MAX),
            Some(RunLimit::UntilStopped)
        );
    }
}
