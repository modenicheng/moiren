//! Control-side progress evidence, separate from confirmed owner lifecycle.

use anyhow::{Context, Result, ensure};
use moiren_engine::host::{ExecutionState, HostSnapshot, HostState, OwnerTimeline};
use std::time::{Duration, Instant};

use super::ValidationPolicy;

const PROGRESS_DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn running(engine: Option<&HostSnapshot>) -> bool {
    engine.is_some_and(|engine| engine.execution == ExecutionState::Running)
}

pub(super) struct Tracker {
    admitted: HostSnapshot,
    last: OwnerTimeline,
    started: Instant,
    last_progress: Duration,
    last_observation: Duration,
    unknown_observations: u64,
    overdue: bool,
    failure: Option<String>,
    policy: ValidationPolicy,
}

impl Tracker {
    pub(super) fn new(admitted: &HostSnapshot, policy: ValidationPolicy) -> Result<Self> {
        policy.validate()?;
        let last = admitted
            .timeline
            .context("initial Engine progress unavailable")?;
        ensure!(
            admitted.host == HostState::Open
                && running(Some(admitted))
                && admitted.fault.is_none()
                && admitted.cleanup_error.is_none()
                && admitted.active_revision.is_some()
                && admitted.plan_revision == Some(last.revision)
                && last.frame > 0,
            "initial Engine identity or progress not confirmed"
        );
        Ok(Self {
            admitted: admitted.clone(),
            last,
            started: Instant::now(),
            last_progress: Duration::ZERO,
            last_observation: Duration::ZERO,
            unknown_observations: 0,
            overdue: false,
            failure: None,
            policy,
        })
    }

    pub(super) fn observe(&mut self, engine: Option<&HostSnapshot>) -> Result<()> {
        // Timestamp after the potentially slow provider/native reads. A late
        // advanced frame cannot retroactively erase an expired control deadline.
        self.observe_at(engine, self.started.elapsed())
    }

    fn observe_at(&mut self, engine: Option<&HostSnapshot>, now: Duration) -> Result<()> {
        let result = self.check_at(engine, now);
        if let Err(error) = &result {
            self.failure.get_or_insert_with(|| format!("{error:#}"));
        }
        result
    }

    fn check_at(&mut self, engine: Option<&HostSnapshot>, now: Duration) -> Result<()> {
        let engine = engine.context("Engine observation unavailable")?;
        ensure!(
            engine.host == HostState::Open
                && running(Some(engine))
                && engine.fault.is_none()
                && engine.cleanup_error.is_none(),
            "Engine lifecycle stopped or faulted"
        );
        ensure!(
            engine.desired_revision == self.admitted.desired_revision
                && engine.active_revision == self.admitted.active_revision
                && engine.plan_revision == self.admitted.plan_revision
                && engine.owner_generation == self.admitted.owner_generation
                && engine.resource_generation == self.admitted.resource_generation,
            "Engine revision/owner/resource identity changed"
        );
        ensure!(
            now >= self.last_observation,
            "Engine control clock regressed"
        );
        self.last_observation = now;
        let age = now.saturating_sub(self.last_progress);
        self.overdue |= age >= PROGRESS_DEADLINE;
        ensure!(
            age < PROGRESS_DEADLINE.saturating_add(self.policy.progress_grace()),
            "Engine progress deadline exceeded"
        );
        if let Some(timeline) = engine.timeline {
            ensure!(
                timeline.revision == self.last.revision && timeline.epoch == self.last.epoch,
                "Engine timeline plan/epoch changed"
            );
            ensure!(
                timeline.frame >= self.last.frame,
                "Engine timeline frame regressed"
            );
            if timeline.frame > self.last.frame {
                self.last = timeline;
                self.last_progress = now;
            }
        } else {
            self.unknown_observations = self.unknown_observations.saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn overdue(&self) -> bool {
        self.overdue
    }

    pub(super) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({
            "scope": "control-side progress guard; last advancing timeline is historical, not a fresh raw snapshot or physical latency",
            "base_deadline_ms": PROGRESS_DEADLINE.as_millis(),
            "progress_grace_ms": self.policy.progress_grace().as_millis(),
            "base_deadline_exceeded": self.overdue,
            "unknown_observations": self.unknown_observations,
            "last_observation_ms": self.last_observation.as_millis(),
            "last_advancement_ms": self.last_progress.as_millis(),
            "historical_last_advancing_timeline": {
                "frame": self.last.frame, "epoch": self.last.epoch, "revision": self.last.revision,
            },
            "first_failure": self.failure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::host::{BackendError, Diagnostic, UpdateState};

    fn snapshot() -> HostSnapshot {
        HostSnapshot {
            host: HostState::Open,
            execution: ExecutionState::Running,
            desired_revision: 2,
            active_revision: Some(2),
            plan_revision: Some(7),
            owner_generation: 1,
            resource_generation: 0,
            timeline: Some(OwnerTimeline {
                epoch: 3,
                revision: 7,
                frame: 128,
            }),
            update: UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        }
    }

    fn correctness() -> ValidationPolicy {
        ValidationPolicy {
            mode: super::super::super::validation_policy::ValidationMode::Correctness,
            jitter_tolerance_ms: 3000,
        }
    }

    #[test]
    fn transient_unknown_is_not_stop_or_progress_and_raw_null_survives() {
        let admitted = snapshot();
        let mut guard = Tracker::new(&admitted, ValidationPolicy::default()).unwrap();
        let mut current = admitted.clone();
        current.timeline = None;
        assert!(running(Some(&current)));
        guard
            .observe_at(Some(&current), Duration::from_secs(1))
            .unwrap();
        assert_eq!(guard.last_progress, Duration::ZERO);
        assert!(current.timeline.is_none());
        current.timeline = Some(OwnerTimeline {
            frame: 256,
            ..admitted.timeline.unwrap()
        });
        guard
            .observe_at(Some(&current), Duration::from_secs(2))
            .unwrap();
        assert_eq!(guard.last_progress, Duration::from_secs(2));
        assert!(!guard.overdue());
    }

    #[test]
    fn absent_unchanged_and_late_advanced_frames_cannot_renew_expired_deadline() {
        for unknown in [false, true] {
            for late_advance in [false, true] {
                let admitted = snapshot();
                let mut guard = Tracker::new(&admitted, ValidationPolicy::default()).unwrap();
                let mut current = admitted.clone();
                if unknown {
                    current.timeline = None;
                }
                guard
                    .observe_at(Some(&current), Duration::from_millis(4999))
                    .unwrap();
                if late_advance {
                    current.timeline = Some(OwnerTimeline {
                        frame: 256,
                        ..admitted.timeline.unwrap()
                    });
                }
                let error = guard
                    .observe_at(Some(&current), Duration::from_secs(5))
                    .unwrap_err();
                assert!(error.to_string().contains("progress deadline"));
                assert_eq!(guard.last_progress, Duration::ZERO);
                assert!(guard.overdue());
            }
        }
    }

    #[test]
    fn correctness_grace_keeps_sticky_quality_failure_and_has_exact_hard_boundary() {
        let admitted = snapshot();
        let mut guard = Tracker::new(&admitted, correctness()).unwrap();
        let mut current = admitted.clone();
        current.timeline = None;
        guard
            .observe_at(Some(&current), Duration::from_secs(5))
            .unwrap();
        assert!(guard.overdue());
        current.timeline = Some(OwnerTimeline {
            frame: 256,
            ..admitted.timeline.unwrap()
        });
        guard
            .observe_at(Some(&current), Duration::from_secs(6))
            .unwrap();
        assert!(guard.overdue());
        assert_eq!(guard.last_progress, Duration::from_secs(6));
        let mut guard = Tracker::new(&admitted, correctness()).unwrap();
        assert!(
            guard
                .observe_at(Some(&current), Duration::from_secs(8))
                .is_err()
        );
        assert_eq!(guard.last_progress, Duration::ZERO);
    }

    #[test]
    fn identity_lifecycle_and_faults_are_hard_even_with_unknown_timeline() {
        let admitted = snapshot();
        for case in 0..11 {
            let mut guard = Tracker::new(&admitted, correctness()).unwrap();
            let mut current = admitted.clone();
            current.timeline = None;
            match case {
                0 => current.host = HostState::Closed,
                1 => current.execution = ExecutionState::Dormant,
                2 => current.desired_revision += 1,
                3 => current.active_revision = Some(3),
                4 => current.plan_revision = Some(8),
                5 => current.owner_generation += 1,
                6 => current.resource_generation += 1,
                7 => current.active_revision = None,
                8 => current.plan_revision = None,
                9 => current.fault = Some(Diagnostic::Fault("fault".into())),
                10 => current.cleanup_error = Some(BackendError::Failed("cleanup".into())),
                _ => unreachable!(),
            }
            assert!(
                guard
                    .observe_at(Some(&current), Duration::from_millis(1))
                    .is_err(),
                "case {case}"
            );
        }
        assert!(
            Tracker::new(&admitted, correctness())
                .unwrap()
                .observe_at(None, Duration::ZERO)
                .is_err()
        );
    }

    #[test]
    fn epoch_plan_and_frame_regressions_are_not_progress() {
        let admitted = snapshot();
        for case in 0..3 {
            let mut guard = Tracker::new(&admitted, correctness()).unwrap();
            let mut current = admitted.clone();
            let timeline = current.timeline.as_mut().unwrap();
            match case {
                0 => timeline.epoch += 1,
                1 => timeline.revision = 2,
                2 => timeline.frame -= 1,
                _ => unreachable!(),
            }
            assert!(
                guard
                    .observe_at(Some(&current), Duration::from_millis(1))
                    .is_err()
            );
            assert_eq!(guard.last_progress, Duration::ZERO);
        }
    }

    #[test]
    fn initial_proof_requires_progress_and_matching_plan_not_graph_revision() {
        let admitted = snapshot();
        assert!(Tracker::new(&admitted, ValidationPolicy::default()).is_ok());
        for case in 0..3 {
            let mut invalid = admitted.clone();
            match case {
                0 => invalid.timeline = None,
                1 => invalid.timeline.as_mut().unwrap().frame = 0,
                2 => invalid.timeline.as_mut().unwrap().revision = 2,
                _ => unreachable!(),
            }
            assert!(Tracker::new(&invalid, ValidationPolicy::default()).is_err());
        }
    }

    #[test]
    fn engine_health_first_failure_survives_later_observations_and_final_serialization() {
        let admitted = snapshot();
        let mut guard = Tracker::new(&admitted, correctness()).unwrap();
        let mut invalid = admitted.clone();
        invalid.timeline = None;
        invalid.owner_generation += 1;
        assert!(
            guard
                .observe_at(Some(&invalid), Duration::from_millis(1))
                .is_err()
        );
        let first = guard.evidence()["first_failure"].clone();
        guard
            .observe_at(Some(&admitted), Duration::from_millis(2))
            .unwrap();
        assert_eq!(guard.evidence()["first_failure"], first);
        assert!(first.as_str().unwrap().contains("identity changed"));
    }
}
