//! Statistics-only phases and ownership of the experiment's child processes.
mod generator;
mod native;
use anyhow::{Context, Result, bail};
use moiren_windows_audio::{
    capture::PreparedCapture,
    process_loopback::{ProcessLoopbackOptions, inspect_process, start_process_capture},
};
use native::{Apartment, EndpointTap, OwnedGenerator, Restoration, SessionGuard};
use serde::Serialize;
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator},
        System::Com::{CLSCTX_ALL, CoCreateInstance},
    },
    core::HSTRING,
};
#[derive(Default, Serialize)]
struct Samples {
    count: u64,
    nonzero: u64,
    peak: f64,
    invalid: u64,
    #[serde(skip)]
    squares: f64,
    rms: Option<f64>,
}
impl Samples {
    fn add(&mut self, value: f64) {
        if !value.is_finite() {
            self.invalid += 1;
            return;
        }
        self.count += 1;
        self.nonzero += u64::from(value != 0.0);
        self.peak = self.peak.max(value.abs());
        self.squares += value * value;
    }
    fn finish(&mut self) {
        self.rms = (self.count != 0).then(|| (self.squares / self.count as f64).sqrt());
    }
}

// Two seconds at 48 kHz stereo normally yield about 192,000 samples.
// Require enough valid data to distinguish sustained silence from a stopped
// stream, while tolerating scheduling jitter and initial bridge priming.
const MIN_PHASE_SAMPLES: u64 = 48_000;

#[derive(Default, Serialize)]
struct TransferDiagnostics {
    requested_frames: u64,
    transferred_frames: u64,
    shortfall_frames: u64,
    xruns: u64,
    discontinuities: u64,
    worker_finished: bool,
}
impl TransferDiagnostics {
    fn record(&mut self, requested: usize, transferred: usize, xruns: u64, discontinuity: bool) {
        self.requested_frames += requested as u64;
        self.transferred_frames += transferred as u64;
        self.shortfall_frames += requested.saturating_sub(transferred) as u64;
        self.xruns += xruns;
        self.discontinuities += u64::from(discontinuity);
    }
    fn check_worker(&mut self, phase: &str, client: usize, finished: bool) -> Result<()> {
        self.worker_finished |= finished;
        if self.worker_finished {
            // Report only the phase and local client index, never backend
            // terminal errors that could contain process or endpoint identity.
            bail!("phase {phase}: capture client {client} worker ended unexpectedly");
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct Phase {
    name: &'static str,
    endpoint: Samples,
    capture_a: Samples,
    capture_b: Samples,
    capture_a_transfer: TransferDiagnostics,
    capture_b_transfer: TransferDiagnostics,
}
impl Phase {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            endpoint: Samples::default(),
            capture_a: Samples::default(),
            capture_b: Samples::default(),
            capture_a_transfer: TransferDiagnostics::default(),
            capture_b_transfer: TransferDiagnostics::default(),
        }
    }
    fn finish(&mut self) {
        self.endpoint.finish();
        self.capture_a.finish();
        self.capture_b.finish();
    }
    fn validate(&self) -> Result<()> {
        for (stream, samples) in [
            ("endpoint", &self.endpoint),
            ("capture client 0", &self.capture_a),
            ("capture client 1", &self.capture_b),
        ] {
            if samples.count < MIN_PHASE_SAMPLES || samples.invalid != 0 {
                bail!(
                    "phase {}: {stream} has insufficient valid samples (valid={}, invalid={}, minimum={MIN_PHASE_SAMPLES})",
                    self.name,
                    samples.count,
                    samples.invalid,
                );
            }
        }
        Ok(())
    }
}
fn measure(
    phase: &mut Phase,
    endpoint: &EndpointTap,
    captures: &mut [PreparedCapture; 2],
) -> Result<()> {
    let start = Instant::now();
    let mut last = start;
    let mut output = [0.0f32; 4096];
    while start.elapsed() < Duration::from_millis(2500) {
        std::thread::sleep(Duration::from_millis(5));
        let now = Instant::now();
        let frames = (now.duration_since(last).as_secs_f64() * 48_000.0).round() as usize;
        last = now;
        let mut endpoint_samples = Samples::default();
        endpoint
            .drain(&mut endpoint_samples)
            .map_err(|_| anyhow::anyhow!("phase {}: endpoint drain failed", phase.name))?;
        let settled = start.elapsed() >= Duration::from_millis(500);
        if settled {
            phase.endpoint.count += endpoint_samples.count;
            phase.endpoint.nonzero += endpoint_samples.nonzero;
            phase.endpoint.invalid += endpoint_samples.invalid;
            phase.endpoint.squares += endpoint_samples.squares;
            phase.endpoint.peak = phase.endpoint.peak.max(endpoint_samples.peak);
        }
        for (index, (capture, (samples, transfer))) in captures
            .iter_mut()
            .zip([
                (&mut phase.capture_a, &mut phase.capture_a_transfer),
                (&mut phase.capture_b, &mut phase.capture_b_transfer),
            ])
            .enumerate()
        {
            transfer.check_worker(phase.name, index, capture.session.is_finished())?;
            let requested = frames.min(2048);
            let report = capture
                .source
                .read_interleaved(&mut output[..requested * 2])
                .map_err(|_| {
                    anyhow::anyhow!(
                        "phase {}: capture client {index} transfer failed",
                        phase.name
                    )
                })?;
            if settled {
                transfer.record(
                    requested,
                    report.transferred_frames,
                    report.xruns,
                    report.discontinuity,
                );
                for &sample in &output[..report.transferred_frames * 2] {
                    samples.add(f64::from(sample));
                }
            }
            transfer.check_worker(phase.name, index, capture.session.is_finished())?;
        }
    }
    for (index, (capture, transfer)) in captures
        .iter()
        .zip([&mut phase.capture_a_transfer, &mut phase.capture_b_transfer])
        .enumerate()
    {
        transfer.check_worker(phase.name, index, capture.session.is_finished())?;
    }
    phase.validate()
}

#[derive(Serialize)]
struct Experiment {
    phases: Vec<Phase>,
    restoration: Restoration,
    failure: Option<String>,
}
fn run(endpoint_id: &str) -> Result<Experiment> {
    // The child guard is declared first so restoration and COM release
    // finish before its private session disappears at child termination.
    let mut child = OwnedGenerator(
        Command::new(std::env::current_exe()?)
            .args(["--owned-tone", endpoint_id])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let _apartment = Apartment::new()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(endpoint_id))? };
    let mut session = SessionGuard::find(&device, &mut child)?;
    let mut phases = Vec::new();
    let result: Result<()> = (|| {
        // Include preparation failures in the explicit restoration/report path.
        let target = inspect_process(child.0.id())?;
        let mut captures = [
            start_process_capture(ProcessLoopbackOptions::continuous(target.clone()))?,
            start_process_capture(ProcessLoopbackOptions::continuous(target))?,
        ];
        let endpoint = EndpointTap::new(&device)?;
        for (name, volume, muted) in [
            ("baseline", session.before_volume, false),
            ("muted", session.before_volume, true),
            ("restored_after_mute", session.before_volume, false),
            ("quarter_volume", 0.25, false),
            ("zero_volume", 0.0, false),
            (
                "restored_after_volume",
                session.before_volume,
                session.before_mute,
            ),
        ] {
            session.set(volume, muted)?;
            let mut phase = Phase::new(name);
            let result = measure(&mut phase, &endpoint, &mut captures);
            // Retain aggregate diagnostics for an interrupted phase as well;
            // the error still takes the existing explicit restoration path.
            phase.finish();
            phases.push(phase);
            result?;
        }
        if phases[0].endpoint.rms.unwrap_or(0.0) < 0.001
            || phases[0].capture_a.rms.unwrap_or(0.0) < 0.001
            || phases[0].capture_b.rms.unwrap_or(0.0) < 0.001
        {
            bail!("baseline has no usable test signal");
        }
        Ok(())
    })();
    let restoration = session
        .restore()
        .context("restoring owned generator session")?;
    Ok(Experiment {
        phases,
        restoration,
        failure: result.err().map(|error| error.to_string()),
    })
}

pub fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode, endpoint] if mode == "--owned-tone" => generator::run(endpoint.clone()),
        [mode, endpoint] if mode == "--output" => {
            // A fresh child for the second run tests process/session restart.
            let results = [run(endpoint)?, run(endpoint)?];
            serde_json::to_writer_pretty(std::io::stdout().lock(), &results)?;
            println!();
            if results
                .iter()
                .any(|r| !r.restoration.verified || r.failure.is_some())
            {
                bail!("experiment or verified restoration failed");
            }
            Ok(())
        }
        _ => bail!(
            "takeover_probe --output <native-48k-stereo-endpoint-id>; spawns its own low-volume tone, tests mute/volume and two parallel captures, then restores and repeats with a new child"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_phase(name: &'static str) -> Phase {
        let mut phase = Phase::new(name);
        for samples in [
            &mut phase.endpoint,
            &mut phase.capture_a,
            &mut phase.capture_b,
        ] {
            for _ in 0..MIN_PHASE_SAMPLES {
                samples.add(0.0);
            }
        }
        phase.finish();
        phase
    }

    #[test]
    fn valid_silent_samples_pass_every_phase() {
        for name in [
            "baseline",
            "muted",
            "restored_after_mute",
            "quarter_volume",
            "zero_volume",
            "restored_after_volume",
        ] {
            let phase = silent_phase(name);
            phase.validate().unwrap();
            assert_eq!(phase.endpoint.rms, Some(0.0));
            assert_eq!(phase.capture_a.rms, Some(0.0));
            assert_eq!(phase.capture_b.rms, Some(0.0));
        }
    }

    #[test]
    fn each_stream_requires_sustained_valid_data() {
        for index in 0..3 {
            for count in [0, MIN_PHASE_SAMPLES - 1] {
                let mut phase = silent_phase("zero_volume");
                let samples = match index {
                    0 => &mut phase.endpoint,
                    1 => &mut phase.capture_a,
                    _ => &mut phase.capture_b,
                };
                *samples = Samples {
                    count,
                    ..Samples::default()
                };
                phase.finish();
                let error = phase.validate().unwrap_err().to_string();
                assert!(error.contains("phase zero_volume"));
                assert!(error.contains("insufficient valid samples"));
                if count == 0 {
                    let json = serde_json::to_value(&phase).unwrap();
                    let stream = ["endpoint", "capture_a", "capture_b"][index];
                    assert!(json[stream]["rms"].is_null());
                }
            }
        }
    }

    #[test]
    fn nonfinite_samples_cannot_count_as_valid_silence() {
        let mut phase = silent_phase("muted");
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            phase.capture_a.add(value);
        }
        phase.finish();
        assert_eq!(phase.capture_a.count, MIN_PHASE_SAMPLES);
        assert_eq!(phase.capture_a.invalid, 3);
        assert!(phase.validate().is_err());
        assert_eq!(phase.capture_a.rms, Some(0.0));
    }

    #[test]
    fn terminal_worker_fails_even_after_valid_transfer() {
        let mut transfer = TransferDiagnostics::default();
        transfer.record(96_000, 96_000, 0, false);
        transfer.check_worker("muted", 1, false).unwrap();
        let error = transfer
            .check_worker("muted", 1, true)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "phase muted: capture client 1 worker ended unexpectedly"
        );
        assert!(transfer.worker_finished);
        assert!(transfer.check_worker("muted", 1, false).is_err());
    }

    #[test]
    fn interrupted_phase_retains_transfer_diagnostics() {
        let mut phase = Phase::new("restored_after_mute");
        phase.capture_a_transfer.record(240, 200, 1, true);
        phase.capture_a_transfer.record(240, 0, 0, false);
        assert!(
            phase
                .capture_a_transfer
                .check_worker(phase.name, 0, true)
                .is_err()
        );
        phase.finish();
        let json = serde_json::to_value(&phase).unwrap();
        let transfer = &json["capture_a_transfer"];
        assert_eq!(transfer["requested_frames"], 480);
        assert_eq!(transfer["transferred_frames"], 200);
        assert_eq!(transfer["shortfall_frames"], 280);
        assert_eq!(transfer["xruns"], 1);
        assert_eq!(transfer["discontinuities"], 1);
        assert_eq!(transfer["worker_finished"], true);
        assert_eq!(json["capture_a"]["count"], 0);
        assert!(json["capture_a"]["rms"].is_null());
    }
}
