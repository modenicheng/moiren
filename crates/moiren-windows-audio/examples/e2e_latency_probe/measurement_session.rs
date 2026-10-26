//! Control-side independent measurement ownership, not source supervision.

use crate::measurement_sink::{EvidenceSink, SinkReport};
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::capture::{CaptureReport, PcmEvidenceFrame};
use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Duration, Instant},
};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FAILURE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct MeasurementConfig {
    pub(crate) input_endpoint_id: String,
    pub(crate) max_active_duration: Duration,
    pub(crate) ring_frames: usize,
    pub(crate) max_pcm_bytes: u64,
    pub(crate) max_metadata_bytes: u64,
    pub(crate) max_packets: u64,
}

pub(crate) struct MeasurementPaths {
    pub(crate) pcm: PathBuf,
    pub(crate) packets: PathBuf,
}

impl MeasurementPaths {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.pcm.as_os_str().is_empty()
                && !self.packets.as_os_str().is_empty()
                && self.pcm != self.packets,
            "measurement output paths must be nonempty and distinct"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MeasurementInfo {
    pub(crate) input_endpoint_id: String,
    pub(crate) sample_rate: u32,
    pub(crate) channels: usize,
    pub(crate) ring_frames: usize,
    pub(crate) max_frames: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MeasurementProgress {
    Prepared,
    Starting,
    Running { elapsed: Duration },
    DeadlineReached,
    Stopping,
}

#[derive(Debug)]
pub(crate) struct MeasurementOutcome {
    pub(crate) info: MeasurementInfo,
    pub(crate) native_capture: Option<CaptureReport>,
    pub(crate) sink: Option<SinkReport>,
    pub(crate) activation_elapsed: Option<Duration>,
    pub(crate) failure: Option<String>,
}

trait NativeOwner: Send {
    fn activate(&self) -> Result<()>;
    fn has_started(&self) -> bool;
    fn is_finished(&self) -> bool;
    fn request_stop(&self) -> Result<()>;
    fn join(&mut self) -> Result<CaptureReport>;
}

trait SinkOwner: Send {
    fn check(&self) -> Result<()>;
    fn cancel(&self);
    fn finish(&mut self) -> Result<SinkReport>;
}

pub(crate) struct MeasurementSession {
    native: Option<Box<dyn NativeOwner>>,
    sink: Option<Box<dyn SinkOwner>>,
    info: MeasurementInfo,
    config: MeasurementConfig,
    activated: Option<Instant>,
    stopping: bool,
    activation_failed: bool,
    failures: RefCell<Option<String>>,
}

impl MeasurementConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            !self.input_endpoint_id.trim().is_empty() && !self.input_endpoint_id.contains('\0'),
            "--input must be an explicit endpoint ID without NUL"
        );
        ensure!(
            self.max_active_duration.subsec_nanos() == 0
                && (1..=7200).contains(&self.max_active_duration.as_secs()),
            "measurement duration must be 1..7200 whole seconds"
        );
        ensure!(
            self.ring_frames > 0
                && self
                    .ring_frames
                    .checked_mul(size_of::<PcmEvidenceFrame>())
                    .is_some_and(|bytes| bytes <= 8 * 1024 * 1024),
            "evidence ring exceeds its nonzero 8 MiB storage budget"
        );
        ensure!(
            (1..=3 * 1024 * 1024 * 1024).contains(&self.max_pcm_bytes),
            "PCM byte cap must be 1..3GiB"
        );
        ensure!(
            (1..=256 * 1024 * 1024).contains(&self.max_metadata_bytes),
            "metadata byte cap must be 1..256MiB"
        );
        ensure!(
            (1..=1_000_000).contains(&self.max_packets),
            "packet cap must be 1..1000000"
        );
        Ok(())
    }

    fn negotiated_info(&self, sample_rate: u32, channels: usize) -> Result<MeasurementInfo> {
        self.validate()?;
        ensure!(
            sample_rate > 0 && (1..=2).contains(&channels),
            "measurement requires negotiated nonzero rate and mono/stereo"
        );
        let width = (channels as u64)
            .checked_mul(4)
            .context("measurement frame width overflow")?;
        let max_frames = self.max_pcm_bytes / width;
        ensure!(
            max_frames > 0,
            "PCM byte cap cannot hold one negotiated frame"
        );
        Ok(MeasurementInfo {
            input_endpoint_id: self.input_endpoint_id.clone(),
            sample_rate,
            channels,
            ring_frames: self.ring_frames,
            max_frames,
        })
    }
}

impl MeasurementSession {
    #[cfg(windows)]
    pub(crate) fn prepare(config: MeasurementConfig, paths: MeasurementPaths) -> Result<Self> {
        use moiren_windows_audio::{
            ActivationGate, SessionDuration, StopSignal,
            capture::{CaptureOptions, PcmEvidenceConfig, prepare_capture_native_with_gate},
            latency::{PreparedInputQueue, TelemetryMode},
        };
        config.validate()?;
        paths.validate()?;
        let stop = StopSignal::new().context("create independent measurement stop")?;
        let gate =
            ActivationGate::new(stop.clone()).context("create measurement activation gate")?;
        let native = prepare_capture_native_with_gate(
            CaptureOptions {
                artifact: None,
                latency_telemetry: TelemetryMode::Summary,
                shared_period: Default::default(),
                endpoint_id: config.input_endpoint_id.clone(),
                duration: SessionDuration::UntilStopped,
            },
            stop.clone(),
            gate.clone(),
        )
        .context("prepare independent native capture")?;
        let negotiated = native.info();
        let info = config.negotiated_info(negotiated.timing.sample_rate, negotiated.channels)?;
        let (capture, reader) = native
            .attach_with_evidence(
                PreparedInputQueue::compatible(),
                PcmEvidenceConfig {
                    capacity_frames: config.ring_frames,
                    detect_position_gaps: true,
                },
            )
            .context("attach bounded measurement tap")?;
        // Install native ownership before any fallible sink setup. Drop must
        // stop/join the gated native owner before waiting on a filesystem worker.
        let mut owner = Self::own_native(
            config,
            info,
            NativeCapture {
                capture: Some(capture),
                gate,
                stop,
            },
        );
        let sink = EvidenceSink::create(
            reader,
            owner.info.sample_rate,
            owner.info.channels,
            &paths.pcm,
            &paths.packets,
            crate::measurement_sink::SinkLimits {
                max_frames: owner.info.max_frames,
                max_pcm_bytes: owner.config.max_pcm_bytes,
                max_metadata_bytes: owner.config.max_metadata_bytes,
                max_packets: owner.config.max_packets,
            },
        )
        .context("prepare bounded non-RT measurement sink")?;
        owner.sink = Some(Box::new(sink));
        Ok(owner)
    }

    fn own_native(
        config: MeasurementConfig,
        info: MeasurementInfo,
        native: impl NativeOwner + 'static,
    ) -> Self {
        Self {
            native: Some(Box::new(native)),
            sink: None,
            config,
            info,
            activated: None,
            stopping: false,
            activation_failed: false,
            failures: RefCell::new(None),
        }
    }

    pub(crate) fn info(&self) -> &MeasurementInfo {
        &self.info
    }

    pub(crate) fn activate(&mut self) -> Result<()> {
        ensure!(
            self.activated.is_none() && !self.stopping,
            "measurement activation already attempted or stopping"
        );
        self.check()?;
        self.activated = Some(Instant::now());
        let result = self
            .native
            .as_ref()
            .context("measurement native owner missing")?
            .activate();
        if let Err(error) = &result {
            self.activation_failed = true;
            append_failure(
                &mut self.failures.borrow_mut(),
                &format!("activate independent measurement: {error:#}"),
            );
        }
        result
    }

    pub(crate) fn check(&self) -> Result<MeasurementProgress> {
        self.check_elapsed(self.activated.map(|instant| instant.elapsed()))
    }

    fn check_elapsed(&self, elapsed: Option<Duration>) -> Result<MeasurementProgress> {
        let result = (|| {
            let native = self
                .native
                .as_ref()
                .context("measurement native owner missing or finished")?;
            let sink = self
                .sink
                .as_ref()
                .context("measurement sink owner missing or finished")?;
            if self.stopping {
                return Ok(MeasurementProgress::Stopping);
            }
            ensure!(!self.activation_failed, "measurement activation failed");
            ensure!(
                !native.is_finished(),
                "measurement owner stopped before deadline"
            );
            sink.check()?;
            let Some(elapsed) = elapsed else {
                return Ok(MeasurementProgress::Prepared);
            };
            if !native.has_started() {
                ensure!(
                    elapsed < START_TIMEOUT,
                    "measurement native Start acknowledgement exceeded 5 seconds"
                );
                ensure!(
                    elapsed < self.config.max_active_duration,
                    "measurement deadline reached without native Start"
                );
                return Ok(MeasurementProgress::Starting);
            }
            if elapsed >= self.config.max_active_duration {
                return Ok(MeasurementProgress::DeadlineReached);
            }
            Ok(MeasurementProgress::Running { elapsed })
        })();
        if let Err(error) = &result {
            append_failure(
                &mut self.failures.borrow_mut(),
                &format!("measurement health: {error:#}"),
            );
        }
        result
    }

    pub(crate) fn request_stop(&mut self) -> Result<()> {
        let native = self
            .native
            .as_ref()
            .context("measurement native owner missing or finished")?;
        self.stopping = true;
        let result = native.request_stop();
        if let Err(error) = &result {
            append_failure(
                &mut self.failures.borrow_mut(),
                &format!("stop independent measurement: {error:#}"),
            );
        }
        result
    }

    pub(crate) fn finish(&mut self) -> Result<MeasurementOutcome> {
        ensure!(
            self.native.is_some() && self.sink.is_some(),
            "measurement owner missing or already finished"
        );
        // No fallible result below may bypass either join. Native EOF must be
        // final before the sink drains, even when native quality already failed.
        let _ = self.request_stop();
        let mut native = self.native.take().expect("owner checked above");
        let native_result = join_native(native.as_mut());
        drop(native);
        let mut sink = self.sink.take().expect("owner checked above");
        let sink_result = join_sink(sink.as_mut());
        drop(sink);
        let mut outcome = MeasurementOutcome {
            info: self.info.clone(),
            native_capture: None,
            sink: None,
            activation_elapsed: self.activated.map(|instant| instant.elapsed()),
            failure: self.failures.take(),
        };
        match native_result {
            Ok(report) => outcome.native_capture = Some(report),
            Err(error) => append_failure(
                &mut outcome.failure,
                &format!("join measurement owner: {error:#}"),
            ),
        }
        match sink_result {
            Ok(report) => outcome.sink = Some(report),
            Err(error) => append_failure(
                &mut outcome.failure,
                &format!("join measurement sink: {error:#}"),
            ),
        }
        validate_outcome(&mut outcome);
        Ok(outcome)
    }
}

impl Drop for MeasurementSession {
    fn drop(&mut self) {
        if let Some(mut native) = self.native.take() {
            let _ = native.request_stop();
            let _ = join_native(native.as_mut());
            drop(native);
        }
        if let Some(mut sink) = self.sink.take() {
            sink.cancel();
            let _ = join_sink(sink.as_mut());
        }
    }
}

fn join_native(owner: &mut dyn NativeOwner) -> Result<CaptureReport> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.join()))
        .map_err(|_| anyhow::anyhow!("measurement native join panicked"))?
}

fn join_sink(owner: &mut dyn SinkOwner) -> Result<SinkReport> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.finish()))
        .map_err(|_| anyhow::anyhow!("measurement sink join panicked"))?
}

impl SinkOwner for EvidenceSink {
    fn check(&self) -> Result<()> {
        EvidenceSink::check(self)
    }
    fn cancel(&self) {
        EvidenceSink::cancel(self);
    }
    fn finish(&mut self) -> Result<SinkReport> {
        EvidenceSink::finish(self)
    }
}

#[cfg(windows)]
struct NativeCapture {
    capture: Option<moiren_windows_audio::capture::PreparedCapture>,
    gate: moiren_windows_audio::ActivationGate,
    stop: moiren_windows_audio::StopSignal,
}

#[cfg(windows)]
impl NativeOwner for NativeCapture {
    fn activate(&self) -> Result<()> {
        self.gate
            .activate()
            .context("activate independent measurement")
    }
    fn has_started(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|capture| capture.session.has_started())
    }
    fn is_finished(&self) -> bool {
        self.capture
            .as_ref()
            .is_none_or(|capture| capture.session.is_finished())
    }
    fn request_stop(&self) -> Result<()> {
        self.stop
            .request_stop()
            .context("stop independent measurement owner")
    }
    fn join(&mut self) -> Result<CaptureReport> {
        let capture = self
            .capture
            .take()
            .context("measurement native owner missing")?;
        let result = capture.session.join().context("join measurement owner");
        // These are the unused monitoring bridge, not the waveform tap. Keep
        // their allocations alive through the native join and reclaim off RT.
        drop((capture.source, capture.observer));
        result
    }
}

fn validate_outcome(outcome: &mut MeasurementOutcome) {
    if outcome.activation_elapsed.is_none() {
        append_failure(&mut outcome.failure, "measurement was never activated");
    }
    if let Some(native) = &outcome.native_capture {
        reconcile(
            native.stream_started,
            native.stop_succeeded,
            native.failure.as_deref(),
            native.evidence_metadata,
            outcome.sink.as_ref(),
            &outcome.info,
            &mut outcome.failure,
        );
    } else if let Some(sink) = &outcome.sink {
        validate_sink(sink, None, &outcome.info, &mut outcome.failure);
    }
}

fn reconcile(
    started: bool,
    stopped: bool,
    native_failure: Option<&str>,
    summary: Option<moiren_windows_audio::capture::PcmEvidenceSummary>,
    sink: Option<&SinkReport>,
    info: &MeasurementInfo,
    failure: &mut Option<String>,
) {
    if let Some(native_failure) = native_failure {
        append_failure(failure, native_failure);
    }
    if !started || !stopped {
        append_failure(
            failure,
            "measurement native stream did not start and stop cleanly",
        );
    }
    match summary {
        Some(summary) => {
            if summary.dropped_frames != 0
                || summary.overflow_packets != 0
                || summary.reader_abandoned_packets != 0
                || summary.gap_events != 0
                || summary.discontinuity_events != 0
                || summary.timestamp_error_packets != 0
                || summary.position_overflow_packets != 0
                || summary.counters_saturated
                || summary.sequence_exhausted
            {
                append_failure(
                    failure,
                    "measurement evidence has loss or invalid continuity/timestamps",
                );
            }
        }
        None => append_failure(failure, "measurement evidence summary is missing"),
    }
    if let Some(sink) = sink {
        validate_sink(sink, summary, info, failure);
    }
}

fn validate_sink(
    sink: &SinkReport,
    summary: Option<moiren_windows_audio::capture::PcmEvidenceSummary>,
    info: &MeasurementInfo,
    failure: &mut Option<String>,
) {
    if let Some(sink_failure) = &sink.failure {
        append_failure(failure, sink_failure);
    }
    if sink.nonfinite_samples != 0
        || sink.missing_packets != 0
        || sink.incomplete_packets != 0
        || sink.counters_saturated
        || sink.cancelled
        || sink.sample_rate != info.sample_rate
        || sink.channels != info.channels
        || summary.is_some_and(|summary| {
            summary.stored_frames == 0
                || sink.written_frames != summary.stored_frames
                || sink.complete_packets != summary.packets
        })
    {
        append_failure(
            failure,
            "measurement sink and final native evidence are not complete and consistent",
        );
    }
}

fn append_failure(failure: &mut Option<String>, message: &str) {
    let text = failure.get_or_insert_with(String::new);
    if text.contains(message) || text.len() >= MAX_FAILURE_BYTES {
        return;
    }
    let separator = if text.is_empty() { "" } else { "; " };
    let available = MAX_FAILURE_BYTES.saturating_sub(text.len() + separator.len());
    let mut end = message.len().min(available);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return;
    }
    text.push_str(separator);
    text.push_str(&message[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct State {
        started: AtomicBool,
        native_finished: AtomicBool,
        sink_finished: AtomicBool,
        activation_failure: AtomicBool,
        stop_failure: AtomicBool,
        native_join_failure: AtomicBool,
        sink_join_failure: AtomicBool,
        log: Mutex<Vec<&'static str>>,
    }

    struct FakeNative {
        state: Arc<State>,
        report: Option<CaptureReport>,
    }

    impl NativeOwner for FakeNative {
        fn activate(&self) -> Result<()> {
            self.state.log.lock().unwrap().push("activate");
            anyhow::ensure!(
                !self.state.activation_failure.load(Ordering::Relaxed),
                "injected activation failure"
            );
            Ok(())
        }
        fn has_started(&self) -> bool {
            self.state.started.load(Ordering::Relaxed)
        }
        fn is_finished(&self) -> bool {
            self.state.native_finished.load(Ordering::Relaxed)
        }
        fn request_stop(&self) -> Result<()> {
            self.state.log.lock().unwrap().push("native_stop");
            anyhow::ensure!(
                !self.state.stop_failure.load(Ordering::Relaxed),
                "injected stop failure"
            );
            Ok(())
        }
        fn join(&mut self) -> Result<CaptureReport> {
            self.state.log.lock().unwrap().push("native_join");
            anyhow::ensure!(
                !self.state.native_join_failure.load(Ordering::Relaxed),
                "injected native worker panicked"
            );
            self.report
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing fake report"))
        }
    }

    struct FakeSink(Arc<State>);

    impl SinkOwner for FakeSink {
        fn check(&self) -> Result<()> {
            anyhow::ensure!(
                !self.0.sink_finished.load(Ordering::Relaxed),
                "injected sink stopped"
            );
            Ok(())
        }
        fn cancel(&self) {
            self.0.log.lock().unwrap().push("sink_cancel");
        }
        fn finish(&mut self) -> Result<SinkReport> {
            self.0.log.lock().unwrap().push("sink_join");
            anyhow::ensure!(
                !self.0.sink_join_failure.load(Ordering::Relaxed),
                "injected sink worker panicked"
            );
            Ok(sink_report())
        }
    }

    fn config() -> MeasurementConfig {
        MeasurementConfig {
            input_endpoint_id: "independent-endpoint".into(),
            max_active_duration: Duration::from_secs(10),
            ring_frames: 8,
            max_pcm_bytes: 1024,
            max_metadata_bytes: 4096,
            max_packets: 10,
        }
    }

    fn info() -> MeasurementInfo {
        MeasurementInfo {
            input_endpoint_id: "independent-endpoint".into(),
            sample_rate: 44_100,
            channels: 1,
            ring_frames: 8,
            max_frames: 256,
        }
    }

    fn sink_report() -> SinkReport {
        SinkReport {
            schema_version: 1,
            sample_rate: 44_100,
            channels: 1,
            written_frames: 2,
            written_pcm_bytes: 8,
            written_metadata_bytes: 180,
            complete_packets: 1,
            sequence_gaps: 0,
            missing_packets: 0,
            incomplete_packets: 0,
            nonfinite_samples: 0,
            counters_saturated: false,
            cancelled: false,
            failure: None,
        }
    }

    fn session() -> (MeasurementSession, Arc<State>) {
        let state = Arc::new(State::default());
        let mut session = MeasurementSession::own_native(
            config(),
            info(),
            FakeNative {
                state: state.clone(),
                report: None,
            },
        );
        session.sink = Some(Box::new(FakeSink(state.clone())));
        (session, state)
    }

    #[test]
    fn preparation_activation_and_native_start_are_distinct() {
        let (mut owner, state) = session();
        assert_eq!(owner.info().sample_rate, 44_100);
        assert_eq!(owner.info().channels, 1);
        assert_eq!(owner.check().unwrap(), MeasurementProgress::Prepared);
        assert!(state.log.lock().unwrap().is_empty());
        owner.activate().unwrap();
        assert_eq!(
            owner.check_elapsed(Some(Duration::ZERO)).unwrap(),
            MeasurementProgress::Starting
        );
        state.started.store(true, Ordering::Relaxed);
        assert_eq!(
            owner.check_elapsed(Some(Duration::from_secs(1))).unwrap(),
            MeasurementProgress::Running {
                elapsed: Duration::from_secs(1)
            }
        );
        assert!(owner.activate().is_err());
        assert_eq!(
            state
                .log
                .lock()
                .unwrap()
                .iter()
                .filter(|&&event| event == "activate")
                .count(),
            1
        );
    }

    #[test]
    fn failed_activation_cannot_retry_and_is_retained_after_cleanup() {
        let (mut owner, state) = session();
        state.activation_failure.store(true, Ordering::Relaxed);
        assert!(owner.activate().is_err());
        assert!(owner.activate().is_err());
        assert!(owner.check().is_err());
        let outcome = owner.finish().unwrap();
        assert!(
            outcome
                .failure
                .unwrap()
                .contains("injected activation failure")
        );
        assert!(owner.finish().is_err());
        assert_eq!(
            &*state.log.lock().unwrap(),
            &["activate", "native_stop", "native_join", "sink_join"]
        );
    }

    #[test]
    fn health_is_checked_before_deadline_and_start_timeout_fails_closed() {
        for native_exit in [true, false] {
            let (mut owner, state) = session();
            owner.activate().unwrap();
            if native_exit {
                state.native_finished.store(true, Ordering::Relaxed);
            } else {
                state.sink_finished.store(true, Ordering::Relaxed);
            }
            assert!(owner.check_elapsed(Some(Duration::from_secs(10))).is_err());
        }
        let (mut owner, state) = session();
        owner.activate().unwrap();
        assert_eq!(
            owner
                .check_elapsed(Some(Duration::from_millis(4999)))
                .unwrap(),
            MeasurementProgress::Starting
        );
        assert!(
            owner
                .check_elapsed(Some(Duration::from_secs(5)))
                .unwrap_err()
                .to_string()
                .contains("Start")
        );
        state.started.store(true, Ordering::Relaxed);
        assert_eq!(
            owner.check_elapsed(Some(Duration::from_secs(10))).unwrap(),
            MeasurementProgress::DeadlineReached
        );
    }

    #[test]
    fn stop_before_activation_is_idempotent_and_does_not_join() {
        let (mut owner, state) = session();
        owner.request_stop().unwrap();
        owner.request_stop().unwrap();
        assert_eq!(owner.check().unwrap(), MeasurementProgress::Stopping);
        assert!(owner.activate().is_err());
        assert!(
            state
                .log
                .lock()
                .unwrap()
                .iter()
                .all(|&event| event == "native_stop")
        );
        let outcome = owner.finish().unwrap();
        assert!(outcome.activation_elapsed.is_none());
        assert!(outcome.failure.is_some());
        assert!(owner.check().is_err());
    }

    #[test]
    fn cleanup_preserves_independent_failures_and_never_cancels_explicit_drain() {
        let (mut owner, state) = session();
        owner.activate().unwrap();
        state.stop_failure.store(true, Ordering::Relaxed);
        state.native_join_failure.store(true, Ordering::Relaxed);
        state.sink_join_failure.store(true, Ordering::Relaxed);
        assert!(owner.request_stop().is_err());
        let outcome = owner.finish().unwrap();
        let failure = outcome.failure.unwrap();
        for expected in [
            "stop failure",
            "native worker panicked",
            "sink worker panicked",
        ] {
            assert!(failure.contains(expected), "{failure}");
        }
        assert!(outcome.native_capture.is_none() && outcome.sink.is_none());
        let log = state.log.lock().unwrap();
        assert_eq!(
            &log[log.len() - 3..],
            &["native_stop", "native_join", "sink_join"]
        );
        assert!(!log.contains(&"sink_cancel"));
    }

    #[test]
    fn unwind_drop_joins_native_before_sink_cancellation() {
        let (owner, state) = session();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owner = owner;
            panic!("injected controller unwind");
        }));
        assert_eq!(
            &*state.log.lock().unwrap(),
            &["native_stop", "native_join", "sink_cancel", "sink_join"]
        );
    }

    #[test]
    fn config_rechecks_caps_and_negotiated_format_without_nominal_identity() {
        let original = config();
        original.validate().unwrap();
        for index in 0..13 {
            let mut invalid = original.clone();
            match index {
                0 => invalid.input_endpoint_id = " ".into(),
                1 => invalid.input_endpoint_id.push('\0'),
                2 => invalid.max_active_duration = Duration::ZERO,
                3 => invalid.max_active_duration = Duration::from_secs(7201),
                4 => invalid.max_active_duration = Duration::from_millis(1500),
                5 => invalid.ring_frames = usize::MAX,
                6 => invalid.ring_frames = 0,
                7 => invalid.max_pcm_bytes = 0,
                8 => invalid.max_pcm_bytes = 3 * 1024 * 1024 * 1024 + 1,
                9 => invalid.max_metadata_bytes = 256 * 1024 * 1024 + 1,
                10 => invalid.max_packets = 1_000_001,
                11 => invalid.max_metadata_bytes = 0,
                _ => invalid.max_packets = 0,
            }
            assert!(
                invalid.validate().is_err(),
                "admitted invalid variant {index}"
            );
        }
        for (rate, channels) in [(0, 1), (48_000, 0), (48_000, 3)] {
            assert!(original.negotiated_info(rate, channels).is_err());
        }
        let negotiated = original.negotiated_info(96_000, 2).unwrap();
        assert_eq!(negotiated.sample_rate, 96_000);
        assert_eq!(negotiated.channels, 2);
        assert_eq!(negotiated.max_frames, 128);
        assert_eq!(negotiated.ring_frames, 8);
        assert_eq!(negotiated.input_endpoint_id, original.input_endpoint_id);
        let mut tiny = original;
        tiny.max_pcm_bytes = 3;
        assert!(tiny.negotiated_info(44_100, 1).is_err());
        for (pcm, packets) in [("", "packet"), ("pcm", ""), ("same", "same")] {
            assert!(
                MeasurementPaths {
                    pcm: pcm.into(),
                    packets: packets.into()
                }
                .validate()
                .is_err()
            );
        }
        MeasurementPaths {
            pcm: "pcm".into(),
            packets: "packets".into(),
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn prepared_health_already_detects_dead_native_or_sink_without_activation() {
        for native in [true, false] {
            let (owner, state) = session();
            if native {
                state.native_finished.store(true, Ordering::Relaxed);
            } else {
                state.sink_finished.store(true, Ordering::Relaxed);
            }
            assert!(
                owner.check().is_err(),
                "prepared health cannot hide a dead peer"
            );
            assert!(
                state.log.lock().unwrap().is_empty(),
                "health must not activate/stop/join peers"
            );
        }
    }

    #[test]
    fn native_outcome_reconciliation_rejects_missing_loss_and_inconsistent_summaries() {
        use moiren_windows_audio::capture::{
            PcmEvidenceConfig, PcmEvidencePacket, pcm_evidence_stream,
        };
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 2,
                detect_position_gaps: true,
            },
            44_100,
            1,
        )
        .unwrap();
        writer
            .record_packet(
                &[0; 8],
                PcmEvidencePacket {
                    frames: 2,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 42,
                },
            )
            .unwrap();
        writer.close();
        assert!(reader.pop().is_some() && reader.pop().is_some());
        let summary = writer.snapshot();
        let mut failure = None;
        reconcile(
            true,
            true,
            None,
            Some(summary),
            Some(&sink_report()),
            &info(),
            &mut failure,
        );
        assert!(failure.is_none(), "{failure:?}");
        for variant in 0..17 {
            let mut summary = summary;
            let mut sink = sink_report();
            match variant {
                0 => summary.dropped_frames = 1,
                1 => summary.overflow_packets = 1,
                2 => summary.reader_abandoned_packets = 1,
                3 => summary.gap_events = 1,
                4 => summary.discontinuity_events = 1,
                5 => summary.timestamp_error_packets = 1,
                6 => summary.position_overflow_packets = 1,
                7 => summary.counters_saturated = true,
                8 => summary.sequence_exhausted = true,
                9 => summary.stored_frames = 0,
                10 => sink.written_frames = 1,
                11 => sink.complete_packets = 0,
                12 => sink.nonfinite_samples = 1,
                13 => sink.missing_packets = 1,
                14 => sink.incomplete_packets = 1,
                15 => sink.counters_saturated = true,
                _ => sink.cancelled = true,
            }
            let mut failure = None;
            reconcile(
                true,
                true,
                None,
                Some(summary),
                Some(&sink),
                &info(),
                &mut failure,
            );
            assert!(failure.is_some(), "quality variant {variant} escaped");
        }
        let mut failure = None;
        reconcile(
            false,
            false,
            Some("retained native failure"),
            None,
            Some(&sink_report()),
            &info(),
            &mut failure,
        );
        let failure = failure.unwrap();
        for expected in [
            "retained native failure",
            "did not start and stop cleanly",
            "summary is missing",
        ] {
            assert!(failure.contains(expected), "{failure}");
        }
    }

    #[test]
    fn failure_byte_cap_includes_separator_and_utf8_boundaries() {
        for remaining in 0..=6 {
            let prefix = "x".repeat(MAX_FAILURE_BYTES - remaining);
            let mut failure = Some(prefix.clone());
            append_failure(&mut failure, "\u{1f9ea}failure");
            let failure = failure.unwrap();
            assert!(
                failure.len() <= MAX_FAILURE_BYTES,
                "separator escaped cap with {remaining} bytes remaining"
            );
            assert!(failure.starts_with(&prefix));
            assert!(std::str::from_utf8(failure.as_bytes()).is_ok());
        }
        let mut failure = None;
        append_failure(&mut failure, &"\u{1f9ea}".repeat(MAX_FAILURE_BYTES));
        assert_eq!(failure.unwrap().len(), MAX_FAILURE_BYTES);
    }

    #[test]
    fn partial_preparation_and_panicking_native_join_still_retire_owned_resources() {
        let state = Arc::new(State::default());
        let owner = MeasurementSession::own_native(
            config(),
            info(),
            FakeNative {
                state: state.clone(),
                report: None,
            },
        );
        drop(owner);
        assert_eq!(&*state.log.lock().unwrap(), &["native_stop", "native_join"]);

        struct PanickingNative(Arc<State>);
        impl NativeOwner for PanickingNative {
            fn activate(&self) -> Result<()> {
                Ok(())
            }
            fn has_started(&self) -> bool {
                true
            }
            fn is_finished(&self) -> bool {
                false
            }
            fn request_stop(&self) -> Result<()> {
                self.0.log.lock().unwrap().push("native_stop");
                Ok(())
            }
            fn join(&mut self) -> Result<CaptureReport> {
                self.0.log.lock().unwrap().push("native_join");
                panic!("injected lifecycle join panic");
            }
        }
        let state = Arc::new(State::default());
        let mut owner =
            MeasurementSession::own_native(config(), info(), PanickingNative(state.clone()));
        owner.sink = Some(Box::new(FakeSink(state.clone())));
        owner.activate().unwrap();
        let outcome = owner.finish().unwrap();
        assert!(outcome.native_capture.is_none());
        assert!(
            outcome.sink.is_some(),
            "native panic must not suppress partial sink report"
        );
        assert!(outcome.failure.unwrap().contains("native join panicked"));
        assert_eq!(
            &*state.log.lock().unwrap(),
            &["native_stop", "native_join", "sink_join"]
        );
    }

    #[test]
    fn blocked_sink_cannot_delay_stop_and_native_join_then_retained_drain() {
        use std::{sync::mpsc, thread};
        struct BlockedFinish {
            state: Arc<State>,
            entered: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
        }
        impl SinkOwner for BlockedFinish {
            fn check(&self) -> Result<()> {
                Ok(())
            }
            fn cancel(&self) {
                panic!("explicit finish must drain, not cancel");
            }
            fn finish(&mut self) -> Result<SinkReport> {
                self.state.log.lock().unwrap().push("sink_join");
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(sink_report())
            }
        }
        let state = Arc::new(State::default());
        state.native_join_failure.store(true, Ordering::Relaxed);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let mut owner = MeasurementSession::own_native(
            config(),
            info(),
            FakeNative {
                state: state.clone(),
                report: None,
            },
        );
        owner.sink = Some(Box::new(BlockedFinish {
            state: state.clone(),
            entered: entered_tx,
            release: release_rx,
        }));
        owner.activate().unwrap();
        let controller = thread::spawn(move || owner.finish().unwrap());
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let log = state.log.lock().unwrap().clone();
        release_tx.send(()).unwrap();
        let outcome = controller.join().unwrap();
        assert_eq!(log, ["activate", "native_stop", "native_join", "sink_join"]);
        assert_eq!(outcome.sink.unwrap().written_frames, 2);
        assert!(outcome.failure.unwrap().contains("native worker panicked"));
    }

    #[test]
    fn real_ring_is_closed_before_drain_and_quality_failure_keeps_pcm_artifacts() {
        use moiren_windows_audio::capture::{
            PcmEvidenceConfig, PcmEvidencePacket, PcmEvidenceWriter, pcm_evidence_stream,
        };
        use std::{fs, sync::atomic::AtomicU64};
        struct RingNative {
            state: Arc<State>,
            writer: PcmEvidenceWriter,
        }
        impl NativeOwner for RingNative {
            fn activate(&self) -> Result<()> {
                Ok(())
            }
            fn has_started(&self) -> bool {
                true
            }
            fn is_finished(&self) -> bool {
                false
            }
            fn request_stop(&self) -> Result<()> {
                self.state.log.lock().unwrap().push("native_stop");
                Ok(())
            }
            fn join(&mut self) -> Result<CaptureReport> {
                self.state.log.lock().unwrap().push("native_join");
                self.writer.close();
                anyhow::bail!("injected native quality failure; waveform must still drain")
            }
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "moiren-measurement-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let (mut writer, reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 2,
                detect_position_gaps: false,
            },
            44_100,
            1,
        )
        .unwrap();
        let bytes = [0x8000_0000u32.to_le_bytes(), 0x3e80_0000u32.to_le_bytes()].concat();
        writer
            .record_packet(
                &bytes,
                PcmEvidencePacket {
                    frames: 2,
                    flags: 0x8000_0000,
                    device_position_frames: 7,
                    qpc_100ns: 42,
                },
            )
            .unwrap();
        let state = Arc::new(State::default());
        let mut owner = MeasurementSession::own_native(
            config(),
            info(),
            RingNative {
                state: state.clone(),
                writer,
            },
        );
        owner.sink = Some(Box::new(
            EvidenceSink::create(
                reader,
                44_100,
                1,
                &root.join("measurement.f32le"),
                &root.join("measurement.packets.jsonl"),
                crate::measurement_sink::SinkLimits {
                    max_frames: 256,
                    max_pcm_bytes: 1024,
                    max_metadata_bytes: 4096,
                    max_packets: 10,
                },
            )
            .unwrap(),
        ));
        owner.activate().unwrap();
        let outcome = owner.finish().unwrap();
        assert_eq!(fs::read(root.join("measurement.f32le")).unwrap(), bytes);
        assert_eq!(outcome.sink.as_ref().unwrap().written_frames, 2);
        assert_eq!(outcome.sink.as_ref().unwrap().complete_packets, 1);
        assert!(!outcome.sink.as_ref().unwrap().cancelled);
        assert!(outcome.failure.unwrap().contains("native quality failure"));
        assert_eq!(&*state.log.lock().unwrap(), &["native_stop", "native_join"]);
        fs::remove_dir_all(root).unwrap();
    }
}
