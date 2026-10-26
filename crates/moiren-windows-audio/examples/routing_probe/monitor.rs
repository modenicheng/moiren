//! Control-thread health checks and bounded native observation history.
use anyhow::{Context, bail};
use moiren_engine::host::{ExecutionState, HostSnapshot, HostState, UpdateState};
use moiren_windows_audio::engine_backend::NativeObservationSnapshot;
use serde::Serialize;
use std::time::Duration;

pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(20);
pub(super) const MAX_SAMPLES: usize = 1801;
const TIMELINE_STALL_LIMIT: Duration = Duration::from_secs(2);

#[derive(Serialize)]
pub(super) struct Sample {
    pub elapsed_seconds: f64,
    pub observation: Option<NativeObservationSnapshot>,
}

pub(super) struct Samples {
    pub values: Vec<Sample>,
    pub limit: usize,
    next_sample: Duration,
}

impl Samples {
    pub fn new(seconds: u64) -> anyhow::Result<Self> {
        if !(1..=1800).contains(&seconds) {
            bail!("sample history requires 1..1800 seconds");
        }
        let limit = usize::try_from(seconds)? + 1;
        Ok(Self {
            values: Vec::with_capacity(limit),
            limit,
            next_sample: Duration::ZERO,
        })
    }

    pub fn due(&self, elapsed: Duration) -> bool {
        // Reserve one terminal sample even when failure falls between ticks.
        self.values.len() < self.limit - 1 && elapsed >= self.next_sample
    }

    pub fn push(
        &mut self,
        elapsed: Duration,
        observation: Option<NativeObservationSnapshot>,
    ) -> anyhow::Result<()> {
        if self.values.len() >= self.limit || self.values.len() >= MAX_SAMPLES {
            bail!("native observation sample count exceeded its bound");
        }
        self.values.push(Sample {
            elapsed_seconds: elapsed.as_secs_f64(),
            observation,
        });
        // A delayed poll records one current observation, never a catch-up burst.
        self.next_sample = Duration::from_secs(elapsed.as_secs() + 1);
        Ok(())
    }
}

fn check_common(snapshot: &HostSnapshot) -> anyhow::Result<()> {
    if let Some(fault) = &snapshot.fault {
        bail!("host fault: {fault:?}");
    }
    if let Some(error) = &snapshot.cleanup_error {
        bail!("host cleanup error: {error}");
    }
    if let UpdateState::Blocked { reason, .. } = &snapshot.update {
        bail!("host update blocked: {reason:?}");
    }
    if matches!(snapshot.host, HostState::Closing | HostState::Closed) {
        bail!("host terminated unexpectedly: {:?}", snapshot.host);
    }
    Ok(())
}

pub(super) fn startup_ready(snapshot: &HostSnapshot) -> anyhow::Result<bool> {
    check_common(snapshot)?;
    Ok(snapshot.host == HostState::Open
        && snapshot.execution == ExecutionState::Running
        && snapshot.timeline.is_some())
}

pub(super) struct Health {
    owner_generation: u64,
    epoch: u64,
    last_frame: u64,
    last_advance: Duration,
}

impl Health {
    pub fn new(initial: &HostSnapshot) -> anyhow::Result<Self> {
        let timeline = initial.timeline.context("initial owner has no timeline")?;
        Ok(Self {
            owner_generation: initial.owner_generation,
            epoch: timeline.epoch,
            last_frame: timeline.frame,
            last_advance: Duration::ZERO,
        })
    }

    pub fn check(&mut self, snapshot: &HostSnapshot, elapsed: Duration) -> anyhow::Result<()> {
        check_common(snapshot)?;
        if snapshot.host != HostState::Open || snapshot.execution != ExecutionState::Running {
            bail!("host stopped running: {:?}", snapshot.execution);
        }
        if snapshot.owner_generation != self.owner_generation {
            bail!(
                "owner generation changed: {} -> {}",
                self.owner_generation,
                snapshot.owner_generation
            );
        }
        if let Some(timeline) = snapshot.timeline {
            if timeline.epoch != self.epoch {
                bail!(
                    "timeline epoch changed: {} -> {}",
                    self.epoch,
                    timeline.epoch
                );
            }
            if timeline.frame < self.last_frame {
                bail!("owner timeline moved backwards");
            }
            if timeline.frame > self.last_frame {
                self.last_frame = timeline.frame;
                self.last_advance = elapsed;
            }
        }
        // Compatible swaps can briefly hide the revision-filtered timeline.
        // Bound that gap as well as a stalled frame without treating it as a restart.
        if elapsed.saturating_sub(self.last_advance) >= TIMELINE_STALL_LIMIT {
            bail!("owner timeline did not advance for {TIMELINE_STALL_LIMIT:?}");
        }
        Ok(())
    }
}

pub(super) fn check_native(
    observation: Option<&NativeObservationSnapshot>,
    owner_generation: u64,
) -> anyhow::Result<()> {
    let observation = observation.context("native running observations disappeared")?;
    if observation.owner_generation != owner_generation {
        bail!(
            "native owner generation changed: {} -> {}",
            owner_generation,
            observation.owner_generation
        );
    }
    if !observation.running {
        bail!("native owner is no longer running");
    }
    Ok(())
}

pub(super) fn check_closed(snapshot: &HostSnapshot) -> anyhow::Result<()> {
    if snapshot.host != HostState::Closed
        || snapshot.fault.is_some()
        || snapshot.cleanup_error.is_some()
    {
        bail!("host closure verification failed: {snapshot:?}");
    }
    Ok(())
}

pub(super) fn finish(
    primary: anyhow::Result<()>,
    closure: anyhow::Result<()>,
) -> anyhow::Result<()> {
    // Both results are evaluated before this call; keep the first run failure.
    primary?;
    closure
}

pub(super) fn snapshot_json(snapshot: &HostSnapshot) -> serde_json::Value {
    serde_json::json!({
        "host": format!("{:?}", snapshot.host),
        "execution": format!("{:?}", snapshot.execution),
        "desired_revision": snapshot.desired_revision,
        "active_revision": snapshot.active_revision,
        "plan_revision": snapshot.plan_revision,
        "owner_generation": snapshot.owner_generation,
        "resource_generation": snapshot.resource_generation,
        "timeline": snapshot.timeline.map(|timeline| serde_json::json!({
            "epoch": timeline.epoch,
            "revision": timeline.revision,
            "frame": timeline.frame,
        })),
        "update": format!("{:?}", snapshot.update),
        "fault": snapshot.fault.as_ref().map(|fault| format!("{fault:?}")),
        "cleanup_error": snapshot.cleanup_error.as_ref().map(ToString::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::host::{BackendError, Diagnostic, OwnerTimeline};

    fn running() -> HostSnapshot {
        HostSnapshot {
            host: HostState::Open,
            execution: ExecutionState::Running,
            desired_revision: 1,
            active_revision: Some(1),
            plan_revision: Some(1),
            owner_generation: 7,
            resource_generation: 1,
            timeline: Some(OwnerTimeline {
                epoch: 11,
                revision: 1,
                frame: 128,
            }),
            update: UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        }
    }

    #[test]
    fn longest_soak_is_bounded_even_with_frequent_polling_and_a_terminal_sample() {
        let mut samples = Samples::new(1800).unwrap();
        for tick in 0..90_000 {
            let elapsed = Duration::from_millis(tick * 20);
            if samples.due(elapsed) {
                samples.push(elapsed, None).unwrap();
            }
        }
        samples.push(Duration::from_secs(1800), None).unwrap();
        assert_eq!(samples.limit, MAX_SAMPLES);
        assert_eq!(samples.values.len(), MAX_SAMPLES);
        for (second, sample) in samples.values.iter().enumerate() {
            assert_eq!(sample.elapsed_seconds, second as f64);
        }
        assert!(!samples.due(Duration::from_secs(1801)));
        assert!(samples.push(Duration::from_secs(1801), None).is_err());
        assert!(Samples::new(1801).is_err());
        assert!(Samples::new(0).is_err());
    }

    #[test]
    fn short_run_reserves_terminal_sample_and_does_not_catch_up_after_delays() {
        let mut samples = Samples::new(1).unwrap();
        assert!(samples.due(Duration::ZERO));
        samples.push(Duration::ZERO, None).unwrap();
        assert!(!samples.due(Duration::from_secs(1)));
        samples.push(Duration::from_secs(1), None).unwrap();
        assert_eq!(samples.values.len(), 2);

        let mut samples = Samples::new(60).unwrap();
        samples.push(Duration::ZERO, None).unwrap();
        let late = Duration::from_millis(3150);
        assert!(samples.due(late));
        samples.push(late, None).unwrap();
        assert!(!samples.due(late));
        assert!(samples.due(Duration::from_secs(4)));
    }

    #[test]
    fn health_rejects_transient_fault_owner_epoch_and_execution_failures() {
        let initial = running();
        let mut bad = Vec::new();
        let mut snapshot = initial.clone();
        snapshot.fault = Some(Diagnostic::Fault("transient native failure".into()));
        bad.push(snapshot);
        let mut snapshot = initial.clone();
        snapshot.owner_generation += 1;
        bad.push(snapshot);
        let mut snapshot = initial.clone();
        snapshot.timeline.as_mut().unwrap().epoch += 1;
        bad.push(snapshot);
        for execution in [
            ExecutionState::Dormant,
            ExecutionState::Preparing,
            ExecutionState::WaitingForResources,
        ] {
            let mut snapshot = initial.clone();
            snapshot.execution = execution;
            bad.push(snapshot);
        }
        let mut snapshot = initial.clone();
        snapshot.host = HostState::Closed;
        bad.push(snapshot);
        let mut snapshot = initial.clone();
        snapshot.update = UpdateState::Blocked {
            graph_revision: 2,
            reason: Diagnostic::Fault("rejected update".into()),
        };
        bad.push(snapshot);
        let mut snapshot = initial.clone();
        snapshot.cleanup_error = Some(BackendError::WorkerPanicked);
        bad.push(snapshot);
        let mut snapshot = initial.clone();
        snapshot.timeline.as_mut().unwrap().frame -= 1;
        bad.push(snapshot);
        for snapshot in bad {
            // The initial/final states alone would miss each intervening failure.
            let mut health = Health::new(&initial).unwrap();
            health.check(&initial, Duration::ZERO).unwrap();
            assert!(
                health.check(&snapshot, POLL_INTERVAL).is_err(),
                "{snapshot:?}"
            );
        }
    }

    #[test]
    fn health_allows_compatible_timeline_gap_but_bounds_stalls() {
        let initial = running();
        let mut health = Health::new(&initial).unwrap();
        let mut snapshot = initial.clone();
        snapshot.timeline = None;
        snapshot.update = UpdateState::Pending(2);
        health.check(&snapshot, POLL_INTERVAL).unwrap();
        snapshot.timeline = initial.timeline;
        snapshot.timeline.as_mut().unwrap().revision = 2;
        snapshot.timeline.as_mut().unwrap().frame += 128;
        health.check(&snapshot, Duration::from_secs(1)).unwrap();
        assert!(health.check(&snapshot, Duration::from_secs(3)).is_err());
        let mut health = Health::new(&initial).unwrap();
        snapshot.timeline = None;
        assert!(health.check(&snapshot, TIMELINE_STALL_LIMIT).is_err());
    }

    #[test]
    fn native_health_rejects_missing_stopped_and_replaced_owners() {
        let mut observation = NativeObservationSnapshot {
            render_padding: Vec::new(),
            owner_generation: 7,
            running: true,
            streams: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
        };
        check_native(Some(&observation), 7).unwrap();
        observation.running = false;
        assert!(check_native(Some(&observation), 7).is_err());
        observation.running = true;
        assert!(check_native(Some(&observation), 8).is_err());
        assert!(check_native(None, 7).is_err());
    }

    #[test]
    fn serialized_samples_and_host_diagnostics_preserve_the_observed_state() {
        let mut samples = Samples::new(5).unwrap();
        samples
            .push(
                Duration::from_millis(1250),
                Some(NativeObservationSnapshot {
                    render_padding: Vec::new(),
                    owner_generation: 7,
                    running: true,
                    streams: Vec::new(),
                    bridges: Vec::new(),
                    boundary_probes: Vec::new(),
                }),
            )
            .unwrap();
        let value = serde_json::to_value(&samples.values).unwrap();
        assert_eq!(value[0]["elapsed_seconds"], 1.25);
        assert_eq!(value[0]["observation"]["owner_generation"], 7);
        assert_eq!(value[0]["observation"]["running"], true);
        assert!(value[0]["observation"]["bridges"].is_array());

        let mut snapshot = running();
        snapshot.fault = Some(Diagnostic::Fault("transient failure".into()));
        let value = snapshot_json(&snapshot);
        assert_eq!(value["owner_generation"], 7);
        assert_eq!(value["timeline"]["epoch"], 11);
        assert!(
            value["fault"]
                .as_str()
                .unwrap()
                .contains("transient failure")
        );
    }

    #[test]
    fn startup_and_closure_failures_keep_the_original_run_error() {
        let mut snapshot = running();
        assert!(startup_ready(&snapshot).unwrap());
        snapshot.execution = ExecutionState::Preparing;
        assert!(!startup_ready(&snapshot).unwrap());
        snapshot.fault = Some(Diagnostic::Fault("startup fault".into()));
        assert!(startup_ready(&snapshot).is_err());
        let error = finish(
            Err(anyhow::anyhow!("original run failure")),
            check_closed(&snapshot),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "original run failure");
        let error = finish(
            finish(
                Err(anyhow::anyhow!("original run failure")),
                check_closed(&snapshot),
            ),
            Err(anyhow::anyhow!("stdout failed")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "original run failure");
        assert!(finish(Ok(()), check_closed(&snapshot)).is_err());
        snapshot.host = HostState::Closed;
        snapshot.fault = None;
        finish(Ok(()), check_closed(&snapshot)).unwrap();
        snapshot.cleanup_error = Some(BackendError::WorkerPanicked);
        assert!(check_closed(&snapshot).is_err());
    }
}
