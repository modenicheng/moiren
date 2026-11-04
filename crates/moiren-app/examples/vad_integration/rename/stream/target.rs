//! Bounded coordinator on one COM-owning worker. Only App owns driver control.
use super::{Case, PcmPhase, owner_running, release_owner, replacement, running};
use crate::{
    native::{Pair, rename_stream},
    rename::{RuntimeEvidence, shared_deadline, windows_name},
    report::{Evidence, Report},
    validation,
};
use anyhow::{Context, ensure};
use moiren_app::service::{AppHandle, AppRequest, AppSnapshot, GraphEdit};
use moiren_windows_audio::vad::{Intent, StableEndpointId};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const PHASE_LIMIT: Duration = Duration::from_secs(10);
const PCM_LIMIT: Duration = Duration::from_secs(12);
const SETTLE: Duration = Duration::from_secs(1);

pub(crate) fn exercise(
    handle: &AppHandle,
    ids: &[String; 2],
    stable_ids: [StableEndpointId; 2],
    deadline: Instant,
    report: &mut Report,
    mut publish: impl FnMut(&Report),
) -> anyhow::Result<()> {
    report.rename_streaming = Some(Vec::new());
    for (index, intent) in [Intent::Playback, Intent::Microphone]
        .into_iter()
        .enumerate()
    {
        let mut case = Case::new(
            format!("{intent:?}"),
            format!("Moiren streaming rename {intent:?} {}", std::process::id()),
        );
        report
            .rename_streaming
            .as_mut()
            .expect("initialized")
            .push(case.clone());
        let mut emit = |case: &Case| {
            report.current_phase = format!("rename_streaming_{}", case.direction);
            report.rename_streaming.as_mut().expect("initialized")[index] = case.clone();
            report.native_clients_started_by_probe = report
                .rename_streaming
                .as_ref()
                .expect("initialized")
                .iter()
                .map(|case| case.before.started + case.after.started)
                .sum();
            publish(report);
        };
        exercise_case(
            handle,
            ids,
            stable_ids,
            index,
            intent,
            shared_deadline(Instant::now(), deadline, (2 - index) as u32),
            &mut case,
            &mut emit,
        );
        publish(report);
    }
    ensure!(
        report
            .rename_streaming
            .as_ref()
            .context("missing streaming report")?
            .iter()
            .all(|case| case.completed),
        "streaming rename acceptance failed; see rename_streaming failures (diagnostic completion is not a pass)"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn exercise_case(
    handle: &AppHandle,
    ids: &[String; 2],
    stable: [StableEndpointId; 2],
    index: usize,
    intent: Intent,
    deadline: Instant,
    case: &mut Case,
    emit: &mut impl FnMut(&Case),
) {
    let mut old_owner = None;
    let mut new_owner = None;
    let outcome = (|| -> anyhow::Result<()> {
        let applied = request(handle, AppRequest::ApplyGraph)?;
        let snapshot = wait(handle, deadline, |s| {
            Evidence::from(s).running(ids, applied)
        })?;
        case.before.admission = Some(observe(&snapshot, &ids[index])?);
        seed_gates(&snapshot, &mut case.before)?;
        // Read-only MMDevice/store owners return and drop before any exclusive
        // Pair initialization. The name observation never activates IAudioClient.
        case.before.name = Some(rename_stream::observe_name(
            stable[index],
            intent,
            phase_deadline(deadline),
        )?);
        emit(case);
        old_owner = Some(Pair::prepare(stable, phase_deadline(deadline))?);
        case.before.initialized = true;
        case.before.pair_mmdevice_id = Some(
            old_owner
                .as_ref()
                .context("old owner missing")?
                .target_mmdevice_id(intent),
        );
        emit(case);
        let start = old_owner
            .as_mut()
            .context("old owner missing")?
            .start(&mut case.before.started);
        if let Err(error) = &start {
            case.native_interruptions
                .push(rename_stream::failure("old_pair_start", error));
        }
        emit(case); // Partial capture Start survives render Start failure.
        start?;
        let progress_template = case.clone();
        signal_phase(
            handle,
            ids,
            &ids[index],
            old_owner.as_mut().context("old owner missing")?,
            false,
            deadline,
            &mut case.before,
            |phase| {
                let mut progress = progress_template.clone();
                progress.before = phase.clone();
                emit(&progress);
            },
        )?;
        let baseline = case
            .before
            .confirmed
            .clone()
            .context("missing old PCM confirmation")?;
        let snapshot = handle.try_snapshot().context("missing pre-edit snapshot")?;
        ensure!(
            Evidence::from(snapshot.as_ref()).running(ids, applied),
            "route changed before rename"
        );
        let current = observe(&snapshot, &ids[index])?;
        same_admission(&baseline, &current)?;
        let mut exports = snapshot.graph.exports.clone();
        exports
            .iter_mut()
            .find(|e| e.endpoint_id.to_string() == ids[index])
            .context("target export missing")?
            .name = case.requested_name.clone();
        ensure!(
            old_owner
                .as_ref()
                .context("old owner missing")?
                .both_running(),
            "old Pair not RUN at rename"
        );
        let edit = request(
            handle,
            AppRequest::EditGraph(GraphEdit::SetExports(exports)),
        )?;
        case.edit_request = Some(edit);
        case.rename_while_started = true;
        emit(case);
        // Explicit Apply starts the intended rename rebuild. This is not a race
        // scenario and does not claim that a queued request proves removal.
        request(handle, AppRequest::ApplyGraph)?;
        let interruption = pump_until_transition(
            handle,
            &ids[index],
            old_owner.as_mut().context("old owner missing")?,
            edit,
            phase_deadline(deadline),
            case,
            emit,
        );
        if let Err(error) = interruption {
            case.fail(format!("rename interruption: {error:#}"));
        }
        release_pair(&mut old_owner, false, case);
        emit(case);

        // Keep the historical direct Removed timing and identity predicate.
        // Failure is retained, but cannot suppress bounded fresh/name/PCM data.
        if let Err(error) = wait_removed(
            handle,
            &ids[index],
            crate::rename::retirement_deadline(Instant::now(), deadline),
            case,
            emit,
        ) {
            case.fail(format!("direct old Removed: {error:#}"));
        }
        emit(case);
        wait(handle, deadline, |s| {
            observe(s, &ids[index]).is_ok_and(|e| {
                e.consumed_request_id >= edit
                    && e.resource == "present"
                    && !e.rename_pending
                    && e.provider_name.as_deref() == Some(case.requested_name.as_str())
            })
        })?;
        // No initialized exclusive Pair exists during this readback/Apply.
        match rename_stream::observe_name(stable[index], intent, phase_deadline(deadline)) {
            Ok(name) => {
                case.after.name = Some(name);
                emit(case);
                if let Err(error) = windows_name(
                    case.before.name.as_ref().context("missing old name")?,
                    case.after.name.as_ref().context("missing fresh name")?,
                    &case.requested_name,
                ) {
                    case.fail(format!("exact Windows name: {error:#}"));
                }
            }
            Err(error) => case.fail(format!("replacement Windows name unknown: {error:#}")),
        }
        emit(case);
        let reapplied = request(handle, AppRequest::ApplyGraph)?;
        let snapshot = wait(handle, deadline, |s| {
            Evidence::from(s).running(ids, reapplied)
        })?;
        let admission = observe(&snapshot, &ids[index])?;
        case.after.admission = Some(admission.clone());
        seed_gates(&snapshot, &mut case.after)?;
        emit(case);
        replacement(&baseline, &admission, &case.requested_name, edit)?;
        // Pair is initialized only after Apply/Engine confirmation. No old
        // exclusive stream/MMDevice reference survives release_pair above.
        new_owner = Some(Pair::prepare(stable, phase_deadline(deadline))?);
        case.after.initialized = true;
        case.after.pair_mmdevice_id = Some(
            new_owner
                .as_ref()
                .context("new owner missing")?
                .target_mmdevice_id(intent),
        );
        new_owner
            .as_mut()
            .context("new owner missing")?
            .set_signal(validation::SECOND_SIGNAL);
        emit(case);
        let start = new_owner
            .as_mut()
            .context("new owner missing")?
            .start(&mut case.after.started);
        if let Err(error) = &start {
            case.native_interruptions
                .push(rename_stream::failure("fresh_pair_start", error));
        }
        emit(case);
        start?;
        let progress_template = case.clone();
        signal_phase(
            handle,
            ids,
            &ids[index],
            new_owner.as_mut().context("new owner missing")?,
            true,
            deadline,
            &mut case.after,
            |phase| {
                let mut progress = progress_template.clone();
                progress.after = phase.clone();
                emit(&progress);
            },
        )?;
        case.diagnostic_completed = true;
        Ok(())
    })();
    if let Err(error) = outcome {
        case.fail(format!("streaming sequence: {error:#}"));
    }
    emit(case);
    release_pair(&mut old_owner, false, case);
    release_pair(&mut new_owner, true, case);
    case.finish();
    emit(case);
}

fn release_pair(owner: &mut Option<Pair>, after: bool, case: &mut Case) {
    let phase = if after {
        &mut case.after
    } else {
        &mut case.before
    };
    let mut native = Vec::new();
    release_owner(owner, phase, |pair| {
        native = pair.stop_for_rename();
        native
            .iter()
            .map(|e| format!("{}: {}", e.stage, e.message))
            .collect()
    });
    // Invalidated Stops are retained as cleanup errors too; a successful Drop
    // must not rewrite an original Stop failure into clean release acceptance.
    case.native_interruptions.extend(native);
}

fn pump_until_transition(
    handle: &AppHandle,
    id: &str,
    pair: &mut Pair,
    edit: u64,
    deadline: Instant,
    case: &mut Case,
    emit: &mut impl FnMut(&Case),
) -> anyhow::Result<()> {
    let mut invalidated = false;
    loop {
        ensure!(Instant::now() < deadline, "rename transition deadline");
        if !invalidated {
            // These frames belong to the old transport, never the fresh phase.
            if let Err(error) = pair.pump(|samples, discontinuity| {
                case.interruption.observe_packet_lenient(
                    samples,
                    validation::EXPECTED,
                    discontinuity,
                )
            }) {
                let failure = rename_stream::failure("old_pair_pump_after_edit", &error);
                invalidated = failure.invalidated;
                case.native_interruptions.push(failure);
                emit(case);
                if !invalidated {
                    return Err(error);
                }
            }
        }
        if let Some(snapshot) = handle.try_snapshot() {
            let runtime = observe(&snapshot, id)?;
            let transitioned = case.observe_transition(runtime, edit);
            emit(case);
            // Freeze the observed transition rather than overwriting a failure
            // or unknown old transport with replacement DTO success.
            if transitioned {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[allow(clippy::too_many_arguments)]
fn signal_phase(
    handle: &AppHandle,
    ids: &[String; 2],
    id: &str,
    pair: &mut Pair,
    second: bool,
    deadline: Instant,
    phase: &mut PcmPhase,
    mut emit: impl FnMut(&PcmPhase),
) -> anyhow::Result<()> {
    let started = Instant::now();
    let deadline = deadline.min(started + PCM_LIMIT);
    let admission = phase
        .admission
        .clone()
        .context("missing signal admission")?;
    let mut last_snapshot: Option<(Arc<AppSnapshot>, Instant)> = None;
    loop {
        ensure!(
            Instant::now() < deadline,
            "PCM phase deadline; matching signal incomplete"
        );
        for gate in &phase.gate_transitions {
            gate.require_deadline(started.elapsed().as_millis())?;
        }
        if let Some((_, observed)) = &last_snapshot {
            ensure!(
                observed.elapsed() <= Duration::from_secs(2),
                "PCM snapshot publication stale"
            );
        }
        if let Some(snapshot) = handle.try_snapshot() {
            if last_snapshot
                .as_ref()
                .is_none_or(|(old, _)| !Arc::ptr_eq(old, &snapshot))
            {
                last_snapshot = Some((snapshot.clone(), Instant::now()));
            }
            let observed = observe(&snapshot, id)?;
            // Save the actual final/failed observation, not a copied admission.
            phase.confirmed = Some(observed.clone());
            emit(phase);
            ensure!(
                Evidence::from(snapshot.as_ref()).owner_running(ids, admission.consumed_request_id),
                "PCM route lacks current Engine confirmation"
            );
            // Native Start may advance the stream generation and temporarily
            // close admission. Reuse the existing bounded 5s transition gate;
            // it never grants permission for a new PDO or changed revision.
            same_owner(&admission, &observed)?;
            for export in &Evidence::from(snapshot.as_ref()).exports {
                let gate = phase
                    .gate_transitions
                    .iter_mut()
                    .find(|g| g.endpoint_id == export.endpoint_id)
                    .context("missing phase-local gate baseline")?;
                gate.observe(
                    export.native_stamp,
                    export.gate_open,
                    started.elapsed().as_millis(),
                    deadline.saturating_duration_since(started).as_millis(),
                )?;
            }
        }
        let sampled = pair.pump(|samples, discontinuity| {
            phase.observe_pcm(
                samples,
                discontinuity,
                second,
                started.elapsed() < SETTLE,
                started.elapsed().as_millis(),
            )
        });
        emit(phase);
        sampled?;
        if started.elapsed() >= SETTLE
            && phase.pcm.require_signal().is_ok()
            && let Some(snapshot) = handle.try_snapshot()
        {
            let observed = observe(&snapshot, id)?;
            phase.confirmed = Some(observed.clone());
            emit(phase);
            same_admission(&admission, &observed)?;
            ensure!(
                Evidence::from(snapshot.as_ref()).running(ids, admission.consumed_request_id),
                "PCM end confirmation missing"
            );
            ensure!(
                Instant::now() < deadline,
                "PCM confirmation exceeded deadline"
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn seed_gates(snapshot: &AppSnapshot, phase: &mut PcmPhase) -> anyhow::Result<()> {
    phase.gate_transitions = Evidence::from(snapshot)
        .exports
        .iter()
        .map(|e| validation::GateProgress::new(e.endpoint_id.clone(), e.native_stamp, 0))
        .collect::<anyhow::Result<_>>()?;
    Ok(())
}

fn same_admission(old: &RuntimeEvidence, current: &RuntimeEvidence) -> anyhow::Result<()> {
    running(current)?;
    same_owner(old, current)
}

fn same_owner(old: &RuntimeEvidence, current: &RuntimeEvidence) -> anyhow::Result<()> {
    owner_running(current)?;
    let previous = old.native_stamp.context("missing PCM admission stamp")?;
    let stamp = current.native_stamp.context("missing current PCM stamp")?;
    ensure!(
        old.intent == current.intent
            && old.requested_name == current.requested_name
            && old.active_revision == current.active_revision
            && stamp[..3] == previous[..3]
            && stamp[3] >= previous[3]
            && stamp[4] >= previous[4]
            && current.observation_generation >= old.observation_generation,
        "PCM phase crossed replacement identity/revision/binding"
    );
    Ok(())
}

fn request(handle: &AppHandle, request: AppRequest) -> anyhow::Result<u64> {
    handle
        .try_request_tracked(request)
        .map(|id| id.0)
        .map_err(|error| anyhow::anyhow!("streaming request enqueue: {error:?}"))
}

fn observe(snapshot: &AppSnapshot, id: &str) -> anyhow::Result<RuntimeEvidence> {
    crate::rename::target::observe(snapshot, id)
}

fn phase_deadline(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + PHASE_LIMIT)
}

fn wait_removed(
    handle: &AppHandle,
    id: &str,
    deadline: Instant,
    case: &mut Case,
    emit: &mut impl FnMut(&Case),
) -> anyhow::Result<()> {
    loop {
        ensure!(Instant::now() < deadline, "direct old Removed deadline");
        if let Some(snapshot) = handle.try_snapshot() {
            let result = case.observe_retirement(observe(&snapshot, id)?);
            // Keep first unknown and last actual result independently of the
            // acceptance flag. Polling never turns a timeout into a pass.
            emit(case);
            if result.is_ok() {
                ensure!(
                    Instant::now() < deadline,
                    "direct Removed snapshot exceeded deadline"
                );
                return result;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait(
    handle: &AppHandle,
    deadline: Instant,
    ready: impl Fn(&AppSnapshot) -> bool,
) -> anyhow::Result<Arc<AppSnapshot>> {
    let deadline = phase_deadline(deadline);
    loop {
        ensure!(
            Instant::now() < deadline,
            "streaming service phase deadline"
        );
        if let Some(snapshot) = handle.try_snapshot()
            && ready(&snapshot)
        {
            ensure!(
                Instant::now() < deadline,
                "streaming snapshot exceeded deadline"
            );
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
