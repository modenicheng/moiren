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
    #[serde(skip)]
    squares: f64,
    rms: f64,
}
impl Samples {
    fn add(&mut self, value: f64) {
        self.count += 1;
        self.nonzero += u64::from(value != 0.0);
        self.peak = self.peak.max(value.abs());
        self.squares += value * value;
    }
    fn finish(&mut self) {
        self.rms = (self.squares / self.count.max(1) as f64).sqrt();
    }
}

#[derive(Serialize)]
struct Phase {
    name: &'static str,
    endpoint: Samples,
    capture_a: Samples,
    capture_b: Samples,
}
fn measure(
    name: &'static str,
    endpoint: &EndpointTap,
    captures: &mut [PreparedCapture; 2],
) -> Result<Phase> {
    let mut phase = Phase {
        name,
        endpoint: Samples::default(),
        capture_a: Samples::default(),
        capture_b: Samples::default(),
    };
    let start = Instant::now();
    let mut last = start;
    let mut output = [0.0f32; 4096];
    while start.elapsed() < Duration::from_millis(2500) {
        std::thread::sleep(Duration::from_millis(5));
        let now = Instant::now();
        let frames = (now.duration_since(last).as_secs_f64() * 48_000.0).round() as usize;
        last = now;
        let mut endpoint_samples = Samples::default();
        endpoint.drain(&mut endpoint_samples)?;
        let settled = start.elapsed() >= Duration::from_millis(500);
        if settled {
            phase.endpoint.count += endpoint_samples.count;
            phase.endpoint.nonzero += endpoint_samples.nonzero;
            phase.endpoint.squares += endpoint_samples.squares;
            phase.endpoint.peak = phase.endpoint.peak.max(endpoint_samples.peak);
        }
        for (capture, samples) in captures
            .iter_mut()
            .zip([&mut phase.capture_a, &mut phase.capture_b])
        {
            let report = capture
                .source
                .read_interleaved(&mut output[..frames.min(2048) * 2])?;
            if settled {
                for &sample in &output[..report.transferred_frames * 2] {
                    samples.add(f64::from(sample));
                }
            }
        }
    }
    phase.endpoint.finish();
    phase.capture_a.finish();
    phase.capture_b.finish();
    Ok(phase)
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
            phases.push(measure(name, &endpoint, &mut captures)?);
        }
        if phases[0].endpoint.rms < 0.001
            || phases[0].capture_a.rms < 0.001
            || phases[0].capture_b.rms < 0.001
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
