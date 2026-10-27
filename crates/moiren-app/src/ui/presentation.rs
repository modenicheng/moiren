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
pub(crate) fn fresh_observation(
    phase: SessionPhase,
    generation: SessionGeneration,
    status_generation: SessionGeneration,
    running: bool,
) -> bool {
    phase == SessionPhase::Running && generation == status_generation && running
}

/// The measured `(sample rate, buffer frames)` of the fresh master output
/// stream. `None` when idle, off Windows, or the observation is stale:
/// callers fall back to configured values instead of plausible-looking
/// stale truth.
#[cfg(windows)]
pub(crate) fn measured_master_output(snapshot: &AppSnapshot) -> Option<(u32, u32)> {
    let status = snapshot
        .professional_observations
        .as_ref()
        .filter(|status| {
            fresh_observation(
                snapshot.phase,
                snapshot.generation,
                status.generation,
                status.observation.running,
            )
        })?;
    let stream = status
        .observation
        .streams
        .iter()
        .find(|stream| stream.master && !stream.input)?;
    Some((stream.actual.sample_rate, stream.actual.buffer_frames))
}

/// Non-Windows builds carry no professional observation snapshot; callers
/// fall back to configured values.
#[cfg(not(windows))]
pub(crate) fn measured_master_output(_: &AppSnapshot) -> Option<(u32, u32)> {
    None
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
                    t!("Input").to_string()
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

/// A linear gain as a signed dB readout; exact digital silence is −∞ dB.
/// Shared by every fader/route/inspector readout so one spelling of silence
/// and one decimal shape appear everywhere.
pub fn db_text(gain: f64) -> String {
    if gain == 0. {
        "−∞ dB".into()
    } else {
        format!("{:+.1} dB", 20. * gain.log10())
    }
}

/// A level amplitude as a signed dBFS readout; exact digital silence is
/// −∞ dBFS, spelled with the same U+2212 minus as [`db_text`].
pub fn dbfs_text(amplitude: f32) -> String {
    if amplitude <= 0. {
        "−∞ dBFS".into()
    } else {
        format!("{:+.1} dBFS", 20. * f64::from(amplitude).log10())
    }
}

/// One multiplicative ±`delta_db` step of a linear gain, clamped to
/// `min..=max`. Exact digital silence has no dB domain: multiplying zero
/// keeps it exactly silent, so a surface that wants silence to become
/// audible seeds a nonzero value itself (the Gain inspector does).
pub fn step_db(gain: f64, delta_db: f64, min: f64, max: f64) -> f64 {
    if gain <= 0. {
        return gain;
    }
    (gain * 10f64.powf(delta_db / 20.)).clamp(min, max)
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

#[cfg(test)]
mod tests {
    use super::{db_text, dbfs_text, step_db};

    #[test]
    fn db_text_reads_signed_db_and_one_infinity_for_silence() {
        assert_eq!(db_text(0.), "−∞ dB");
        assert_eq!(db_text(1.), "+0.0 dB");
        assert_eq!(db_text(10f64.powf(-6. / 20.)), "-6.0 dB");
        assert_eq!(db_text(16.), "+24.1 dB");
    }

    #[test]
    fn dbfs_text_matches_db_text_but_in_the_meter_unit() {
        assert_eq!(dbfs_text(0.), "−∞ dBFS");
        assert_eq!(dbfs_text(1.), "+0.0 dBFS");
        assert_eq!(dbfs_text(10f32.powf(-6. / 20.)), "-6.0 dBFS");
        // Negative amplitudes are as absent as silence: ballistics never
        // produce them, and silence is the honest rendering if they appear.
        assert_eq!(dbfs_text(-0.5), "−∞ dBFS");
    }

    #[test]
    fn step_db_moves_one_db_and_clamps_silence_is_sticky() {
        let one_db = 10f64.powf(1. / 20.);
        assert!((step_db(1., 1., 0., 16.) - one_db).abs() < 1e-12);
        assert!((step_db(one_db, -1., 0., 16.) - 1.).abs() < 1e-12);
        // Silence has no dB domain and stays exactly silent.
        assert_eq!(step_db(0., 1., 0., 16.), 0.);
        assert_eq!(step_db(0., -1., 0., 16.), 0.);
        // The Gain node domain clamps at amplitude 16.
        assert_eq!(step_db(16., 1., 0., 16.), 16.);
        // Send gains have no upper limit by design.
        assert!((step_db(16., 1., 0., f64::INFINITY) - 16. * one_db).abs() < 1e-9);
    }
}
