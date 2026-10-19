//! Pure text projections from [`AppSnapshot`]. These functions format
//! snapshot state for display; they hold no UI state and dispatch no intent.

use crate::service::{AppSnapshot, AudioBackend, SessionGeneration, SessionPhase};

/// Display-level stale guard mirroring the service's write-side filter. The
/// owner identity is not published to the UI, so this deliberately checks only
/// phase, generation, and the observation's own running flag.
fn fresh_observation(
    phase: SessionPhase,
    generation: SessionGeneration,
    status_generation: SessionGeneration,
    running: bool,
) -> bool {
    phase == SessionPhase::Running && generation == status_generation && running
}

pub fn phase_text(snapshot: &AppSnapshot) -> &'static str {
    match snapshot.phase {
        SessionPhase::Idle => "Idle",
        SessionPhase::Starting => "Starting",
        SessionPhase::Running => "Running",
        SessionPhase::Stopping => "Stopping",
        SessionPhase::Failed => "Failed",
        SessionPhase::Exiting => "Exiting",
        SessionPhase::Exited => "Exited",
    }
}

pub fn latency_apply_text(snapshot: &AppSnapshot) -> &'static str {
    if snapshot.audio_settings != crate::service::AudioSettings::default() {
        return "Explicit stream buffers control professional audio latency";
    }
    match snapshot.active_latency_profile {
        Some(active) if active == snapshot.latency_profile => "Active profile matches selection",
        Some(_) => "Pending: apply graph or start again to use selected profile",
        None => "Selected profile applies on next Apply / Start",
    }
}

/// Only the currently running owner can supply timing. In particular, a
/// stopped or replaced owner must not leave plausible-looking stale values.
pub fn native_timing_rows(snapshot: &AppSnapshot) -> Vec<(String, String)> {
    #[cfg(windows)]
    {
        use moiren_windows_audio::engine_backend::NativeStreamKind;
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
                    "Processing".into(),
                    format!(
                        "{} Hz; {}; {} frames",
                        settings.processing.sample_rate,
                        settings.processing.precision.title(),
                        settings.processing.block_frames,
                    ),
                ));
            }
            for (index, stream) in status.observation.streams.iter().enumerate() {
                let kind = if stream.input {
                    "Input"
                } else if stream.master {
                    "Master output"
                } else {
                    "Follower output"
                };
                let prefix = format!("{} {kind}", index + 1);
                rows.push((
                    format!("{prefix}: actual stream"),
                    format!(
                        "{} Hz; {}; {} channels; {} frames ({:.2} ms)",
                        stream.actual.sample_rate,
                        stream.actual.format.title(),
                        stream.actual.channels,
                        stream.actual.buffer_frames,
                        f64::from(stream.actual.buffer_frames) * 1000.0
                            / f64::from(stream.actual.sample_rate),
                    ),
                ));
                rows.push((
                    format!("{prefix}: backend / mode"),
                    match stream.requested.backend {
                        AudioBackend::Asio => "ASIO driver (driver owns mode)".into(),
                        AudioBackend::Wasapi => format!("WASAPI / {:?}", stream.requested.mode),
                    },
                ));
                rows.push((
                    format!("{prefix}: requested buffer"),
                    stream
                        .requested
                        .period_frames
                        .map(|frames| format!("{frames} frames"))
                        .unwrap_or_else(|| "Driver default".into()),
                ));
                rows.push((
                    format!("{prefix}: bridge counters"),
                    format!(
                        "Dropped {} / silence {} frames",
                        stream.dropped_frames, stream.silent_frames,
                    ),
                ));
                if stream.requested.backend == AudioBackend::Asio {
                    rows.push((
                        format!("{prefix}: ASIO callback counters"),
                        format!("{} callbacks; capture dropped {} / render silence {} frames (includes startup)", stream.native_callbacks, stream.native_dropped_frames, stream.native_silent_frames),
                    ));
                    rows.push((
                        format!("{prefix}: driver-reported latency"),
                        stream
                            .driver_latency
                            .map(|latency| {
                                format!(
                                    "Input {} / output {} frames; excludes application queues",
                                    latency.input_frames, latency.output_frames
                                )
                            })
                            .unwrap_or_else(|| {
                                "Unavailable (not measured round-trip latency)".into()
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
                let kind = match stream.kind {
                    NativeStreamKind::MasterOutput => "Master output",
                    NativeStreamKind::FollowerOutput => "Follower output",
                    NativeStreamKind::PhysicalInput => "Physical input",
                    NativeStreamKind::ProcessInput => "Application input",
                };
                let prefix = format!("{} {kind}", index + 1);
                rows.push((
                    format!("{prefix}: actual timing"),
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
                                .unwrap_or_else(|| "period unknown".into());
                            format!(
                                "{} Hz; {period}; capacity {} frames",
                                timing.sample_rate, timing.capacity_frames
                            )
                        })
                        .unwrap_or_else(|| "Virtual capture; no hardware period".into()),
                ));
                if let Some(period) = stream.shared_period {
                    rows.push((
                        format!("{prefix}: period request"),
                        period
                            .desired_period_ms
                            .map(|ms| format!("{ms:.2} ms requested"))
                            .unwrap_or_else(|| "Device default requested".into()),
                    ));
                    rows.push((
                        format!("{prefix}: initialization"),
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
                        format!("{prefix}: render queue"),
                        format!(
                            "Target {} / capacity {} frames",
                            queue.target_frames, queue.capacity_frames
                        ),
                    ));
                }
                if let Some(queue) = stream.input_queue {
                    rows.push((
                        format!("{prefix}: bridge queue"),
                        format!(
                            "Prime {} / capacity {} frames",
                            queue.prime_frames, queue.capacity_frames
                        ),
                    ));
                }
            }
            for bridge in &status.observation.bridges {
                let prefix = status
                    .observation
                    .streams
                    .iter()
                    .position(|stream| stream.node == bridge.node)
                    .map(|index| format!("Stream {}", index + 1))
                    .unwrap_or_else(|| "Unmapped stream".into());
                rows.push((
                    format!("{prefix}: live queue / clock"),
                    format!(
                        "{} frames; {:+.1} ppm",
                        bridge.counters.fill_frames, bridge.counters.correction_ppm
                    ),
                ));
                rows.push((
                    format!("{prefix}: faults / recovery"),
                    format!(
                        "Underrun {} / dropped {} frames; resets {} / backlog recoveries {}",
                        bridge.counters.underrun_frames,
                        bridge.counters.dropped_frames,
                        bridge.counters.resets,
                        bridge.counters.backlog_recoveries,
                    ),
                ));
            }
            return rows;
        }
    }
    vec![("Actual native timing".into(), "—".into())]
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
        return "Applying changes...".into();
    }
    if control.accepted_pending > 0 {
        return "Applying changes...".into();
    }
    if let Some(reply) = control.last_result {
        use moiren_core::protocol::ReplyCode;
        return match reply.code {
            ReplyCode::Applied | ReplyCode::AppliedLate => "Changes applied".into(),
            ReplyCode::Accepted => "Applying changes...".into(),
            ReplyCode::StaleRevision | ReplyCode::StaleEpoch => {
                "Audio routing changed. Try again.".into()
            }
            ReplyCode::UnknownParameter => "This control is no longer available.".into(),
            ReplyCode::InvalidValue => "Choose a valid parameter value.".into(),
            ReplyCode::QueueFull => "Audio controls are busy. Try again.".into(),
            ReplyCode::OutOfOrder | ReplyCode::InvalidTime => {
                "Unable to apply this change. Try again.".into()
            }
        };
    }
    if snapshot.phase == SessionPhase::Running {
        "No parameter changes requested".into()
    } else {
        "Next start · engine stopped".into()
    }
}
