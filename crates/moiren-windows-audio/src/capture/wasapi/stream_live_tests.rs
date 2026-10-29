//! Hardware-free exercise of the actual shared capture loop, before join.
use super::*;
use crate::{capture::CaptureSource, clock_bridge::ClockBridgeConfig, stats::SILENT};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
};
use windows::{
    Win32::{Foundation::E_NOINTERFACE, Media::Audio::*},
    core::{GUID, HRESULT, Interface, Result as WinResult, implement},
};

enum DrainEnd {
    Observe {
        reached: mpsc::SyncSender<()>,
        resume: mpsc::Receiver<()>,
    },
    StopAfterRelease,
    PacketQueryError,
}

#[implement(IAudioCaptureClient)]
struct Packets {
    audio: HANDLE,
    stop: HANDLE,
    next: Cell<u32>,
    packet: Cell<u64>,
    origin: u64,
    end: DrainEnd,
    frames: u32,
    releases: Rc<RefCell<Vec<u32>>>,
    fail_release: bool,
}
impl IAudioCaptureClient_Impl for Packets_Impl {
    fn GetBuffer(
        &self,
        data: *mut *mut u8,
        frames: *mut u32,
        flags: *mut u32,
        position: *mut u64,
        qpc: *mut u64,
    ) -> WinResult<()> {
        let packet = self.packet.get();
        // SAFETY: the real transfer function supplies live scalar outputs.
        unsafe {
            *data = std::ptr::null_mut();
            *frames = self.frames;
            *flags = SILENT;
            *position = packet * u64::from(self.frames);
            *qpc = self.origin + (packet * u64::from(self.frames) * 10_000_000).div_ceil(48_000);
        }
        self.packet.set(packet + 1);
        Ok(())
    }
    fn ReleaseBuffer(&self, frames: u32) -> WinResult<()> {
        self.releases.borrow_mut().push(frames);
        unsafe { SetEvent(self.audio)? };
        if matches!(self.end, DrainEnd::StopAfterRelease) && self.packet.get() == 2 {
            unsafe { SetEvent(self.stop)? };
        }
        if self.fail_release {
            Err(E_NOINTERFACE.into())
        } else {
            Ok(())
        }
    }
    fn GetNextPacketSize(&self) -> WinResult<u32> {
        let next = self.next.get();
        self.next.set(next + 1);
        match next {
            0 | 1 => Ok(self.frames),
            2 if matches!(self.end, DrainEnd::Observe { .. }) => {
                unsafe { SetEvent(self.audio)? };
                Ok(0)
            }
            _ => {
                // Only the fake blocks: park after a complete real callback so
                // the control thread can inspect observations before Stop/join.
                if let DrainEnd::Observe { reached, resume } = &self.end {
                    reached.send(()).unwrap();
                    resume.recv().unwrap();
                }
                Err(E_NOINTERFACE.into())
            }
        }
    }
}

#[implement(IAudioClient)]
struct Client {
    capture: IAudioCaptureClient,
    audio: HANDLE,
    buffer_size: Result<u32, i32>,
}
impl IAudioClient_Impl for Client_Impl {
    fn Initialize(
        &self,
        _: AUDCLNT_SHAREMODE,
        _: u32,
        _: i64,
        _: i64,
        _: *const WAVEFORMATEX,
        _: *const GUID,
    ) -> WinResult<()> {
        unreachable!()
    }
    fn GetBufferSize(&self) -> WinResult<u32> {
        self.buffer_size
            .map_err(|status| windows::core::Error::from_hresult(HRESULT(status)))
    }
    fn GetStreamLatency(&self) -> WinResult<i64> {
        Ok(0)
    }
    fn GetCurrentPadding(&self) -> WinResult<u32> {
        unreachable!()
    }
    fn IsFormatSupported(
        &self,
        _: AUDCLNT_SHAREMODE,
        _: *const WAVEFORMATEX,
        _: *mut *mut WAVEFORMATEX,
    ) -> HRESULT {
        unreachable!()
    }
    fn GetMixFormat(&self) -> WinResult<*mut WAVEFORMATEX> {
        unreachable!()
    }
    fn GetDevicePeriod(&self, _: *mut i64, _: *mut i64) -> WinResult<()> {
        unreachable!()
    }
    fn Start(&self) -> WinResult<()> {
        unsafe { SetEvent(self.audio) }
    }
    fn Stop(&self) -> WinResult<()> {
        Ok(())
    }
    fn Reset(&self) -> WinResult<()> {
        unreachable!()
    }
    fn SetEventHandle(&self, _: HANDLE) -> WinResult<()> {
        unreachable!()
    }
    fn GetService(&self, iid: *const GUID, object: *mut *mut std::ffi::c_void) -> WinResult<()> {
        // SAFETY: caller requests an owned COM interface through GetService.
        unsafe { self.capture.query(iid, object).ok() }
    }
}

#[test]
fn native_delivery_is_visible_before_join_and_age_clock_works_with_trace_disabled() {
    for source_kind in [CaptureSource::Physical, CaptureSource::ProcessLoopback] {
        let stop = crate::StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let worker_gate = gate.clone();
        let (reached, observed) = mpsc::sync_channel(1);
        let (resume, continue_worker) = mpsc::sync_channel(1);
        let (tap, mut evidence) = crate::capture::pcm_evidence_stream(
            crate::capture::PcmEvidenceConfig {
                capacity_frames: 32,
                detect_position_gaps: source_kind == CaptureSource::Physical,
            },
            48_000,
            2,
        )
        .unwrap();
        let prepared = super::super::start_owner(stop, move |stop, sender, started| {
            let audio = crate::owner::OwnedHandle::event().unwrap();
            let capture: IAudioCaptureClient = Packets {
                audio: audio.0,
                stop: handle(&stop),
                next: Cell::new(0),
                packet: Cell::new(0),
                origin: QpcClock::prepare(TelemetryMode::Summary)
                    .unwrap()
                    .now()
                    .unwrap()
                    - 100_000,
                end: DrainEnd::Observe {
                    reached,
                    resume: continue_worker,
                },
                frames: 8,
                releases: Rc::default(),
                fail_release: false,
            }
            .into();
            let client: IAudioClient = Client {
                capture,
                audio: audio.0,
                buffer_size: Ok(64),
            }
            .into();
            let mut report = CaptureReport::new(source_kind, SessionDuration::UntilStopped);
            report.evidence_recorder = Some(tap);
            let result = run(
                StreamInput {
                    client: &client,
                    audio: audio.0,
                    target: None,
                    stop: &stop,
                    duration: SessionDuration::UntilStopped,
                    latency_telemetry: TelemetryMode::Disabled,
                    gate: &worker_gate,
                    started: &started,
                    sample_rate: 48_000,
                    channels: 2,
                    artifact: None,
                },
                &sender,
                &mut report,
                ClockBridgeConfig {
                    capacity_frames: 64,
                    target_fill_frames: 2,
                    trim_on_prime: false,
                    max_correction_ppm: 0.0,
                    detect_position_gaps: source_kind == CaptureSource::Physical,
                    ..Default::default()
                },
            );
            finish(result, &stop, &sender, report)
        })
        .unwrap();
        let before = serde_json::to_value(prepared.observer.snapshot()).unwrap();
        gate.activate().unwrap();
        observed.recv_timeout(Duration::from_secs(10)).unwrap();
        let live = serde_json::to_value(prepared.observer.snapshot()).unwrap();
        let mut source = prepared.source;
        source.read_interleaved(&mut [0.0; 16]).unwrap();
        source.read_interleaved(&mut [0.0; 2]).unwrap();
        let reads = prepared.observer.queue_history().unwrap();
        assert_eq!(reads.total_events, 2);
        for event in &reads.events {
            assert!(
                event.sample.qpc_before_100ns.unwrap() <= event.sample.qpc_after_100ns.unwrap()
            );
        }
        assert_eq!(
            reads.unavailable_clocks, 0,
            "prepared host timing survives disabled trace collection"
        );
        assert_eq!(reads.regressed_clocks, 0);
        let age = serde_json::to_value(prepared.observer.snapshot()).unwrap();
        let running = !prepared.session.is_finished();
        let evidence_first = evidence.pop();
        resume.send(()).unwrap();
        let report = prepared.session.join().unwrap();
        let first = evidence_first.expect("native leased PCM must be visible before join");
        assert_eq!(first.samples, [0.0; 2]);
        assert_eq!(first.packet_sequence, 1);
        assert_eq!(first.packet_frames, 8);
        assert_eq!(first.device_position_frames, 0);
        assert_eq!(first.flags, SILENT);
        assert!(first.qpc_100ns > 0);
        let mut retained_frames = 1;
        while evidence.pop().is_some() {
            retained_frames += 1;
        }
        assert_eq!(retained_frames, 16);
        assert!(evidence.is_finished());
        assert_eq!(report.evidence_metadata.unwrap().stored_frames, 16);
        assert!(before["native_delivery"].is_null());
        assert!(running);
        assert_eq!(live["native_delivery"]["packets"], 2);
        assert_eq!(live["native_delivery"]["max_packet_frames"], 8);
        assert_eq!(live["native_delivery"]["max_callback_burst_frames"], 16);
        assert_eq!(live["native_delivery"]["gap_samples"], 1);
        assert_eq!(live["silent_frames"], 16);
        assert_eq!(live["dropped_frames"], 0);
        assert_eq!(
            age["oldest_frame_age"].is_object(),
            source_kind == CaptureSource::Physical
        );
        assert_eq!(
            live["native_delivery"],
            serde_json::to_value(report.delivery).unwrap()
        );
        assert_eq!(report.packets, 2);
        assert!(report.stop_succeeded);
        assert_eq!(report.latency.mode, TelemetryMode::Disabled);
        assert_eq!(report.latency.trace.records().len(), 0);
    }
}

fn interrupted_drain(end: DrainEnd) -> (CaptureReport, crate::clock_bridge::BridgeObserver) {
    let stop = crate::StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let worker_gate = gate.clone();
    let prepared = super::super::start_owner(stop, move |stop, sender, started| {
        let audio = crate::owner::OwnedHandle::event().unwrap();
        let capture: IAudioCaptureClient = Packets {
            audio: audio.0,
            stop: handle(&stop),
            next: Cell::new(0),
            packet: Cell::new(0),
            origin: QpcClock::new().unwrap().now().unwrap(),
            end,
            frames: 8,
            releases: Rc::default(),
            fail_release: false,
        }
        .into();
        let client: IAudioClient = Client {
            capture,
            audio: audio.0,
            buffer_size: Ok(64),
        }
        .into();
        let mut report = CaptureReport::new(
            CaptureSource::ProcessLoopback,
            SessionDuration::UntilStopped,
        );
        let result = run(
            StreamInput {
                client: &client,
                audio: audio.0,
                target: None,
                stop: &stop,
                duration: SessionDuration::UntilStopped,
                latency_telemetry: TelemetryMode::Trace,
                gate: &worker_gate,
                started: &started,
                sample_rate: 48_000,
                channels: 2,
                artifact: None,
            },
            &sender,
            &mut report,
            ClockBridgeConfig {
                capacity_frames: 64,
                target_fill_frames: 2,
                trim_on_prime: false,
                max_correction_ppm: 0.0,
                ..Default::default()
            },
        );
        finish(result, &stop, &sender, report)
    })
    .unwrap();
    let observer = prepared.observer;
    gate.activate().unwrap();
    let report = prepared.session.join().unwrap();
    (report, observer)
}

fn assert_final_drain_is_retained(
    report: &CaptureReport,
    observer: &crate::clock_bridge::BridgeObserver,
) {
    let history = observer.delivery_events().unwrap();
    assert_eq!(history.total_events, 1);
    assert_eq!(history.overwritten_events, 0);
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].frames, 16);
    assert!(history.events[0].qpc_100ns.is_some());
    let packets = observer.packet_history().unwrap();
    assert_eq!(
        packets.total_events, 2,
        "individual packets, not the drain total"
    );
    assert_eq!(packets.offered_frames, 16);
    assert_eq!(packets.published_frames, 16);
    for event in &packets.events {
        assert_eq!(event.sample.offered_frames, 8);
        assert_eq!(event.sample.accepted_frames, 8);
        assert_eq!(
            event.sample.origin,
            crate::latency::packets::PacketPublicationOrigin::NativeCapture
        );
        let before = event.sample.qpc_before_100ns.unwrap();
        let after = event.sample.qpc_after_100ns.unwrap();
        assert!(before <= after);
        assert!(after <= history.events[0].qpc_100ns.unwrap());
    }
    assert_eq!(packets.unavailable_clocks, 0);
    assert_eq!(packets.regressed_clocks, 0);
    assert!(!packets.saturated);
    assert!(observer.producer_finished());
    let counters = observer.snapshot();
    assert_eq!(counters.captured_frames, 16);
    assert_eq!(counters.written_frames, 16);
    assert_eq!(counters.native_delivery, Some(report.delivery));
    assert_eq!(report.packets, 2);
    assert_eq!(report.delivery.max_callback_burst_frames, 16);
    assert!(report.stop_succeeded);
    assert_eq!(report.shared_period.actual_period_frames, None);
    let trace = report.latency.trace.records();
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].demand_frames, 16);
    assert_eq!(trace[0].submitted_frames, 16);
    assert_eq!(report.latency.demand_frames.samples, 1);
    assert_eq!(report.latency.wake_interval_100ns.samples, 0);
}

#[test]
fn native_drain_history_survives_stop_after_successful_packet_releases() {
    let (report, observer) = interrupted_drain(DrainEnd::StopAfterRelease);
    assert_eq!(report.status, CaptureStatus::Stopped);
    assert_eq!(report.failure, None);
    assert_final_drain_is_retained(&report, &observer);
}

#[test]
fn native_drain_history_survives_packet_query_error_after_successful_releases() {
    let (report, observer) = interrupted_drain(DrainEnd::PacketQueryError);
    assert_eq!(report.status, CaptureStatus::Failed);
    assert_eq!(
        report.failure,
        Some(
            CaptureError::Api {
                stage: "GetNextPacketSize(capture)",
                hresult: E_NOINTERFACE.0,
            }
            .to_string()
        )
    );
    assert_final_drain_is_retained(&report, &observer);
}

fn rejected_buffer_query(
    source: CaptureSource,
    buffer_size: Result<u32, i32>,
) -> (CaptureError, CaptureReport) {
    let stop = crate::StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let audio = crate::owner::OwnedHandle::event().unwrap();
    let capture: IAudioCaptureClient = Packets {
        audio: audio.0,
        stop: handle(&stop.event),
        next: Cell::new(0),
        packet: Cell::new(0),
        origin: 1,
        end: DrainEnd::PacketQueryError,
        frames: 8,
        releases: Rc::default(),
        fail_release: false,
    }
    .into();
    let client: IAudioClient = Client {
        capture,
        audio: audio.0,
        buffer_size,
    }
    .into();
    let mut report = CaptureReport::new(source, SessionDuration::UntilStopped);
    let (sender, receiver) = mpsc::sync_channel(1);
    let error = run(
        StreamInput {
            client: &client,
            audio: audio.0,
            target: None,
            stop: &stop.event,
            duration: SessionDuration::UntilStopped,
            latency_telemetry: TelemetryMode::Disabled,
            gate: &gate,
            started: &AtomicBool::new(false),
            sample_rate: 48_000,
            channels: 2,
            artifact: None,
        },
        &sender,
        &mut report,
        ClockBridgeConfig::default(),
    )
    .unwrap_err();
    assert!(!report.stream_started);
    assert!(
        receiver.try_recv().is_err(),
        "no prepared source on refusal"
    );
    (error, report)
}

#[test]
fn capture_buffer_query_retains_zero_and_oversized_raw_values_without_fallback() {
    for frames in [0, 1_048_577, 3_722_304_989, u32::MAX] {
        let (error, report) = rejected_buffer_query(CaptureSource::Physical, Ok(frames));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["buffer_size_query"]["hresult"], 0);
        assert_eq!(json["buffer_size_query"]["raw_frames"], frames);
        assert_eq!(report.buffer_frames, None, "not an admitted capacity");
        assert!(error.to_string().contains(&frames.to_string()));
        assert_eq!(report.input_queue, None);
    }
}

#[test]
fn capture_buffer_query_refuses_s_false_and_retains_unchanged_output() {
    // The COM implementation macro returns this HRESULT without writing the
    // output. The ordinary windows binding currently maps S_FALSE to Ok(0).
    let (error, report) = rejected_buffer_query(CaptureSource::ProcessLoopback, Err(1));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["buffer_size_query"]["hresult"], 1);
    assert_eq!(json["buffer_size_query"]["output_changed"], false);
    assert!(error.to_string().contains("expected S_OK"));
    assert_eq!(report.buffer_frames, None);
}

#[test]
fn capture_buffer_query_retains_failed_hresult_before_returning() {
    let (error, report) =
        rejected_buffer_query(CaptureSource::ProcessLoopback, Err(E_NOINTERFACE.0));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["buffer_size_query"]["hresult"], E_NOINTERFACE.0);
    assert_eq!(json["buffer_size_query"]["output_changed"], false);
    assert_eq!(
        error,
        CaptureError::Api {
            stage: "GetBufferSize(capture)",
            hresult: E_NOINTERFACE.0,
        }
    );
    assert_eq!(report.buffer_frames, None);
}

#[test]
fn capture_buffer_query_refuses_s_ok_without_output_instead_of_using_a_default() {
    // Even S_OK cannot authorize a capacity when the output remains poisoned.
    let (error, report) = rejected_buffer_query(CaptureSource::Physical, Err(0));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["buffer_size_query"]["hresult"], 0);
    assert_eq!(json["buffer_size_query"]["output_changed"], false);
    assert!(error.to_string().contains("8 MiB"));
    assert_eq!(report.buffer_frames, None);
}

fn process_bounded_packet_run(
    source: CaptureSource,
    buffer_size: Result<u32, i32>,
    frames: u32,
    fail_release: bool,
) -> (Result<CaptureStatus, CaptureError>, CaptureReport, Vec<u32>) {
    let stop = crate::StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    gate.activate().unwrap();
    let audio = crate::owner::OwnedHandle::event().unwrap();
    let releases = Rc::new(RefCell::new(Vec::new()));
    let capture: IAudioCaptureClient = Packets {
        audio: audio.0,
        stop: handle(&stop.event),
        next: Cell::new(0),
        packet: Cell::new(0),
        origin: 1,
        end: DrainEnd::StopAfterRelease,
        frames,
        releases: releases.clone(),
        fail_release,
    }
    .into();
    let client: IAudioClient = Client {
        capture,
        audio: audio.0,
        buffer_size,
    }
    .into();
    let mut report = CaptureReport::new(source, SessionDuration::UntilStopped);
    let (sender, _receiver) = mpsc::sync_channel(1);
    let result = run(
        StreamInput {
            client: &client,
            audio: audio.0,
            target: None,
            stop: &stop.event,
            duration: SessionDuration::UntilStopped,
            latency_telemetry: TelemetryMode::Disabled,
            gate: &gate,
            started: &AtomicBool::new(false),
            sample_rate: 48_000,
            channels: 2,
            artifact: Some(crate::capture::PcmArtifactConfig::new(32)),
        },
        &sender,
        &mut report,
        ClockBridgeConfig {
            capacity_frames: 64,
            target_fill_frames: 2,
            ..Default::default()
        },
    );
    let report = finish(result, &stop.event, &sender, report);
    let released = releases.borrow().clone();
    (result, report, released)
}

#[test]
fn process_bounded_packet_s_ok_query_is_never_native_capacity() {
    for raw in [Ok(0), Ok(1), Ok(64), Ok(6_684_721), Ok(u32::MAX), Err(0)] {
        let (result, report, released) =
            process_bounded_packet_run(CaptureSource::ProcessLoopback, raw, 8, false);
        assert_eq!(
            result,
            Ok(CaptureStatus::Stopped),
            "{raw:?}: {:?}",
            report.failure
        );
        let query = report.buffer_size_query.unwrap();
        assert_eq!(query.hresult, 0);
        assert_eq!(query.raw_frames, raw.unwrap_or(u32::MAX));
        assert_eq!(
            query.output_changed,
            raw.is_ok_and(|frames| frames != u32::MAX)
        );
        assert_eq!(
            report.buffer_frames, None,
            "even a plausible value is not trusted"
        );
        assert_eq!(report.shared_period.actual_period_frames, None);
        assert_eq!(
            report.input_queue, None,
            "compatible queue remains compatible"
        );
        assert!(report.stream_started && report.stop_succeeded);
        assert_eq!(report.delivery.packets, 2);
        assert_eq!(report.delivery.max_packet_frames, 8);
        assert_eq!(released, [8, 8]);
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["schema_version"], 11);
        assert!(json.as_object().unwrap().contains_key("buffer_frames"));
        assert!(json["buffer_frames"].is_null(), "unknown stays explicit");
        assert_eq!(json["buffer_size_query"]["raw_frames"], query.raw_frames);
        assert!(json["shared_period"]["actual_period_frames"].is_null());
        assert_eq!(json["process_packet_safety_bound_bytes"], 8 * 1024 * 1024);
        assert_eq!(json["artifact_metadata"]["observed_frames"], 16);
    }
}

#[test]
fn process_bounded_packet_non_s_ok_query_still_refuses_before_start() {
    for source in [CaptureSource::ProcessLoopback, CaptureSource::Physical] {
        for status in [1, E_NOINTERFACE.0, windows::Win32::Foundation::E_NOTIMPL.0] {
            let (result, report, released) =
                process_bounded_packet_run(source, Err(status), 8, false);
            assert!(result.is_err());
            assert!(!report.stream_started);
            assert!(released.is_empty());
            assert_eq!(report.buffer_size_query.unwrap().hresult, status);
            assert_eq!(report.buffer_frames, None);
        }
    }
}

#[test]
fn process_bounded_packet_limit_accepts_exact_bytes_but_refuses_one_more_frame() {
    let exact_frames = 8 * 1024 * 1024 / 8;
    let (result, report, released) =
        process_bounded_packet_run(CaptureSource::ProcessLoopback, Ok(0), exact_frames, false);
    assert_eq!(result, Ok(CaptureStatus::Stopped));
    assert_eq!(released, [exact_frames, exact_frames]);
    assert_eq!(report.delivery.max_packet_frames, exact_frames);
    for frames in [exact_frames + 1, u32::MAX] {
        for fail_release in [false, true] {
            let (result, report, released) = process_bounded_packet_run(
                CaptureSource::ProcessLoopback,
                Ok(0),
                frames,
                fail_release,
            );
            assert!(result.is_err());
            assert_eq!(released, [frames], "complete lease released exactly once");
            assert!(report.stream_started && report.stop_succeeded);
            assert_eq!(report.delivery.packets, 0);
            let text = report.failure.unwrap();
            assert!(
                text.contains("packet") && text.contains("8388608"),
                "{text}"
            );
            assert!(text.contains(&frames.to_string()));
            if fail_release {
                assert!(
                    text.contains("release") && text.contains(&E_NOINTERFACE.0.to_string()),
                    "{text}"
                );
            }
            assert_eq!(report.artifact_metadata.unwrap().observed_frames, 0);
        }
    }
}

#[test]
fn process_bounded_packet_split_preserves_physical_capacity_enforcement() {
    let (result, report, released) =
        process_bounded_packet_run(CaptureSource::Physical, Ok(64), 8, false);
    assert_eq!(result, Ok(CaptureStatus::Stopped));
    assert_eq!(report.buffer_frames, Some(64));
    assert_eq!(released, [8, 8]);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema_version"], 11);
    assert_eq!(json["buffer_frames"], 64);
    assert!(json.get("process_packet_safety_bound_bytes").is_none());
    let (result, report, released) =
        process_bounded_packet_run(CaptureSource::Physical, Ok(1), 8, false);
    assert_eq!(result, Err(CaptureError::InvalidPacket));
    assert_eq!(report.buffer_frames, Some(1));
    assert_eq!(released, [8]);
}

#[test]
fn process_bounded_packet_unknown_capacity_prepares_without_start_before_gate() {
    let stop = crate::StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let worker_gate = gate.clone();
    let prepared = super::super::start_owner(stop, move |stop, sender, started| {
        let audio = crate::owner::OwnedHandle::event().unwrap();
        let capture: IAudioCaptureClient = Packets {
            audio: audio.0,
            stop: handle(&stop),
            next: Cell::new(0),
            packet: Cell::new(0),
            origin: 1,
            end: DrainEnd::PacketQueryError,
            frames: 8,
            releases: Rc::default(),
            fail_release: false,
        }
        .into();
        let client: IAudioClient = Client {
            capture,
            audio: audio.0,
            buffer_size: Ok(0),
        }
        .into();
        let mut report = CaptureReport::new(
            CaptureSource::ProcessLoopback,
            SessionDuration::UntilStopped,
        );
        let result = run(
            StreamInput {
                client: &client,
                audio: audio.0,
                target: None,
                stop: &stop,
                duration: SessionDuration::UntilStopped,
                latency_telemetry: TelemetryMode::Disabled,
                gate: &worker_gate,
                started: &started,
                sample_rate: 48_000,
                channels: 2,
                artifact: None,
            },
            &sender,
            &mut report,
            ClockBridgeConfig::default(),
        );
        finish(result, &stop, &sender, report)
    })
    .unwrap();
    assert!(!prepared.session.has_started());
    assert!(!prepared.session.is_finished());
    assert_eq!(prepared.observer.snapshot().captured_frames, 0);
    assert_eq!(prepared.source.prepared_input(), None);
    gate.cancel().unwrap();
    let report = prepared.session.join().unwrap();
    assert_eq!(report.status, CaptureStatus::Stopped);
    assert!(!report.stream_started);
    assert_eq!(report.buffer_frames, None);
    assert_eq!(report.shared_period.actual_period_frames, None);
    assert_eq!(report.packets, 0);
}
