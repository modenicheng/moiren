//! Real production App owner plus apartment-local WASAPI clients; no control IO.
use super::{
    Case, IntentEvidence, RuntimeEvidence, held, replacement, same_held_device, windows_name,
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
            deadline,
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
    let outcome = (|| -> anyhow::Result<()> {
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
        emit(case);

        let mut exports = snapshot.graph.exports.clone();
        let export = exports
            .iter_mut()
            .find(|e| e.endpoint_id.to_string() == ids[slot(intent)])
            .context("rename target missing from graph")?;
        export.name = case.requested_name.clone();
        let edited = request(
            handle,
            AppRequest::EditGraph(GraphEdit::SetExports(exports)),
        )?;
        // An accepted edit is not a completed rename. Require the service's
        // explicit pending/removing resource state before checking the held pin.
        wait(
            handle,
            &ids[slot(intent)],
            deadline,
            case,
            emit,
            Sample::Held,
            |_, e| e.consumed_request_id >= edited && e.resource == "removing",
        )?;
        let hold_until = Instant::now() + HELD_OBSERVATION;
        loop {
            ensure!(
                Instant::now() < deadline,
                "held rename observation exceeded work deadline"
            );
            let snapshot = handle.try_snapshot().context("missing held snapshot")?;
            let current = observe(&snapshot, &ids[slot(intent)])?;
            case.held = Some(current.clone());
            // Persist failures as well as success before attempting native cleanup.
            case.while_held = Some(client.as_ref().context("held client missing")?.observe()?);
            emit(case);
            held(&baseline, &current, &case.requested_name, edited)?;
            same_held_device(
                case.before.as_ref().context("missing before property")?,
                case.while_held.as_ref().context("missing held property")?,
            )?;
            if Instant::now() >= hold_until {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        case.held_completion_refused = true;
        emit(case);
        let released = client.take().context("held client missing")?.release();
        case.held_client_released = true;
        if let Err(error) = released {
            case.cleanup_errors
                .push(format!("held native release: {error:#}"));
            anyhow::bail!("held native release failed");
        }
        emit(case);

        // Production alone observes removal and creates a replacement. Open a
        // fresh native client after that observation, forcing fresh enumeration,
        // exact-root/stable-ID/flow matching and actual IAudioClient initialization.
        let requested_name = case.requested_name.clone();
        wait(
            handle,
            &ids[slot(intent)],
            deadline,
            case,
            emit,
            Sample::Replacement,
            |_, e| {
                e.resource == "present"
                    && !e.rename_pending
                    && e.provider_name.as_deref() == Some(requested_name.as_str())
            },
        )?;
        fresh_client = Some(HeldClient::prepare(
            stable,
            intent,
            phase_deadline(deadline),
        )?);
        case.replacement_client_initialized = true;
        case.after = Some(
            fresh_client
                .as_ref()
                .context("replacement client missing")?
                .observe()?,
        );
        emit(case);
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
        case.after = Some(
            fresh_client
                .as_ref()
                .context("replacement client missing")?
                .observe()?,
        );
        emit(case);
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
    })();
    if let Err(error) = outcome {
        case.error = Some(format!("{error:#}"));
    }
    // Publish the primary failure before cleanup can cross any native wait.
    emit(case);
    // Every result releases all initialized clients before the next direction
    // or service exit. Failed Stop and primary evidence errors remain separate.
    if let Some(client) = client {
        let result = client.release();
        case.held_client_released = true;
        if let Err(error) = result {
            case.cleanup_errors
                .push(format!("held native release: {error:#}"));
        }
        emit(case);
    }
    if let Some(client) = fresh_client {
        let result = client.release();
        case.replacement_client_released = true;
        if let Err(error) = result {
            case.cleanup_errors
                .push(format!("replacement native release: {error:#}"));
        }
        emit(case);
    }
    case.finish();
    emit(case);
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
