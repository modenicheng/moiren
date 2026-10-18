//! Opt-in, finite ASIO acceptance. No captured PCM is printed or persisted.

use anyhow::{Context, Result, ensure};

const APPROVED_DRIVER: &str = "{188135E1-7171-3434-854F-01A3C71F3DF9}";

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

#[derive(Debug, PartialEq, Eq)]
struct Options {
    driver_id: String,
    run_seconds: Option<u64>,
    json: bool,
    trace: bool,
}

fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Option<Options>> {
    let mut args = args.into_iter();
    let mut seen = std::collections::BTreeSet::new();
    let (mut driver_id, mut run_seconds, mut json) = (None, None, false);
    let mut trace = false;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(None);
        }
        ensure!(seen.insert(arg.clone()), "duplicate argument: {arg}");
        match arg.as_str() {
            "--driver" => {
                driver_id = Some(args.next().context("--driver requires an exact CLSID")?)
            }
            "--run-seconds" => {
                let seconds = args
                    .next()
                    .context("--run-seconds requires 1..30")?
                    .parse::<u64>()
                    .context("invalid --run-seconds; expected 1..30")?;
                ensure!((1..=30).contains(&seconds), "--run-seconds must be 1..30");
                run_seconds = Some(seconds);
            }
            "--json" => json = true,
            "--trace" => trace = true,
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    let driver_id = driver_id
        .context("explicit --driver <CLSID> is required; no driver is selected automatically")?;
    let bare = driver_id
        .strip_prefix('{')
        .and_then(|id| id.strip_suffix('}'))
        .unwrap_or(&driver_id);
    ensure!(
        bare.eq_ignore_ascii_case(&APPROVED_DRIVER[1..APPROVED_DRIVER.len() - 1]),
        "only FL Studio ASIO {APPROVED_DRIVER} is approved for this probe; no fallback"
    );
    Ok(Some(Options {
        driver_id: APPROVED_DRIVER.into(),
        run_seconds,
        json,
        trace,
    }))
}

#[cfg(all(windows, target_pointer_width = "64"))]
mod probe {
    use super::*;
    use moiren_engine::host::BackendError;
    use moiren_windows_audio::professional::{
        asio::{AsioCapabilities, AsioOptions, AsioStream, query_capabilities},
        native::{NativeStream, NativeStreamStats},
    };
    use serde::Serialize;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };

    #[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
    pub(super) struct Transfers {
        wait_wakes: u64,
        wait_timeouts: u64,
        write_calls: u64,
        silent_frames_accepted: u64,
        capture_frames_discarded: u64,
        progress: ProgressDiagnostics,
    }

    #[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
    struct ProgressDiagnostics {
        windows_checked: u64,
        windows_failed: u64,
        last_callbacks: u64,
        last_capture_frames: u64,
        last_accepted_frames: u64,
        last_render_consumed_lower_bound: u64,
    }

    struct Checkpoint {
        elapsed: Duration,
        callbacks: u64,
        captured: u64,
        accepted: u64,
        rendered: u64,
    }

    #[derive(Default)]
    struct ProgressMonitor {
        initial_remaining: Option<Duration>,
        checkpoint: Option<Checkpoint>,
    }

    impl ProgressMonitor {
        fn sample(
            &mut self,
            remaining: Duration,
            stream: &impl NativeStream,
            transfers: &mut Transfers,
            input: bool,
            output: bool,
        ) {
            let total = *self.initial_remaining.get_or_insert(remaining);
            let elapsed = total.saturating_sub(remaining);
            // Ignore synchronous start and the first priming interval. Require
            // progress in EVERY following interval, not merely at the deadline.
            // Real CLI runs are >=1s; a 250ms window covers multiple periods
            // even at the largest supported buffer and lowest candidate rate.
            let interval = (total / 4).min(Duration::from_millis(250));
            if interval.is_zero() || elapsed < interval {
                return;
            }
            let stats = stream.stats();
            let now = Checkpoint {
                elapsed,
                callbacks: stats.callbacks,
                captured: transfers.capture_frames_discarded,
                accepted: transfers.silent_frames_accepted,
                // This is an upper bound while a callback is in flight: its
                // count advances before its starvation total. Compare the next
                // lower bound against THIS upper bound, never two fluctuating
                // lower bounds, to avoid mistaking a silent callback for data.
                rendered: stats
                    .callbacks
                    .saturating_mul(u64::from(stream.info().buffer_frames))
                    .saturating_sub(stats.render_silent_frames),
            };
            let Some(previous) = &self.checkpoint else {
                self.checkpoint = Some(now);
                return;
            };
            if elapsed.saturating_sub(previous.elapsed) < interval {
                return;
            }
            let progress = &mut transfers.progress;
            progress.windows_checked += 1;
            progress.last_callbacks = now.callbacks.saturating_sub(previous.callbacks);
            progress.last_capture_frames = now.captured.saturating_sub(previous.captured);
            progress.last_accepted_frames = now.accepted.saturating_sub(previous.accepted);
            // Exclude one possible in-flight block at the new checkpoint.
            // This establishes callback consumption, NOT physical playback.
            progress.last_render_consumed_lower_bound = now
                .rendered
                .saturating_sub(u64::from(stream.info().buffer_frames))
                .saturating_sub(previous.rendered);
            if progress.last_callbacks == 0
                || (input && progress.last_capture_frames == 0)
                || (output
                    && (progress.last_accepted_frames == 0
                        || progress.last_render_consumed_lower_bound == 0))
            {
                progress.windows_failed += 1;
            }
            self.checkpoint = Some(now);
        }
    }

    fn validate_progress(transfers: &Transfers) -> Result<(), BackendError> {
        let progress = &transfers.progress;
        if progress.windows_checked == 0 || progress.windows_failed != 0 {
            return Err(BackendError::Failed(format!(
                "ASIO ongoing IO progress failed: windows={} failed={} callbacks={} capture={} accepted={} native_render_consumed_lower_bound={}; totals: capture={} accepted={}; queued render acceptance alone is not native IO",
                progress.windows_checked,
                progress.windows_failed,
                progress.last_callbacks,
                progress.last_capture_frames,
                progress.last_accepted_frames,
                progress.last_render_consumed_lower_bound,
                transfers.capture_frames_discarded,
                transfers.silent_frames_accepted,
            )));
        }
        Ok(())
    }

    fn transfer_until(
        stream: &mut impl NativeStream,
        input: bool,
        output: bool,
        silence: &[[f64; 2]],
        capture: &mut [[f64; 2]],
        mut remaining: impl FnMut() -> Duration,
    ) -> Result<Transfers, BackendError> {
        let mut transfers = Transfers::default();
        let mut progress = ProgressMonitor::default();
        loop {
            let left = remaining();
            progress.sample(left, stream, &mut transfers, input, output);
            if left.is_zero() {
                return Ok(transfers);
            }
            if stream.wait(left.min(Duration::from_millis(20)))? {
                transfers.wait_wakes += 1;
            } else {
                transfers.wait_timeouts += 1;
            }
            // One bounded block per direction per iteration: a continuously
            // producing input can never keep a drain-until-empty loop alive.
            // The render source is separate immutable silence, never capture.
            if input {
                transfers.capture_frames_discarded += stream.read(capture)? as u64;
            }
            if output {
                transfers.write_calls += 1;
                transfers.silent_frames_accepted += stream.write(silence)? as u64;
            }
        }
    }

    fn run_stream(
        stream: &mut impl NativeStream,
        input: bool,
        output: bool,
        remaining: impl FnMut() -> Duration,
        observe: impl FnOnce(&dyn NativeStream, Option<&Transfers>),
    ) -> Result<Transfers, BackendError> {
        let mut result = (|| {
            let frames = stream.info().buffer_frames as usize;
            // Do not trust a native buffer preference to size an unbounded probe.
            if !(64..=4096).contains(&frames) {
                return Err(BackendError::Unsupported(format!(
                    "ASIO probe period {frames} is outside 64..=4096"
                )));
            }
            let silence = vec![[0.0; 2]; frames];
            let mut capture = vec![[0.0; 2]; frames];
            stream.start()?;
            transfer_until(stream, input, output, &silence, &mut capture, remaining)
        })();
        // Attempt BOTH cleanup operations even after start/transfer/stop errors.
        // The callback context (and its counters) is retired by release, so take
        // the owner-side observation after stop and before final teardown.
        let stop = stream.stop();
        observe(stream, result.as_ref().ok());
        if let Ok(transfers) = &result
            && let Err(error) = validate_progress(transfers)
        {
            result = Err(error);
        }
        let release = stream.release();
        for (operation, cleanup) in [("stop", stop), ("release", release)] {
            if let Err(error) = cleanup {
                result = Err(match result {
                    Ok(_) => error,
                    Err(prior) => {
                        BackendError::Failed(format!("{prior}; {operation} also failed: {error}"))
                    }
                });
            }
        }
        result
    }

    fn stream_options(capabilities: &AsioCapabilities) -> Result<AsioOptions> {
        let input = !capabilities.inputs.is_empty();
        let output = !capabilities.outputs.is_empty();
        ensure!(
            input || output,
            "ASIO driver reports no input or output channels"
        );
        ensure!(
            (64..=4096).contains(&capabilities.buffer.preferred_frames),
            "driver preferred buffer {} is outside the stream's 64..=4096 range; not substituting another size",
            capabilities.buffer.preferred_frames
        );
        let sample_rate = capabilities
            .sample_rates
            .iter()
            .find(|rate| {
                rate.supported
                    && capabilities.current_sample_rate == Some(f64::from(rate.sample_rate))
            })
            .or_else(|| capabilities.sample_rates.iter().find(|rate| rate.supported))
            .context(
                "driver supports none of 44100/48000/88200/96000 Hz; not substituting another rate",
            )?
            .sample_rate;
        Ok(AsioOptions {
            driver_id: capabilities.driver.id.clone(),
            sample_rate,
            // Zero is the native-preferred sentinel; explicitly requesting 96
            // would violate the existing exact power-of-two menu contract.
            buffer_frames: 0,
            input,
            output,
        })
    }

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

    #[cfg(test)]
    mod tests {
        use super::*;
        use moiren_engine::audio_config::SampleFormat;
        use moiren_windows_audio::professional::{
            asio::{AsioBufferCapabilities, AsioDriver, AsioRateSupport},
            native::NativeStreamInfo,
        };
        use std::sync::{Arc, Mutex};

        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
        enum ProgressMode {
            #[default]
            Healthy,
            None,
            QueuedOnly,
            StartupOnly,
            CaptureStalled,
            RenderStalled,
        }

        #[derive(Default)]
        struct State {
            events: Vec<&'static str>,
            waits: Vec<Duration>,
            reads: usize,
            writes: usize,
            fail: Option<&'static str>,
            fail_release: bool,
            progress: ProgressMode,
            callbacks: u64,
            render_silent_frames: u64,
            render_queued: usize,
            capture_available: usize,
        }

        struct FakeStream(Arc<Mutex<State>>);

        impl Drop for FakeStream {
            fn drop(&mut self) {
                self.0.lock().unwrap().events.push("drop");
            }
        }

        impl NativeStream for FakeStream {
            fn info(&self) -> NativeStreamInfo {
                NativeStreamInfo {
                    sample_rate: 48000,
                    format: SampleFormat::Float32,
                    buffer_frames: 96,
                    channels: 2,
                }
            }
            fn start(&mut self) -> Result<(), BackendError> {
                let mut state = self.0.lock().unwrap();
                state.events.push("start");
                if state.progress == ProgressMode::StartupOnly {
                    state.callbacks = 2;
                    state.capture_available = 96;
                    state.render_silent_frames = 192;
                }
                if state.fail == Some("start") {
                    Err(BackendError::Failed("start".into()))
                } else {
                    Ok(())
                }
            }
            fn stop(&mut self) -> Result<(), BackendError> {
                let mut state = self.0.lock().unwrap();
                state.events.push("stop");
                if state.fail == Some("stop") {
                    Err(BackendError::Failed("stop".into()))
                } else {
                    Ok(())
                }
            }
            fn release(&mut self) -> Result<(), BackendError> {
                let mut state = self.0.lock().unwrap();
                state.events.push("release");
                if state.fail == Some("release") || state.fail_release {
                    Err(BackendError::Failed("release".into()))
                } else {
                    Ok(())
                }
            }
            fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError> {
                let mut state = self.0.lock().unwrap();
                state.waits.push(timeout);
                if state.fail == Some("wait") {
                    Err(BackendError::Failed("wait".into()))
                } else {
                    let callbacks = matches!(
                        state.progress,
                        ProgressMode::Healthy
                            | ProgressMode::CaptureStalled
                            | ProgressMode::RenderStalled
                    ) || (state.progress == ProgressMode::StartupOnly
                        && state.waits.len() == 1);
                    if callbacks {
                        state.callbacks += 2;
                        if state.progress != ProgressMode::CaptureStalled {
                            state.capture_available = (state.capture_available + 192).min(384);
                        }
                        let consumed = if state.progress == ProgressMode::RenderStalled {
                            0
                        } else {
                            state.render_queued.min(192)
                        };
                        state.render_queued -= consumed;
                        state.render_silent_frames += (192 - consumed) as u64;
                    }
                    Ok(callbacks)
                }
            }
            fn read(&mut self, output: &mut [[f64; 2]]) -> Result<usize, BackendError> {
                let mut state = self.0.lock().unwrap();
                state.reads += 1;
                output.fill([0.75, -0.25]); // Must never be fed back to render.
                let count = output.len().min(state.capture_available);
                state.capture_available -= count;
                Ok(count)
            }
            fn write(&mut self, input: &[[f64; 2]]) -> Result<usize, BackendError> {
                let mut state = self.0.lock().unwrap();
                state.writes += 1;
                assert!(input.iter().all(|&frame| frame == [0.0; 2]));
                if state.fail == Some("write") {
                    Err(BackendError::Failed("write".into()))
                } else {
                    let accepted = if state.progress == ProgressMode::None {
                        0
                    } else {
                        (input.len() / 2).min(384 - state.render_queued)
                    };
                    state.render_queued += accepted;
                    Ok(accepted)
                }
            }
            fn stats(&self) -> NativeStreamStats {
                let state = self.0.lock().unwrap();
                NativeStreamStats {
                    callbacks: state.callbacks,
                    render_silent_frames: state.render_silent_frames,
                    ..Default::default()
                }
            }
        }

        fn acceptance_ticks() -> impl FnMut() -> Duration {
            let mut milliseconds = Some(2000u64);
            move || {
                let left = milliseconds.unwrap_or(0);
                milliseconds = left.checked_sub(100);
                Duration::from_millis(left)
            }
        }

        fn assert_rejected_progress(mode: ProgressMode) {
            let state = Arc::new(Mutex::new(State {
                progress: mode,
                ..State::default()
            }));
            let result = {
                let mut stream = FakeStream(Arc::clone(&state));
                run_stream(
                    &mut stream,
                    true,
                    true,
                    acceptance_ticks(),
                    |_, transfers| {
                        let transfers =
                            transfers.expect("progress failures must preserve raw counters");
                        assert!(transfers.wait_wakes + transfers.wait_timeouts > 0);
                    },
                )
            };
            let error = result
                .expect_err(&format!("{mode:?} must not pass silent-stream acceptance"))
                .to_string();
            assert!(error.contains("progress"), "{mode:?}: {error}");
            assert!(error.contains("callbacks="), "{mode:?}: {error}");
            assert!(error.contains("accepted="), "{mode:?}: {error}");
            assert!(error.contains("capture="), "{mode:?}: {error}");
            assert_eq!(
                state.lock().unwrap().events,
                ["start", "stop", "release", "drop"]
            );
        }

        #[test]
        fn acceptance_rejects_no_callbacks_or_transfer_progress() {
            assert_rejected_progress(ProgressMode::None);
        }

        #[test]
        fn acceptance_rejects_queued_render_without_callbacks() {
            assert_rejected_progress(ProgressMode::QueuedOnly);
        }

        #[test]
        fn acceptance_rejects_startup_only_progress() {
            assert_rejected_progress(ProgressMode::StartupOnly);
        }

        #[test]
        fn acceptance_rejects_missing_capture_progress_with_healthy_callbacks() {
            assert_rejected_progress(ProgressMode::CaptureStalled);
        }

        #[test]
        fn acceptance_rejects_missing_native_render_consumption() {
            assert_rejected_progress(ProgressMode::RenderStalled);
        }

        #[test]
        fn acceptance_healthy_callbacks_and_enabled_directions_keep_progressing() {
            for (input, output) in [(true, true), (true, false), (false, true)] {
                let state = Arc::new(Mutex::new(State::default()));
                let result = {
                    let mut stream = FakeStream(Arc::clone(&state));
                    run_stream(&mut stream, input, output, acceptance_ticks(), |_, _| {})
                }
                .unwrap();
                assert_eq!(result.capture_frames_discarded > 0, input);
                assert_eq!(result.silent_frames_accepted > 0, output);
                assert_eq!(
                    state.lock().unwrap().events,
                    ["start", "stop", "release", "drop"]
                );
            }
        }

        fn ticks() -> impl FnMut() -> Duration {
            let mut ticks = [
                Duration::from_millis(30),
                Duration::from_millis(3),
                Duration::ZERO,
            ]
            .into_iter();
            move || ticks.next().unwrap_or(Duration::ZERO)
        }

        #[test]
        fn finite_transfer_caps_waits_and_discards_capture_without_loopback() {
            let state = Arc::new(Mutex::new(State::default()));
            let mut stream = FakeStream(Arc::clone(&state));
            let mut capture = [[0.0; 2]; 96];
            let result = transfer_until(
                &mut stream,
                true,
                true,
                &[[0.0; 2]; 96],
                &mut capture,
                ticks(),
            )
            .unwrap();
            assert_eq!(
                result,
                Transfers {
                    wait_wakes: 2,
                    wait_timeouts: 0,
                    write_calls: 2,
                    silent_frames_accepted: 96,
                    capture_frames_discarded: 192,
                    progress: ProgressDiagnostics::default(),
                }
            );
            let state = state.lock().unwrap();
            assert_eq!(
                state.waits,
                [Duration::from_millis(20), Duration::from_millis(3)]
            );
            assert_eq!((state.reads, state.writes), (2, 2));
        }

        #[test]
        fn lifecycle_always_stops_releases_and_drops_even_after_any_failure() {
            for fail in [
                None,
                Some("start"),
                Some("wait"),
                Some("write"),
                Some("stop"),
                Some("release"),
            ] {
                let state = Arc::new(Mutex::new(State {
                    fail,
                    ..State::default()
                }));
                let result = {
                    let mut stream = FakeStream(Arc::clone(&state));
                    run_stream(&mut stream, true, true, acceptance_ticks(), |_, _| {})
                };
                assert_eq!(result.is_err(), fail.is_some(), "{fail:?}");
                assert_eq!(
                    state.lock().unwrap().events,
                    ["start", "stop", "release", "drop"]
                );
            }
        }

        #[test]
        fn final_release_error_is_not_hidden_by_start_or_transfer_failure() {
            for fail in ["start", "wait", "write", "stop"] {
                let state = Arc::new(Mutex::new(State {
                    fail: Some(fail),
                    fail_release: true,
                    ..State::default()
                }));
                let error = run_stream(
                    &mut FakeStream(Arc::clone(&state)),
                    true,
                    true,
                    acceptance_ticks(),
                    |_, _| {},
                )
                .unwrap_err()
                .to_string();
                assert!(error.contains(fail), "{error}");
                assert!(error.contains("release"), "{error}");
                assert_eq!(
                    state.lock().unwrap().events,
                    ["start", "stop", "release", "drop"]
                );
            }
        }

        #[test]
        fn zero_deadline_never_transfers_and_disabled_directions_are_not_touched() {
            let state = Arc::new(Mutex::new(State::default()));
            let mut stream = FakeStream(Arc::clone(&state));
            let mut capture = [[0.0; 2]; 96];
            assert_eq!(
                transfer_until(
                    &mut stream,
                    true,
                    true,
                    &[[0.0; 2]; 96],
                    &mut capture,
                    || Duration::ZERO
                )
                .unwrap(),
                Transfers::default()
            );
            assert!(state.lock().unwrap().waits.is_empty());
            let result = transfer_until(
                &mut stream,
                false,
                false,
                &[[0.0; 2]; 96],
                &mut capture,
                ticks(),
            )
            .unwrap();
            assert_eq!(result.wait_wakes, 2);
            let state = state.lock().unwrap();
            assert_eq!((state.reads, state.writes), (0, 0));
        }

        #[test]
        fn stream_selection_uses_supported_rate_and_native_preferred_size() {
            let mut capabilities = AsioCapabilities {
                driver: AsioDriver {
                    id: APPROVED_DRIVER.into(),
                    name: "Fake".into(),
                },
                driver_name: "Fake".into(),
                driver_version: 1,
                inputs: Vec::new(),
                outputs: Vec::new(),
                buffer: AsioBufferCapabilities {
                    min_frames: 64,
                    max_frames: 4096,
                    preferred_frames: 96,
                    granularity: 32,
                },
                sample_rates: [44100, 48000, 88200, 96000].map(|sample_rate| AsioRateSupport {
                    sample_rate,
                    supported: sample_rate == 48000,
                }),
                current_sample_rate: Some(44100.0),
                latencies: None,
            };
            // No channels cannot be presented as a successful silent stream.
            assert!(stream_options(&capabilities).is_err());
            capabilities
                .outputs
                .push(moiren_windows_audio::professional::asio::AsioChannel {
                    index: 0,
                    name: "Main".into(),
                    sample_type:
                        moiren_windows_audio::professional::asio::AsioSampleType::Float32Lsb,
                    is_active: false,
                    group: 0,
                });
            let options = stream_options(&capabilities).unwrap();
            assert_eq!(options.driver_id, APPROVED_DRIVER);
            assert_eq!(options.sample_rate, 48000);
            assert_eq!(
                options.buffer_frames, 0,
                "zero requests the driver's native preferred size, including 96"
            );
            assert!(!options.input && options.output);
            capabilities
                .sample_rates
                .iter_mut()
                .for_each(|rate| rate.supported = false);
            assert!(stream_options(&capabilities).is_err());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn explicit_driver_is_required_and_query_only_is_the_default() {
        assert!(parse_options(args(&[])).is_err());
        assert_eq!(
            parse_options(args(&["--driver", APPROVED_DRIVER])).unwrap(),
            Some(Options {
                driver_id: APPROVED_DRIVER.into(),
                run_seconds: None,
                json: false,
                trace: false,
            })
        );
        assert_eq!(parse_options(args(&["--help"])).unwrap(), None);
    }

    #[test]
    fn trace_is_an_explicit_query_only_opt_in_and_rejects_duplicates() {
        let result = parse_options(args(&["--driver", APPROVED_DRIVER, "--trace", "--json"]));
        assert!(
            result.is_ok(),
            "trace must be accepted without starting IO: {result:?}"
        );
        let options = result.unwrap().unwrap();
        assert_eq!(options.run_seconds, None);
        assert!(options.trace);
        assert!(parse_options(args(&["--driver", APPROVED_DRIVER, "--trace", "--trace"])).is_err());
    }

    #[test]
    fn only_the_exact_approved_driver_and_finite_run_are_accepted() {
        let options = parse_options(args(&[
            "--driver",
            "188135e1-7171-3434-854f-01a3c71f3df9",
            "--run-seconds",
            "2",
            "--json",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            options,
            Options {
                driver_id: APPROVED_DRIVER.into(),
                run_seconds: Some(2),
                json: true,
                trace: false,
            }
        );
        for values in [
            vec!["--driver", "FL Studio ASIO"],
            vec!["--driver", "{00000000-0000-0000-0000-000000000000}"],
            vec!["--driver", APPROVED_DRIVER, "--run-seconds", "0"],
            vec!["--driver", APPROVED_DRIVER, "--run-seconds", "31"],
            vec!["--driver", APPROVED_DRIVER, "--run-seconds", "NaN"],
            vec!["--driver", APPROVED_DRIVER, "--driver", APPROVED_DRIVER],
            vec!["--driver", APPROVED_DRIVER, "--json", "--json"],
            vec![
                "--driver",
                APPROVED_DRIVER,
                "--run-seconds",
                "1",
                "--run-seconds",
                "2",
            ],
            vec!["--driver", APPROVED_DRIVER, "--unknown"],
            vec!["--driver"],
        ] {
            assert!(parse_options(args(&values)).is_err(), "{values:?}");
        }
    }
}
