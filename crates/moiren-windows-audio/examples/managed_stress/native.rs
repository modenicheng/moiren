use super::{options::Options, project};
use anyhow::{Context, ensure};
use moiren_core::protocol::ParamValue;
use moiren_engine::{
    host::{
        BackendError, CancellationToken, EditOutcome, EditReceipt, EngineBackend, EngineHost,
        ExecutionState, HostConfig, HostError, HostSnapshot, HostState, NodeConfig,
        ParameterOutcome, ParameterReceipt, ParameterTarget, ParameterTiming, PrepareRequest,
        PreparedBackend, ResourceEvent,
    },
    processor::Gain,
};
use moiren_windows_audio::engine_backend::WindowsBackend;
use serde::Serialize;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Backend(Arc<WindowsBackend>);
impl EngineBackend<f32> for Backend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f32>, BackendError> {
        self.0.prepare(request, cancel)
    }
    fn poll_resource(&self) -> Option<ResourceEvent> {
        self.0.poll_resource()
    }
}

#[derive(Default, Serialize)]
struct Counts {
    edit_submissions: u64,
    edits_activated: u64,
    edits_superseded: u64,
    parameter_submissions: u64,
    parameters_applied: u64,
    parameters_superseded: u64,
    queue_backpressure: u64,
    max_pending_receipts: usize,
    max_desired_active_gap: u64,
    max_timeline_stall_ms: u64,
    cycles_completed: u32,
    max_close_ms: u64,
}

fn config() -> HostConfig {
    HostConfig {
        receipt_capacity: 512,
        max_nodes: 128,
        ..Default::default()
    }
}

fn wait_running(host: &EngineHost<f32>) -> anyhow::Result<HostSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = host.observer().snapshot();
        ensure!(
            snapshot.fault.is_none() && snapshot.cleanup_error.is_none(),
            "host failed: {snapshot:?}"
        );
        if snapshot.execution == ExecutionState::Running && snapshot.timeline.is_some() {
            return Ok(snapshot);
        }
        ensure!(Instant::now() < deadline, "startup timed out: {snapshot:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn collect(
    edits: &mut Vec<EditReceipt>,
    parameters: &mut Vec<ParameterReceipt>,
    counts: &mut Counts,
) -> anyhow::Result<()> {
    let mut failure = None;
    edits.retain(|receipt| match receipt.outcome() {
        EditOutcome::Activated { .. } => {
            counts.edits_activated += 1;
            false
        }
        EditOutcome::Superseded { .. } => {
            counts.edits_superseded += 1;
            false
        }
        EditOutcome::Queued | EditOutcome::Accepted { .. } => true,
        outcome => {
            failure = Some(format!("edit: {outcome:?}"));
            false
        }
    });
    parameters.retain(|receipt| match receipt.outcome() {
        ParameterOutcome::Applied(_) => {
            counts.parameters_applied += 1;
            false
        }
        ParameterOutcome::Superseded => {
            counts.parameters_superseded += 1;
            false
        }
        ParameterOutcome::Queued | ParameterOutcome::Accepted => true,
        outcome => {
            failure = Some(format!("parameter: {outcome:?}"));
            false
        }
    });
    ensure!(failure.is_none(), "unexpected terminal result: {failure:?}");
    Ok(())
}

fn close(host: EngineHost<f32>, counts: &mut Counts) -> anyhow::Result<()> {
    let observer = host.observer();
    let started = Instant::now();
    drop(host);
    counts.max_close_ms = counts
        .max_close_ms
        .max(started.elapsed().as_millis() as u64);
    let snapshot = observer.snapshot();
    ensure!(
        snapshot.host == HostState::Closed,
        "shutdown not closed: {snapshot:?}"
    );
    ensure!(
        snapshot.fault.is_none() && snapshot.cleanup_error.is_none(),
        "cleanup failed: {snapshot:?}"
    );
    Ok(())
}

pub fn run(options: Options) -> anyhow::Result<()> {
    let built = project::build(options.nodes)?;
    let backend = Arc::new(WindowsBackend::new(Default::default())?);
    let host = EngineHost::<f32>::open(config(), built.project.clone(), Backend(backend.clone()))?;
    let initial = wait_running(&host)?;
    let control = host.control();
    let observer = host.observer();
    let started = Instant::now();
    let mut progress_at = started;
    let mut last_frame = initial.timeline.unwrap().frame;
    let mut edit_attempts = 0u64;
    let mut parameter_attempts = 0u64;
    let mut edits = Vec::with_capacity(128);
    let mut parameters = Vec::with_capacity(256);
    let mut counts = Counts::default();
    println!(
        "{}",
        serde_json::json!({"event":"ready","pid":std::process::id(),"processors":options.nodes,"owner":initial.owner_generation})
    );
    while started.elapsed() < Duration::from_secs(u64::from(options.seconds)) {
        collect(&mut edits, &mut parameters, &mut counts)?;
        let elapsed = started.elapsed().as_secs_f64();
        let due_edits = (elapsed * f64::from(options.edits_per_second)) as u64;
        let due_parameters = (elapsed * f64::from(options.parameters_per_second)) as u64;
        for _ in 0..due_edits.saturating_sub(edit_attempts).min(64) {
            edit_attempts += 1;
            if edits.len() >= 128 {
                counts.queue_backpressure += 1;
                continue;
            }
            let node = built.edited;
            let value = if edit_attempts.is_multiple_of(2) {
                0.95
            } else {
                1.0
            };
            match control.edit(move |project| {
                project
                    .nodes
                    .insert(node, NodeConfig::Gain { initial: value });
                Ok(())
            }) {
                Ok(receipt) => {
                    counts.edit_submissions += 1;
                    edits.push(receipt);
                }
                Err(HostError::Capacity) => counts.queue_backpressure += 1,
                Err(error) => return Err(error.into()),
            }
        }
        for _ in 0..due_parameters.saturating_sub(parameter_attempts).min(128) {
            parameter_attempts += 1;
            if parameters.len() >= 256 {
                counts.queue_backpressure += 1;
                continue;
            }
            let value = if parameter_attempts.is_multiple_of(2) {
                0.8
            } else {
                0.9
            };
            match control.parameter(
                ParameterTarget::Node {
                    node: built.parameter,
                    parameter: Gain::LEVEL,
                },
                ParamValue::Float(value),
                ParameterTiming::NextBlock,
                32,
            ) {
                Ok(receipt) => {
                    counts.parameter_submissions += 1;
                    parameters.push(receipt);
                }
                Err(HostError::Capacity) => counts.queue_backpressure += 1,
                Err(error) => return Err(error.into()),
            }
        }
        counts.max_pending_receipts = counts
            .max_pending_receipts
            .max(edits.len() + parameters.len());
        let snapshot = observer.snapshot();
        ensure!(
            snapshot.execution == ExecutionState::Running
                && snapshot.fault.is_none()
                && snapshot.cleanup_error.is_none(),
            "lost running owner: {snapshot:?}"
        );
        ensure!(
            snapshot.owner_generation == initial.owner_generation,
            "compatible edits restarted native owner"
        );
        counts.max_desired_active_gap = counts.max_desired_active_gap.max(
            snapshot
                .desired_revision
                .saturating_sub(snapshot.active_revision.unwrap_or(0)),
        );
        if let Some(timeline) = snapshot.timeline
            && timeline.frame > last_frame
        {
            counts.max_timeline_stall_ms = counts
                .max_timeline_stall_ms
                .max(progress_at.elapsed().as_millis() as u64);
            progress_at = Instant::now();
            last_frame = timeline.frame;
        }
        ensure!(
            progress_at.elapsed() < Duration::from_secs(2),
            "timeline stalled: {snapshot:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let drain_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        collect(&mut edits, &mut parameters, &mut counts)?;
        let snapshot = observer.snapshot();
        if edits.is_empty()
            && parameters.is_empty()
            && snapshot.active_revision == Some(snapshot.desired_revision)
        {
            break;
        }
        ensure!(
            Instant::now() < drain_deadline,
            "accepted work did not settle: {snapshot:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let final_snapshot = observer.snapshot();
    ensure!(
        counts.edit_submissions == counts.edits_activated + counts.edits_superseded,
        "edit receipt mismatch"
    );
    ensure!(
        counts.parameter_submissions == counts.parameters_applied + counts.parameters_superseded,
        "parameter receipt mismatch"
    );
    close(host, &mut counts)?;
    let native_report: serde_json::Value = serde_json::from_str(
        &backend
            .last_native_report()
            .context("native report absent")?,
    )?;
    ensure!(
        native_report["render"]["stats"]["bridge_shortfall_frames"] == 0,
        "native bridge shortfall: {native_report}"
    );
    ensure!(
        native_report["render"]["stats"]["nonzero_samples"] == 0,
        "muted output guard failed"
    );
    for _ in 0..options.cycles {
        let host =
            EngineHost::<f32>::open(config(), built.project.clone(), Backend(backend.clone()))?;
        wait_running(&host)?;
        close(host, &mut counts)?;
        counts.cycles_completed += 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "event":"complete","processors":options.nodes,"seconds":options.seconds,
            "counts":counts,"active_revision":final_snapshot.active_revision,
            "desired_revision":final_snapshot.desired_revision,"native":native_report,
        })
    );
    Ok(())
}
