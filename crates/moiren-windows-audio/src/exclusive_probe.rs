//! Opt-in endpoint occupation experiment, separate from the managed backend.
//! A positive result means process audio was observed, not selective takeover.
use crate::{
    catalog::{ApiFailure, FormatSnapshot},
    owner::{Apartment, Process, Streaming},
    process_loopback::{ProcessIdentity, inspect_process},
    stats::CaptureSummary,
};
use anyhow::Context;
use serde::Serialize;
use std::time::Duration;

mod capture;
mod render;

const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default, Serialize)]
pub struct WindowReport {
    pub phase: &'static str,
    pub elapsed_seconds: f64,
    pub signal_frames: u64,
    pub summary: CaptureSummary,
}

#[derive(Default, Serialize)]
pub struct ExclusiveReport {
    pub started: bool,
    pub stop_succeeded: bool,
    pub last_stage: &'static str,
    pub format: Option<FormatSnapshot>,
    pub buffer_frames: u32,
    pub alignment_retried: bool,
    pub submitted_frames: u64,
    pub callbacks: u64,
    pub elapsed_seconds: f64,
    pub errors: Vec<ApiFailure>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    ExperimentFailed,
    BaselineInconclusive,
    SignalObservedDuringExclusive,
    NoSignalDuringExclusive,
    IntermittentSignalDuringExclusive,
}

#[derive(Serialize)]
pub struct ExclusiveProbeReport {
    pub schema_version: u32,
    pub target: ProcessIdentity,
    pub endpoint_id: String,
    pub seconds_per_window: u32,
    pub settle_ms: u64,
    pub capture_stop_succeeded: bool,
    pub windows: Vec<WindowReport>,
    pub exclusive: ExclusiveReport,
    pub errors: Vec<String>,
    pub observation: Observation,
    /// Physical output suppression needs a separate listening/measurement check.
    pub endpoint_silence_verified: bool,
}

fn sustained_signal(window: &WindowReport) -> bool {
    // Require signal over >=80% of wall-clock time, not just one residual packet.
    window.elapsed_seconds > 0.0
        && window.signal_frames as f64 >= window.elapsed_seconds * 48_000.0 * 0.8
        && window.summary.non_finite_samples == 0
}

fn observation(report: &ExclusiveProbeReport) -> Observation {
    if !report.errors.is_empty()
        || !report.capture_stop_succeeded
        || !report.exclusive.started
        || !report.exclusive.stop_succeeded
        || !report.exclusive.errors.is_empty()
        || report.windows.len() != 3
    {
        return Observation::ExperimentFailed;
    }
    if !sustained_signal(&report.windows[0]) {
        return Observation::BaselineInconclusive;
    }
    if sustained_signal(&report.windows[1]) {
        Observation::SignalObservedDuringExclusive
    } else if report.windows[1].signal_frames == 0 {
        Observation::NoSignalDuringExclusive
    } else {
        Observation::IntermittentSignalDuringExclusive
    }
}

/// Temporarily silences ALL shared playback on the explicitly selected endpoint.
/// Does not change endpoint settings or save PCM. Hardware opt-in only.
pub fn run(pid: u32, endpoint_id: String, seconds: u32) -> anyhow::Result<ExclusiveProbeReport> {
    anyhow::ensure!(
        (1..=60).contains(&seconds),
        "--seconds must be within 1..=60"
    );
    anyhow::ensure!(
        !endpoint_id.trim().is_empty() && !endpoint_id.contains('\0'),
        "a nonempty endpoint ID without NUL is required"
    );
    let target = inspect_process(pid)?;
    let _apartment = Apartment::new()?;
    let process = Process::open(pid)?;
    anyhow::ensure!(
        target.same_process(&process.identity),
        "target PID was recycled"
    );
    let mut report = ExclusiveProbeReport {
        schema_version: 1,
        target,
        endpoint_id,
        seconds_per_window: seconds,
        settle_ms: SETTLE.as_millis() as u64,
        capture_stop_succeeded: false,
        windows: Vec::with_capacity(3),
        exclusive: ExclusiveReport::default(),
        errors: Vec::new(),
        observation: Observation::ExperimentFailed,
        endpoint_silence_verified: false,
    };
    // One client survives all phases. Re-activation could hide invalidation.
    let capture = capture::Loopback::prepare(&process)?;
    let mut streaming = Streaming::start(&capture.client)?;
    let duration = Duration::from_secs(u64::from(seconds));
    let result = (|| -> anyhow::Result<()> {
        report
            .windows
            .push(capture.sample(&process, "baseline", SETTLE, duration, || Ok(()))?);
        let mut exclusive = render::Owner::start(report.endpoint_id.clone())?;
        let during = capture.sample(&process, "exclusive", SETTLE, duration, || {
            exclusive.check()
        });
        // Join even when capture fails, releasing exclusive ownership first.
        report.exclusive = exclusive.finish()?;
        report.windows.push(during?);
        report
            .windows
            .push(capture.sample(&process, "released", SETTLE, duration, || Ok(()))?);
        Ok(())
    })();
    if let Err(error) = result {
        report.errors.push(format!("{error:#}"));
    }
    match streaming.stop().context("Stop(process loopback)") {
        Ok(()) => report.capture_stop_succeeded = true,
        Err(error) => report.errors.push(format!("{error:#}")),
    }
    report.observation = observation(&report);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(signal_frames: [u64; 3]) -> ExclusiveProbeReport {
        ExclusiveProbeReport {
            schema_version: 1,
            target: ProcessIdentity {
                pid: 1,
                creation_time_100ns: 1,
                executable_name: "test".into(),
            },
            endpoint_id: "test".into(),
            seconds_per_window: 2,
            settle_ms: 500,
            capture_stop_succeeded: true,
            windows: ["baseline", "exclusive", "released"]
                .into_iter()
                .zip(signal_frames)
                .map(|(phase, signal_frames)| WindowReport {
                    phase,
                    elapsed_seconds: 2.0,
                    signal_frames,
                    ..Default::default()
                })
                .collect(),
            exclusive: ExclusiveReport {
                started: true,
                stop_succeeded: true,
                ..Default::default()
            },
            errors: Vec::new(),
            observation: Observation::ExperimentFailed,
            endpoint_silence_verified: false,
        }
    }

    #[test]
    fn silence_requires_a_valid_baseline() {
        assert_eq!(
            observation(&report([0, 0, 0])),
            Observation::BaselineInconclusive
        );
        assert_eq!(
            observation(&report([96_000, 0, 96_000])),
            Observation::NoSignalDuringExclusive
        );
    }

    #[test]
    fn a_residual_packet_cannot_establish_sustained_capture() {
        assert_eq!(
            observation(&report([96_000, 480, 96_000])),
            Observation::IntermittentSignalDuringExclusive
        );
        assert_eq!(
            observation(&report([96_000; 3])),
            Observation::SignalObservedDuringExclusive
        );
    }

    #[test]
    fn failure_overrides_signal_and_partial_windows() {
        let mut report = report([96_000; 3]);
        report.exclusive.stop_succeeded = false;
        assert_eq!(observation(&report), Observation::ExperimentFailed);
        report.exclusive.stop_succeeded = true;
        report.windows.pop();
        assert_eq!(observation(&report), Observation::ExperimentFailed);
    }

    #[test]
    fn invalid_options_are_rejected_before_opening_audio_devices() {
        assert!(run(0, "test".into(), 0).is_err());
        assert!(run(0, "test".into(), 61).is_err());
        assert!(run(0, String::new(), 1).is_err());
        assert!(run(0, "test\0endpoint".into(), 1).is_err());
        assert!(run(0, "test".into(), 1).is_err());
        assert!(run(std::process::id(), "test".into(), 1).is_err());
    }

    #[test]
    fn non_finite_samples_cannot_establish_sustained_capture() {
        let mut report = report([96_000; 3]);
        report.windows[1].summary.non_finite_samples = 1;
        assert_eq!(
            observation(&report),
            Observation::IntermittentSignalDuringExclusive
        );
        report.windows[0].summary.non_finite_samples = 1;
        assert_eq!(observation(&report), Observation::BaselineInconclusive);
    }
}
