//! Pure text projections from [`AppSnapshot`]. These functions format
//! snapshot state for display; they hold no UI state and dispatch no intent.

use crate::service::{AppSnapshot, SessionPhase};
#[cfg(windows)]
use crate::service::{AudioBackend, SessionGeneration};
#[cfg(windows)]
use moiren_windows_audio::engine_backend::NativeStreamKind;
use rust_i18n::t;
use std::borrow::Cow;

mod export_diagnostics;
pub use export_diagnostics::{export_diagnostic_rows, windows_name_match_text};
#[cfg(windows)]
mod native_live;

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

#[cfg(windows)]
fn native_stream_kind_text(kind: NativeStreamKind) -> Cow<'static, str> {
    match kind {
        NativeStreamKind::MasterOutput => t!("Master output"),
        NativeStreamKind::FollowerOutput => t!("Follower output"),
        NativeStreamKind::PhysicalInput => t!("Physical input"),
        NativeStreamKind::ProcessInput => t!("Application input"),
        NativeStreamKind::VirtualPlayback => t!("Virtual playback"),
        NativeStreamKind::VirtualMicrophone => t!("Virtual microphone"),
    }
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
    if snapshot.audio_settings != crate::service::AudioSettings::default() {
        return t!("Explicit stream buffers control professional audio latency");
    }
    match snapshot.active_latency_profile {
        Some(active) if active == snapshot.latency_profile => {
            t!("Active profile matches selection")
        }
        Some(_) => t!("Selected profile activates on the next audio start"),
        None => t!("Selected profile activates on the next audio start"),
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
        if let Some(status) = snapshot.native_observations.as_ref().filter(|status| {
            fresh_observation(
                snapshot.phase,
                snapshot.generation,
                status.generation,
                status.observation.running,
            )
        }) {
            let mut rows = Vec::new();
            for (index, stream) in status.observation.streams.iter().enumerate() {
                let kind = native_stream_kind_text(stream.kind);
                let prefix = format!("{} {kind}", index + 1);
                rows.push((
                    t!("%{prefix}: actual timing", prefix = prefix).to_string(),
                    stream
                        .timing
                        .map(|timing| {
                            let period = timing
                                .period_frames
                                .map(|frames| {
                                    format!(
                                        "{frames} frames ({:.2} ms)",
                                        f64::from(frames) * 1000. / f64::from(timing.sample_rate)
                                    )
                                })
                                .unwrap_or_else(|| t!("period unknown").to_string());
                            t!(
                                "%{rate} Hz; %{period}; capacity %{capacity} frames",
                                rate = timing.sample_rate,
                                period = period,
                                capacity = timing.capacity_frames,
                            )
                            .to_string()
                        })
                        .unwrap_or_else(|| t!("Virtual capture; no hardware period").to_string()),
                ));
                if let Some(period) = stream.shared_period {
                    rows.push((
                        t!("%{prefix}: period request", prefix = prefix).to_string(),
                        period
                            .desired_period_ms
                            .map(|ms| t!("%{ms} ms requested", ms = format!("{ms:.2}")).to_string())
                            .unwrap_or_else(|| t!("Device default requested").to_string()),
                    ));
                    // The initialization value is {:?} debug output and stays in English.
                    rows.push((
                        t!("%{prefix}: initialization", prefix = prefix).to_string(),
                        format!(
                            "{:?}; fallback {}",
                            period.initialization_api,
                            period
                                .fallback_reason
                                .map(|reason| format!("{reason:?}"))
                                .unwrap_or_else(|| "none".into())
                        ),
                    ));
                }
                if let Some(queue) = stream.render_queue {
                    rows.push((
                        t!(
                            "%{prefix}: effective prepared render queue",
                            prefix = prefix
                        )
                        .to_string(),
                        t!(
                            "Target %{target} / capacity %{capacity} frames",
                            target = queue.target_frames,
                            capacity = queue.capacity_frames,
                        )
                        .to_string(),
                    ));
                }
                if let Some(queue) = stream.input_queue {
                    rows.push((
                        t!(
                            "%{prefix}: effective prepared bridge queue",
                            prefix = prefix
                        )
                        .to_string(),
                        t!(
                            "Prime %{prime} / capacity %{capacity} frames",
                            prime = queue.prime_frames,
                            capacity = queue.capacity_frames,
                        )
                        .to_string(),
                    ));
                }
            }
            for bridge in &status.observation.bridges {
                let prefix = status
                    .observation
                    .streams
                    .iter()
                    .position(|stream| stream.node == bridge.node)
                    .map(|index| t!("Stream %{index}", index = index + 1).to_string())
                    .unwrap_or_else(|| t!("Unmapped stream").to_string());
                rows.push((
                    t!("%{prefix}: live queue / clock", prefix = prefix).to_string(),
                    format!(
                        "{} frames; {:+.1} ppm",
                        bridge.counters.fill_frames, bridge.counters.correction_ppm
                    ),
                ));
                rows.push((
                    t!("%{prefix}: faults / recovery", prefix = prefix).to_string(),
                    t!(
                        "Underrun %{underrun} / dropped %{dropped} frames; resets %{resets} / backlog recoveries %{recoveries}",
                        underrun = bridge.counters.underrun_frames,
                        dropped = bridge.counters.dropped_frames,
                        resets = bridge.counters.resets,
                        recoveries = bridge.counters.backlog_recoveries,
                    )
                    .to_string(),
                ));
            }
            for probe in &status.observation.boundary_probes {
                let stream = status
                    .observation
                    .streams
                    .iter()
                    .position(|stream| stream.node == probe.node && stream.kind == probe.kind)
                    .map(|index| (index + 1).to_string())
                    .unwrap_or_else(|| t!("Unmapped stream").to_string());
                let prefix = format!("{stream} {}", native_stream_kind_text(probe.kind));
                rows.push((
                    t!("%{prefix}: prepared boundary capacity", prefix = prefix).to_string(),
                    t!(
                        "%{frames} native frames at 48 kHz (storage headroom)",
                        frames = probe.capacity_frames,
                    )
                    .to_string(),
                ));
                // Preparation's total target and RT's residual occupancy use
                // different observation points; neither is physical latency.
                rows.push((
                    t!(
                        "%{prefix}: total pre-transfer content target",
                        prefix = prefix
                    )
                    .to_string(),
                    probe
                        .target_frames
                        .map(|frames| {
                            t!("%{frames} native frames at 48 kHz", frames = frames).to_string()
                        })
                        .unwrap_or_else(|| t!("Unknown").to_string()),
                ));
                rows.push((
                    t!(
                        "%{prefix}: post-transfer occupancy / correction",
                        prefix = prefix
                    )
                    .to_string(),
                    t!(
                        "%{frames} native frames at 48 kHz; applied %{ppm} ppm",
                        frames = probe.level_frames,
                        ppm = format!("{:+.3}", f64::from(probe.applied_milli_ppm) / 1000.0),
                    )
                    .to_string(),
                ));
                rows.push((
                    t!("%{prefix}: cumulative boundary events", prefix = prefix).to_string(),
                    t!(
                        "Xrun %{xruns} / soft %{soft} events (not driver drops or silent frames)",
                        xruns = probe.xruns,
                        soft = probe.soft_events,
                    )
                    .to_string(),
                ));
            }
            native_live::append_rows(&mut rows, &status.observation);
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
