//! Opt-in, finite ASIO acceptance. No captured PCM is printed or persisted.
//! Option parsing, stream selection and the finite transfer loop with its
//! progress acceptance live in `moiren_windows_audio::professional::asio::
//! probe` (unit-tested there); this example keeps the CLI entry and the
//! win64 stream-driving report.

use anyhow::{Result, ensure};
use moiren_windows_audio::professional::asio::probe::{APPROVED_DRIVER, Options, parse_options};

struct StderrTrace;

impl log::Log for StderrTrace {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Debug
            && metadata.target().starts_with("moiren::audio::asio")
    }

    fn log(&self, record: &log::Record<'_>) {
        use std::io::Write;
        if self.enabled(record.metadata()) {
            // Owner/control-side only. Flush before the next native entry so
            // external timeout termination still preserves the last stage.
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "[asio-probe {:?} {}] {}",
                std::thread::current().id(),
                record.target(),
                record.args()
            );
            let _ = stderr.flush();
        }
    }

    fn flush(&self) {
        use std::io::Write;
        let _ = std::io::stderr().lock().flush();
    }
}

fn initialize_trace(explicit: bool) -> Result<()> {
    if explicit || std::env::var_os("MOIREN_ASIO_TRACE").is_some() {
        static LOGGER: StderrTrace = StderrTrace;
        log::set_logger(&LOGGER)
            .map_err(|error| anyhow::anyhow!("cannot install ASIO stderr trace logger: {error}"))?;
        log::set_max_level(log::LevelFilter::Debug);
        log::debug!(target: "moiren::audio::asio::probe", "trace enabled; native stage logs go to stderr; stdout remains metadata only");
    }
    Ok(())
}

#[cfg(all(windows, target_pointer_width = "64"))]
mod probe {
    use super::*;
    use moiren_windows_audio::professional::{
        asio::{
            AsioCapabilities, AsioStream,
            probe::{Transfers, run_stream, stream_options},
            query_capabilities,
        },
        native::{NativeStream, NativeStreamStats},
    };
    use serde::Serialize;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };

    #[derive(Debug, Default, Serialize)]
    struct NativeCounters {
        callbacks: u64,
        capture_dropped_frames: u64,
        render_silent_frames: u64,
    }

    impl From<NativeStreamStats> for NativeCounters {
        fn from(stats: NativeStreamStats) -> Self {
            Self {
                callbacks: stats.callbacks,
                capture_dropped_frames: stats.capture_dropped_frames,
                render_silent_frames: stats.render_silent_frames,
            }
        }
    }

    #[derive(Serialize)]
    struct StreamReport {
        requested_seconds: u64,
        elapsed_seconds: f64,
        sample_rate: u32,
        buffer_frames: u32,
        input: bool,
        output: bool,
        transfers: Option<Transfers>,
        native: NativeCounters,
        driver_latency_frames: Option<(u32, u32)>,
    }

    #[derive(Serialize)]
    struct Report {
        mode: &'static str,
        capabilities: AsioCapabilities,
        stream: Option<StreamReport>,
        error: Option<String>,
        scope: &'static str,
    }

    pub(super) fn run(options: Options) -> Result<()> {
        let capabilities = query_capabilities(&options.driver_id)?;
        let mut report = Report {
            mode: if options.run_seconds.is_some() {
                "silent_stream"
            } else {
                "query_only"
            },
            capabilities,
            stream: None,
            error: None,
            scope: "FL Studio ASIO software-driver acceptance only; silence out, capture discarded. Driver latency is not measured round-trip latency. Callback silence/loss counters are not physical XRUNs. Finite host loop cannot preempt hung third-party driver entry points.",
        };
        if let Some(seconds) = options.run_seconds {
            let result = (|| -> Result<StreamReport> {
                let selection = stream_options(&report.capabilities)?;
                // Query teardown has completed before opening this session;
                // AsioStream owns its apartment, lease and all native objects.
                let mut stream = AsioStream::open(selection.clone())?;
                let info = stream.info();
                ensure!(
                    info.buffer_frames == report.capabilities.buffer.preferred_frames,
                    "driver preferred buffer changed between query and stream open; refusing to substitute"
                );
                let mut started = None;
                let duration = Duration::from_secs(seconds);
                // Start the finite transfer deadline after successful native
                // start, not before an uninterruptible native open/start call.
                let mut native = NativeCounters::default();
                let mut driver_latency_frames = None;
                let mut transfer_report = None;
                let result = run_stream(
                    &mut stream,
                    selection.input,
                    selection.output,
                    || duration.saturating_sub(started.get_or_insert_with(Instant::now).elapsed()),
                    |stream, transfers| {
                        native = stream.stats().into();
                        transfer_report = transfers.cloned();
                        driver_latency_frames = stream
                            .driver_latency()
                            .map(|latency| (latency.input_frames, latency.output_frames));
                    },
                );
                let elapsed_seconds = started
                    .as_ref()
                    .map_or(0.0, |start| start.elapsed().as_secs_f64());
                let stream_report = StreamReport {
                    requested_seconds: seconds,
                    elapsed_seconds,
                    sample_rate: info.sample_rate,
                    buffer_frames: info.buffer_frames,
                    input: selection.input,
                    output: selection.output,
                    transfers: transfer_report,
                    native,
                    driver_latency_frames,
                };
                match result {
                    Ok(_) => {}
                    Err(error) => report.error = Some(error.to_string()),
                }
                Ok(stream_report)
            })();
            match result {
                Ok(stream) => report.stream = Some(stream),
                Err(error) => report.error = Some(error.to_string()),
            }
        }
        if options.json {
            let mut out = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &report)?;
            writeln!(out)?;
        } else {
            print_human(&report);
        }
        ensure!(
            report.error.is_none(),
            "ASIO probe failed: {}",
            report.error.as_deref().unwrap_or_default()
        );
        Ok(())
    }

    fn print_human(report: &Report) {
        let capabilities = &report.capabilities;
        println!(
            "ASIO {}: {} ({})",
            report.mode, capabilities.driver.name, capabilities.driver.id
        );
        println!(
            "Native driver: {} version {}",
            capabilities.driver_name, capabilities.driver_version
        );
        for (direction, channels) in [
            ("Input", &capabilities.inputs),
            ("Output", &capabilities.outputs),
        ] {
            println!("{direction} channels: {}", channels.len());
            for channel in channels {
                println!(
                    "  {}: {:?}, {:?}, active={}, group={}",
                    channel.index,
                    channel.name,
                    channel.sample_type,
                    channel.is_active,
                    channel.group
                );
            }
        }
        println!(
            "Buffers (frames): min={}, max={}, preferred={}, granularity={}",
            capabilities.buffer.min_frames,
            capabilities.buffer.max_frames,
            capabilities.buffer.preferred_frames,
            capabilities.buffer.granularity
        );
        for rate in capabilities.sample_rates {
            println!(
                "  {} Hz: {}",
                rate.sample_rate,
                if rate.supported {
                    "supported"
                } else {
                    "unsupported"
                }
            );
        }
        println!(
            "Current rate (Hz): {:?}; driver input/output latency (frames): {:?}",
            capabilities.current_sample_rate, capabilities.latencies
        );
        if let Some(stream) = &report.stream {
            println!(
                "Silent stream: {:.3}s / {}s, {} Hz, {} frames, input={}, output={}",
                stream.elapsed_seconds,
                stream.requested_seconds,
                stream.sample_rate,
                stream.buffer_frames,
                stream.input,
                stream.output
            );
            println!(
                "Transfers: {:?}; native counters: {:?}",
                stream.transfers, stream.native
            );
            println!(
                "Stream driver input/output latency (frames): {:?}",
                stream.driver_latency_frames
            );
        }
        if let Some(error) = &report.error {
            println!("Failure: {error}");
        }
        println!("{}", report.scope);
    }
}

fn main() -> Result<()> {
    let Some(options) = parse_options(std::env::args().skip(1))? else {
        println!(
            "asio_probe --driver <CLSID> [--run-seconds 1..30] [--json] [--trace]\n\
                  Default: query only; --run-seconds opts into finite silent IO and capture discard.\n\
                  Only approved FL Studio ASIO {APPROVED_DRIVER} may be activated.\n\
                  Prints metadata and counters only; never prints or stores PCM.\n\
                  --trace or MOIREN_ASIO_TRACE=1 enables owner-stage stderr logs."
        );
        return Ok(());
    };
    initialize_trace(options.trace)?;
    #[cfg(all(windows, target_pointer_width = "64"))]
    return probe::run(options);
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = options;
        anyhow::bail!("asio_probe requires 64-bit Windows");
    }
}
