//! Pure text projections from [`AppSnapshot`]. These functions format
//! snapshot state for display; they hold no UI state and dispatch no intent.

use crate::service::{AppSnapshot, SessionPhase};
#[cfg(windows)]
use crate::service::{AudioBackend, SessionGeneration};
use rust_i18n::t;
use std::borrow::Cow;

/// Display-level stale guard mirroring the service's write-side filter. The
/// owner identity is not published to the UI, so this deliberately checks only
/// phase, generation, and the observation's own running flag.
#[cfg(windows)]
fn fresh_observation(
    phase: SessionPhase,
    generation: SessionGeneration,
    status_generation: SessionGeneration,
    running: bool,
) -> bool {
    phase == SessionPhase::Running && generation == status_generation && running
}

pub fn phase_text(snapshot: &AppSnapshot) -> Cow<'static, str> {
    match snapshot.phase {
        SessionPhase::Idle => t!("Idle"),
        SessionPhase::Starting => t!("Starting"),
        SessionPhase::Running => t!("Running"),
        SessionPhase::Stopping => t!("Stopping"),
        SessionPhase::Failed => t!("Failed"),
        SessionPhase::Exiting => t!("Exiting"),
        SessionPhase::Exited => t!("Exited"),
    }
}

pub fn latency_apply_text(snapshot: &AppSnapshot) -> Cow<'static, str> {
    match snapshot.active_latency_profile {
        Some(active) if active == snapshot.latency_profile => {
            t!("Active profile matches selection")
        }
        _ => t!("Selected profile activates on the next audio start"),
    }
}

/// Only the currently running owner can supply timing. In particular, a
/// stopped or replaced owner must not leave plausible-looking stale values.
pub fn native_timing_rows(snapshot: &AppSnapshot) -> Vec<(String, String)> {
    #[cfg(not(windows))]
    let _ = snapshot;
    #[cfg(windows)]
    {
        if let Some(status) = snapshot
            .professional_observations
            .as_ref()
            .filter(|status| {
                fresh_observation(
                    snapshot.phase,
                    snapshot.generation,
                    status.generation,
                    status.observation.running,
                )
            })
        {
            let mut rows = Vec::new();
            if let Some(settings) = &snapshot.active_audio_settings {
                rows.push((
                    t!("Processing").to_string(),
                    format!(
                        "{} Hz; {}; {} frames",
                        settings.processing.sample_rate,
                        t!(settings.processing.precision.title()),
                        settings.processing.block_frames,
                    ),
                ));
            }
            for (index, stream) in status.observation.streams.iter().enumerate() {
                let kind = if stream.input {
                    // "Input" is not in the i18n key catalog; kept as source text.
                    "Input".to_string()
                } else if stream.master {
                    t!("Master output").to_string()
                } else {
                    t!("Follower output").to_string()
                };
                let prefix = format!("{} {kind}", index + 1);
                rows.push((
                    t!("%{prefix}: actual stream", prefix = prefix).to_string(),
                    format!(
                        "{} Hz; {}; {} channels; {} frames ({:.2} ms)",
                        stream.actual.sample_rate,
                        t!(stream.actual.format.title()),
                        stream.actual.channels,
                        stream.actual.buffer_frames,
                        f64::from(stream.actual.buffer_frames) * 1000.0
                            / f64::from(stream.actual.sample_rate),
                    ),
                ));
                rows.push((
                    t!("%{prefix}: backend / mode", prefix = prefix).to_string(),
                    match stream.requested.backend {
                        AudioBackend::Asio => t!("ASIO driver (driver owns mode)").to_string(),
                        AudioBackend::Wasapi => format!("WASAPI / {:?}", stream.requested.mode),
                    },
                ));
                rows.push((
                    t!("%{prefix}: requested buffer", prefix = prefix).to_string(),
                    stream
                        .requested
                        .period_frames
                        .map(|frames| format!("{frames} frames"))
                        .unwrap_or_else(|| t!("Driver default").to_string()),
                ));
                rows.push((
                    t!("%{prefix}: bridge counters", prefix = prefix).to_string(),
                    t!(
                        "Dropped %{dropped} / silence %{silent} frames",
                        dropped = stream.dropped_frames,
                        silent = stream.silent_frames,
                    )
                    .to_string(),
                ));
                if stream.requested.backend == AudioBackend::Asio {
                    rows.push((
                        t!("%{prefix}: ASIO callback counters", prefix = prefix).to_string(),
                        t!(
                            "%{callbacks} callbacks; capture dropped %{dropped} / render silence %{silent} frames (includes startup)",
                            callbacks = stream.native_callbacks,
                            dropped = stream.native_dropped_frames,
                            silent = stream.native_silent_frames,
                        )
                        .to_string(),
                    ));
                    rows.push((
                        t!("%{prefix}: driver-reported latency", prefix = prefix).to_string(),
                        stream
                            .driver_latency
                            .map(|latency| {
                                t!(
                                    "Input %{input} / output %{output} frames; excludes application queues",
                                    input = latency.input_frames,
                                    output = latency.output_frames,
                                )
                                .to_string()
                            })
                            .unwrap_or_else(|| {
                                t!("Unavailable (not measured round-trip latency)").to_string()
                            }),
                    ));
                }
            }
            return rows;
        }
    }
    vec![(t!("Actual native timing").to_string(), "—".into())]
}

pub fn elapsed_text(snapshot: &AppSnapshot) -> String {
    let secs = snapshot.elapsed.as_secs();
    if secs == 0 && snapshot.phase != SessionPhase::Running {
        "—".into()
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

pub fn control_feedback(snapshot: &AppSnapshot, pending_request: bool) -> String {
    let control = &snapshot.control;
    if pending_request {
        return t!("Applying changes...").into();
    }
    if control.accepted_pending > 0 {
        return t!("Applying changes...").into();
    }
    if let Some(reply) = control.last_result {
        use moiren_core::protocol::ReplyCode;
        return match reply.code {
            ReplyCode::Applied | ReplyCode::AppliedLate => t!("Changes applied").into(),
            ReplyCode::Accepted => t!("Applying changes...").into(),
            ReplyCode::StaleRevision | ReplyCode::StaleEpoch => {
                t!("Audio routing changed. Try again.").into()
            }
            ReplyCode::UnknownParameter => t!("This control is no longer available.").into(),
            ReplyCode::InvalidValue => t!("Choose a valid parameter value.").into(),
            ReplyCode::QueueFull => t!("Audio controls are busy. Try again.").into(),
            ReplyCode::OutOfOrder | ReplyCode::InvalidTime => {
                t!("Unable to apply this change. Try again.").into()
            }
        };
    }
    if snapshot.phase == SessionPhase::Running {
        t!("No parameter changes requested").into()
    } else {
        t!("Next start · engine stopped").into()
    }
}
