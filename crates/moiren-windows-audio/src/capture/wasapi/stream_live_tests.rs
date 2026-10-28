//! Hardware-free exercise of the actual shared capture loop, before join.
use super::*;
use crate::{capture::CaptureSource, clock_bridge::ClockBridgeConfig, stats::SILENT};
use std::{cell::Cell, sync::mpsc};
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
            *frames = 8;
            *flags = SILENT;
            *position = packet * 8;
            *qpc = self.origin + (packet * 8 * 10_000_000).div_ceil(48_000);
        }
        self.packet.set(packet + 1);
        Ok(())
    }
    fn ReleaseBuffer(&self, _: u32) -> WinResult<()> {
        unsafe { SetEvent(self.audio)? };
        if matches!(self.end, DrainEnd::StopAfterRelease) && self.packet.get() == 2 {
            unsafe { SetEvent(self.stop)? };
        }
        Ok(())
    }
    fn GetNextPacketSize(&self) -> WinResult<u32> {
        let next = self.next.get();
        self.next.set(next + 1);
        match next {
            0 | 1 => Ok(8),
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
        Ok(64)
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
            }
            .into();
            let client: IAudioClient = Client {
                capture,
                audio: audio.0,
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
        }
        .into();
        let client: IAudioClient = Client {
            capture,
            audio: audio.0,
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
