//! Hardware-free exercise of the actual shared capture loop, before join.
use super::*;
use crate::{capture::CaptureSource, clock_bridge::ClockBridgeConfig, stats::SILENT};
use std::{cell::Cell, sync::mpsc};
use windows::{
    Win32::{Foundation::E_NOINTERFACE, Media::Audio::*},
    core::{GUID, HRESULT, Interface, Result as WinResult, implement},
};

#[implement(IAudioCaptureClient)]
struct Packets {
    audio: HANDLE,
    next: Cell<u32>,
    packet: Cell<u64>,
    origin: u64,
    reached: mpsc::SyncSender<()>,
    resume: mpsc::Receiver<()>,
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
        unsafe { SetEvent(self.audio) }
    }
    fn GetNextPacketSize(&self) -> WinResult<u32> {
        let next = self.next.get();
        self.next.set(next + 1);
        match next {
            0 | 1 => Ok(8),
            2 => {
                unsafe { SetEvent(self.audio)? };
                Ok(0)
            }
            _ => {
                // Only the fake blocks: park after a complete real callback so
                // the control thread can inspect observations before Stop/join.
                self.reached.send(()).unwrap();
                self.resume.recv().unwrap();
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
        let prepared = super::super::start_owner(stop, move |stop, sender, started| {
            let audio = crate::owner::OwnedHandle::event().unwrap();
            let capture: IAudioCaptureClient = Packets {
                audio: audio.0,
                next: Cell::new(0),
                packet: Cell::new(0),
                origin: QpcClock::prepare(TelemetryMode::Summary)
                    .unwrap()
                    .now()
                    .unwrap()
                    - 100_000,
                reached,
                resume: continue_worker,
            }
            .into();
            let client: IAudioClient = Client {
                capture,
                audio: audio.0,
            }
            .into();
            let mut report = CaptureReport::new(source_kind, SessionDuration::UntilStopped);
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
        resume.send(()).unwrap();
        let report = prepared.session.join().unwrap();
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
