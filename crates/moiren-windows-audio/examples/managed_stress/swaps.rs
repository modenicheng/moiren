use anyhow::{Context, bail, ensure};
use moiren_engine::host::{EditOutcome, HostSnapshot};
use serde::Serialize;
use std::time::Duration;

pub struct ConfirmedSwaps {
    owner: u64,
    epoch: u64,
    plan: u64,
    frame: u64,
    completed: u64,
    latency_us: u64,
    max_latency_us: u64,
    histogram_ms: [u64; 10_001],
}

#[derive(Serialize)]
pub struct SwapReport {
    pub completed: u64,
    pub measured_seconds: f64,
    pub per_second: f64,
    pub latency_mean_ms: f64,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_p99_ms: u64,
    pub latency_max_ms: f64,
}

impl ConfirmedSwaps {
    pub fn new(initial: &HostSnapshot) -> anyhow::Result<Self> {
        let timeline = initial.timeline.context("initial RT timeline absent")?;
        ensure!(
            initial.plan_revision == Some(timeline.revision),
            "initial plan mismatch"
        );
        Ok(Self {
            owner: initial.owner_generation,
            epoch: timeline.epoch,
            plan: timeline.revision,
            frame: timeline.frame,
            completed: 0,
            latency_us: 0,
            max_latency_us: 0,
            histogram_ms: [0; 10_001],
        })
    }

    pub fn observe(
        &mut self,
        outcome: &EditOutcome,
        snapshot: &HostSnapshot,
        latency: Duration,
    ) -> anyhow::Result<bool> {
        ensure!(
            snapshot.owner_generation == self.owner,
            "native owner restarted"
        );
        let (graph, plan) = match outcome {
            EditOutcome::Queued | EditOutcome::Accepted { .. } => return Ok(false),
            EditOutcome::Activated {
                graph_revision,
                plan_revision: Some(plan),
            } => (*graph_revision, *plan),
            outcome => bail!("confirmed edit failed: {outcome:?}"),
        };
        ensure!(
            plan > self.plan,
            "no-op or duplicate activation is not a swap"
        );
        let Some(timeline) = snapshot.timeline else {
            return Ok(false);
        };
        ensure!(timeline.epoch == self.epoch, "RT timeline epoch changed");
        if snapshot.active_revision != Some(graph)
            || snapshot.plan_revision != Some(plan)
            || timeline.revision != plan
        {
            return Ok(false);
        }
        ensure!(timeline.frame > self.frame, "RT timeline did not advance");
        self.plan = plan;
        self.frame = timeline.frame;
        self.completed += 1;
        let micros = latency.as_micros() as u64;
        self.latency_us += micros;
        self.max_latency_us = self.max_latency_us.max(micros);
        let bucket = micros.div_ceil(1000).min(10_000) as usize;
        self.histogram_ms[bucket] += 1;
        Ok(true)
    }

    pub fn report(&self, seconds: f64) -> SwapReport {
        let percentile = |percent: u64| {
            if self.completed == 0 {
                return 0;
            }
            let target = (self.completed * percent).div_ceil(100);
            let mut total = 0;
            for (millis, count) in self.histogram_ms.iter().enumerate() {
                total += count;
                if total >= target {
                    return millis as u64;
                }
            }
            10_000
        };
        SwapReport {
            completed: self.completed,
            measured_seconds: seconds,
            per_second: self.completed as f64 / seconds.max(f64::EPSILON),
            latency_mean_ms: self.latency_us as f64 / self.completed.max(1) as f64 / 1000.0,
            latency_p50_ms: percentile(50),
            latency_p95_ms: percentile(95),
            latency_p99_ms: percentile(99),
            latency_max_ms: self.max_latency_us as f64 / 1000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::host::{ExecutionState, HostState, OwnerTimeline, UpdateState};

    fn snapshot(plan: u64, frame: u64) -> HostSnapshot {
        HostSnapshot {
            host: HostState::Open,
            execution: ExecutionState::Running,
            desired_revision: plan,
            active_revision: Some(plan),
            plan_revision: Some(plan),
            owner_generation: 1,
            resource_generation: 0,
            timeline: Some(OwnerTimeline {
                epoch: 1,
                revision: plan,
                frame,
            }),
            update: UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        }
    }

    fn activated(plan: u64) -> EditOutcome {
        EditOutcome::Activated {
            graph_revision: plan,
            plan_revision: Some(plan),
        }
    }

    #[test]
    fn counts_only_a_new_plan_confirmed_on_the_same_advancing_rt_timeline() {
        let mut swaps = ConfirmedSwaps::new(&snapshot(1, 256)).unwrap();
        let latency = Duration::from_millis(12);
        assert!(
            !swaps
                .observe(
                    &EditOutcome::Accepted { graph_revision: 2 },
                    &snapshot(1, 512),
                    latency
                )
                .unwrap()
        );
        assert!(
            !swaps
                .observe(&activated(2), &snapshot(1, 512), latency)
                .unwrap()
        );
        let mut missing_timeline = snapshot(2, 768);
        missing_timeline.timeline = None;
        assert!(
            !swaps
                .observe(&activated(2), &missing_timeline, latency)
                .unwrap()
        );
        assert_eq!(swaps.report(1.0).completed, 0);
        assert!(
            swaps
                .observe(&activated(2), &snapshot(2, 768), latency)
                .unwrap()
        );
        let report = swaps.report(1.0);
        assert_eq!(report.completed, 1);
        assert_eq!(report.per_second, 1.0);
        assert_eq!(report.latency_p99_ms, 12);
        assert!(
            swaps
                .observe(&activated(2), &snapshot(2, 768), latency)
                .is_err()
        );
    }

    #[test]
    fn rejects_noops_supersession_owner_restarts_and_timeline_resets() {
        for (outcome, observed) in [
            (activated(1), snapshot(1, 512)),
            (
                EditOutcome::Superseded {
                    by_graph_revision: 3,
                },
                snapshot(3, 768),
            ),
            (activated(2), snapshot(2, 256)),
            (activated(2), {
                let mut observed = snapshot(2, 768);
                observed.owner_generation = 2;
                observed
            }),
            (activated(2), {
                let mut observed = snapshot(2, 768);
                observed.timeline.as_mut().unwrap().epoch = 2;
                observed
            }),
        ] {
            let mut swaps = ConfirmedSwaps::new(&snapshot(1, 256)).unwrap();
            assert!(
                swaps
                    .observe(&outcome, &observed, Duration::from_millis(10))
                    .is_err()
            );
            assert_eq!(swaps.report(1.0).completed, 0);
        }
    }

    #[test]
    fn latency_percentiles_are_millisecond_upper_bounds_and_empty_reports_are_finite() {
        let mut swaps = ConfirmedSwaps::new(&snapshot(1, 256)).unwrap();
        let empty = swaps.report(0.0);
        assert_eq!(empty.latency_p99_ms, 0);
        assert_eq!(empty.per_second, 0.0);
        for plan in 2..=101 {
            let latency = Duration::from_micros((plan - 1) * 1000 + 1);
            assert!(
                swaps
                    .observe(&activated(plan), &snapshot(plan, plan * 256), latency)
                    .unwrap()
            );
        }
        let report = swaps.report(2.0);
        assert_eq!(report.completed, 100);
        assert_eq!(report.per_second, 50.0);
        assert_eq!(report.latency_p50_ms, 51);
        assert_eq!(report.latency_p95_ms, 96);
        assert_eq!(report.latency_p99_ms, 100);
        assert_eq!(report.latency_max_ms, 100.001);
    }
}
