//! Real production App owner plus apartment-local WASAPI clients; no control IO.
use super::{
    Case, IntentEvidence, Phase, RuntimeEvidence, continue_after_held, direct_removed, held,
    replacement, retirement_deadline, same_held_device, shared_deadline, step, windows_name,
};
use crate::{
    native::HeldClient,
    report::{Evidence, Report},
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
const HELD_OBSERVATION: Duration = Duration::from_millis(500);

pub(crate) fn exercise(
    handle: &AppHandle,
    ids: &[String; 2],
    stable_ids: [StableEndpointId; 2],
    deadline: Instant,
    report: &mut Report,
    mut publish: impl FnMut(&Report),
) -> anyhow::Result<()> {
    report.rename = Some(Vec::new());
    let mut failures = Vec::new();
    for (index, intent) in [Intent::Playback, Intent::Microphone]
        .into_iter()
        .enumerate()
    {
        let mut case = Case::new(
            format!("{intent:?}"),
            format!("Moiren rename {intent:?} {}", std::process::id()),
        );
        report
            .rename
            .as_mut()
            .expect("initialized")
            .push(case.clone());
        let mut emit = |case: &Case| {
            report.current_phase = format!("rename_{}", case.direction);
            report.rename.as_mut().expect("initialized")[index] = case.clone();
            publish(report);
        };
        exercise_case(
            handle,
            ids,
            stable_ids[index],
            intent,
            // Playback cannot spend Microphone's share of the work deadline.
            // Native calls can still stall: the unchanged process watchdog is
            // the ultimate bound, never proof of completed native cleanup.
            shared_deadline(Instant::now(), deadline, (2 - index) as u32),
            &mut case,
            &mut emit,
        );
        if let Some(error) = &case.error {
            failures.push(format!("{intent:?}: {error}"));
        }
        emit(&case);
    }
    ensure!(
        failures.is_empty(),
        "rename acceptance failed: {}",
        failures.join("; ")
    );
    Ok(())
}

fn exercise_case(
    handle: &AppHandle,
    ids: &[String; 2],
    stable: StableEndpointId,
    intent: Intent,
    deadline: Instant,
    case: &mut Case,
    emit: &mut impl FnMut(&Case),
) {
    let mut client = None;
    let mut fresh_client = None;
    let baseline = step(case, Phase::Baseline, emit, |case, emit| {
        let deadline = shared_deadline(Instant::now(), deadline, 2);
        // Each direction starts from a separately confirmed real running graph.
        let applied = request(handle, AppRequest::ApplyGraph)?;
        wait(
            handle,
            &ids[slot(intent)],
            deadline,
            case,
            emit,
            Sample::Baseline,
            |s, _| Evidence::from(s).running(ids, applied),
        )?;
        client = Some(HeldClient::prepare(
            stable,
            intent,
            phase_deadline(deadline),
        )?);
        case.held_client_initialized = true;
        emit(case);
        let stopped = request(handle, AppRequest::Stop)?;
        let (snapshot, baseline) = wait(
            handle,
            &ids[slot(intent)],
            deadline,
            case,
            emit,
            Sample::Baseline,
            |s, runtime| Evidence::from(s).stopped(ids, stopped) && runtime.native_stamp.is_some(),
        )?;
        case.baseline = Some(baseline.clone());
        case.before = Some(client.as_ref().context("held client missing")?.observe()?);
        Ok((snapshot, baseline))
    });
    if let Some((snapshot, baseline)) = baseline {
        let mut edited = None;
        step(case, Phase::StrictHeld, emit, |case, emit| {
            let deadline = shared_deadline(Instant::now(), deadline, 2);
            let mut exports = snapshot.graph.exports.clone();
            let export = exports
                .iter_mut()
                .find(|e| e.endpoint_id.to_string() == ids[slot(intent)])
                .context("rename target missing from graph")?;
            export.name = case.requested_name.clone();
            let request_id = request(
                handle,
                AppRequest::EditGraph(GraphEdit::SetExports(exports)),
            )?;
            edited = Some(request_id);
            let requested_name = case.requested_name.clone();
            // Do not wait specifically for Removing: actual removal can finish
            // before a snapshot. Consumed intent permits the strict observation,
            // but neither Absent nor DTO name success satisfies its old gate.
            wait(
                handle,
                &ids[slot(intent)],
                deadline,
                case,
                emit,
                Sample::Held,
                |_, e| {
                    e.consumed_request_id >= request_id
                        && e.requested_name == requested_name
                        && (e.resource != "present" || e.provider_name != baseline.provider_name)
                },
            )?;
            let hold_until = Instant::now() + HELD_OBSERVATION;
            ensure!(
                hold_until < deadline,
                "insufficient work budget for strict held observation"
            );
            loop {
                ensure!(
                    Instant::now() < deadline,
                    "held rename observation exceeded work deadline"
                );
                let snapshot = handle.try_snapshot().context("missing held snapshot")?;
                let current = observe(&snapshot, &ids[slot(intent)])?;
                case.held = Some(current.clone());
                case.while_held = Some(client.as_ref().context("held client missing")?.observe()?);
                emit(case);
                // Historical strict acceptance expectation only. Holding a COM
                // client is not a driver promise to keep its PDO Removing.
                held(&baseline, &current, &case.requested_name, request_id)?;
                same_held_device(
                    case.before.as_ref().context("missing before property")?,
                    case.while_held.as_ref().context("missing held property")?,
                )?;
                ensure!(
                    Instant::now() < deadline,
                    "held native readback exceeded work deadline"
                );
                if Instant::now() >= hold_until {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            case.held_completion_refused = true;
            Ok(())
        });
        continue_after_held(
            case,
            emit,
            |case, _| release_held(&mut client, case),
            |case, emit| {
                let edited =
                    edited.context("rename request was not enqueued; no replacement diagnostic")?;
                // Persist direct old Removed independently and before recording
                // replacement evidence. Failure must not suppress native/name
                // diagnostics, but can never be replaced by Absent/new token.
                step(case, Phase::DirectRemoved, emit, |case, emit| {
                    let result = wait(
                        handle,
                        &ids[slot(intent)],
                        retirement_deadline(Instant::now(), deadline),
                        case,
                        emit,
                        Sample::Retirement,
                        |_, e| direct_removed(&baseline, e.native_retirement.as_ref()).is_ok(),
                    );
                    if let Err(error) = result {
                        direct_removed(&baseline, case.old_removed.as_ref())
                            .with_context(|| format!("direct old-Removed wait: {error:#}"))?;
                        return Err(error);
                    }
                    direct_removed(&baseline, case.old_removed.as_ref())?;
                    case.direct_removed_verified = true;
                    Ok(())
                });
                let requested_name = case.requested_name.clone();
                step(case, Phase::ReplacementPresent, emit, |case, emit| {
                    wait(
                        handle,
                        &ids[slot(intent)],
                        shared_deadline(Instant::now(), deadline, 3),
                        case,
                        emit,
                        Sample::Replacement,
                        |_, e| {
                            e.consumed_request_id >= edited
                                && e.resource == "present"
                                && !e.rename_pending
                                && e.provider_name.as_deref() == Some(requested_name.as_str())
                        },
                    )
                })
                .context("replacement not present; native initialization/readback unavailable")?;
                // This is a new COM owner/enumeration, not a reopened held
                // MMDevice. It still does not substitute for native generations.
                step(case, Phase::ReplacementNativeInit, emit, |case, _| {
                    fresh_client = Some(HeldClient::prepare(
                        stable,
                        intent,
                        phase_deadline(shared_deadline(Instant::now(), deadline, 2)),
                    )?);
                    case.replacement_client_initialized = true;
                    Ok(())
                })
                .context("replacement native initialization unavailable")?;
                // Read and persist the exact Windows property BEFORE Engine
                // confirmation, so an unrelated Engine failure cannot hide it.
                step(case, Phase::ExactWindowsName, emit, |case, emit| {
                    ensure!(
                        Instant::now() < deadline,
                        "Windows name readback work deadline exhausted"
                    );
                    case.after = Some(
                        fresh_client
                            .as_ref()
                            .context("replacement client missing")?
                            .observe()?,
                    );
                    emit(case);
                    ensure!(
                        Instant::now() < deadline,
                        "Windows name readback exceeded work deadline"
                    );
                    windows_name(
                        case.before
                            .as_ref()
                            .context("missing old native property")?,
                        case.after
                            .as_ref()
                            .context("missing replacement native property")?,
                        &case.requested_name,
                    )?;
                    case.windows_name_verified = true;
                    Ok(())
                });
                step(case, Phase::EngineConfirmation, emit, |case, emit| {
                    let reapplied = request(handle, AppRequest::ApplyGraph)?;
                    let (_, current) = wait(
                        handle,
                        &ids[slot(intent)],
                        deadline,
                        case,
                        emit,
                        Sample::Replacement,
                        |s, _| Evidence::from(s).running(ids, reapplied),
                    )?;
                    replacement(&baseline, &current, &case.requested_name, reapplied)?;
                    case.fresh_replacement_confirmed = true;
                    Ok(())
                });
                Ok(())
            },
        );
    }
    // Publish the primary failure before cleanup can cross any native wait.
    emit(case);
    // Every result releases all initialized clients before the next direction
    // or service exit. Failed Stop and primary evidence errors remain separate.
    if client.is_some() {
        step(case, Phase::HeldRelease, emit, |case, _| {
            release_held(&mut client, case)
        });
    }
    if let Some(client) = fresh_client {
        step(case, Phase::ReplacementRelease, emit, |case, _| {
            let result = client.release();
            case.replacement_client_released = true;
            if let Err(error) = &result {
                case.cleanup_errors
                    .push(format!("replacement native release: {error:#}"));
            }
            result
        });
    }
    case.finish();
    emit(case);
}

fn release_held(client: &mut Option<HeldClient>, case: &mut Case) -> anyhow::Result<()> {
    let result = client.take().context("held client missing")?.release();
    case.held_client_released = true;
    if let Err(error) = &result {
        case.cleanup_errors
            .push(format!("held native release: {error:#}"));
    }
    result
}

fn slot(intent: Intent) -> usize {
    if intent == Intent::Playback { 0 } else { 1 }
}

fn request(handle: &AppHandle, request: AppRequest) -> anyhow::Result<u64> {
    handle
        .try_request_tracked(request)
        .map(|id| id.0)
        .map_err(|error| anyhow::anyhow!("rename request enqueue: {error:?}"))
}

fn phase_deadline(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + PHASE_LIMIT)
}

enum Sample {
    Baseline,
    Held,
    Retirement,
    Replacement,
}

#[allow(clippy::too_many_arguments)]
fn wait(
    handle: &AppHandle,
    id: &str,
    deadline: Instant,
    case: &mut Case,
    emit: &mut impl FnMut(&Case),
    sample: Sample,
    ready: impl Fn(&AppSnapshot, &RuntimeEvidence) -> bool,
) -> anyhow::Result<(Arc<AppSnapshot>, RuntimeEvidence)> {
    let deadline = phase_deadline(deadline);
    loop {
        ensure!(Instant::now() < deadline, "rename service phase timed out");
        if let Some(snapshot) = handle.try_snapshot() {
            let runtime = observe(&snapshot, id)?;
            match sample {
                Sample::Baseline => case.baseline = Some(runtime.clone()),
                Sample::Held => case.held = Some(runtime.clone()),
                Sample::Retirement => case.old_removed = runtime.native_retirement,
                Sample::Replacement => case.replacement = Some(runtime.clone()),
            }
            emit(case);
            ensure!(
                Instant::now() < deadline,
                "rename service snapshot arrived after deadline"
            );
            if ready(&snapshot, &runtime) {
                return Ok((snapshot, runtime));
            }
            ensure!(
                runtime.phase != "failed",
                "rename service failed: {:?}",
                runtime.service_error
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn observe(snapshot: &AppSnapshot, id: &str) -> anyhow::Result<RuntimeEvidence> {
    let mut matching = snapshot
        .exports
        .iter()
        .filter(|e| e.intent.endpoint_id.to_string() == id);
    let export = matching.next().context("rename export status missing")?;
    ensure!(matching.next().is_none(), "duplicate rename export status");
    let diagnostics = export.diagnostics.as_ref();
    Ok(RuntimeEvidence {
        intent: IntentEvidence {
            endpoint_id: id.into(),
            direction: format!("{:?}", export.intent.direction),
            channels: export.intent.channels,
            binding: export.intent.binding.map(|id| format!("{id:?}")),
            enabled: export.intent.enabled,
        },
        requested_name: export.intent.name.clone(),
        provider_name: export.actual_name.clone(),
        rename_pending: export.rename_pending,
        resource: format!("{:?}", export.resource).to_ascii_lowercase(),
        observation_generation: diagnostics.map(|d| d.generation),
        native_stamp: diagnostics.and_then(|d| d.native_stamp),
        native_retirement: export.native_retirement.map(Into::into),
        gate_open: export.gate_open,
        allow_default: export.allow_default,
        export_active_revision: export.active_revision,
        candidate_revision: snapshot.graph.candidate_revision,
        active_revision: snapshot.graph.active_revision,
        pending_revision: snapshot.graph.pending_revision,
        consumed_request_id: snapshot.consumed_request_id.0,
        phase: format!("{:?}", snapshot.phase).to_ascii_lowercase(),
        service_error: snapshot.error.clone(),
        graph_error: snapshot.graph.last_error.clone(),
        provider_error: diagnostics.and_then(|d| d.error.clone()),
    })
}
