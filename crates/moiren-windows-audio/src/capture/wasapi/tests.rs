use super::*;

#[test]
fn target_exit_wakes_without_audio_and_stop_wins_all_signals() {
    let stop = crate::StopSignal::new().unwrap();
    let target = crate::owner::OwnedHandle::event().unwrap();
    let audio = crate::owner::OwnedHandle::event().unwrap();
    unsafe {
        SetEvent(target.0).unwrap();
    }
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 1000).unwrap(),
        Wake::TargetExited
    );
    // Keep target signaled alongside audio; target wins the packet wake.
    unsafe {
        SetEvent(target.0).unwrap();
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 0).unwrap(),
        Wake::TargetExited
    );
    stop.request_stop().unwrap();
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 0).unwrap(),
        Wake::Stop
    );
}
use super::{format::validate_format, packet::transfer_packet};
use crate::{
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    owner::OwnedHandle,
    stats::SILENT,
};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};
use windows::{
    Win32::Media::Audio::*,
    core::{Interface, Result as WinResult, implement},
};

#[implement(IAudioClient)]
struct RetiringClient {
    stop: HANDLE,
    capture: IAudioCaptureClient,
    ready: Rc<RefCell<std::sync::mpsc::Receiver<Result<Startup, CaptureError>>>>,
    observed_finished: Rc<RefCell<Vec<bool>>>,
}
impl IAudioClient_Impl for RetiringClient_Impl {
    fn Initialize(
        &self,
        _: AUDCLNT_SHAREMODE,
        _: u32,
        _: i64,
        _: i64,
        _: *const WAVEFORMATEX,
        _: *const windows::core::GUID,
    ) -> WinResult<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn GetBufferSize(&self) -> WinResult<u32> {
        Ok(480)
    }
    fn GetStreamLatency(&self) -> WinResult<i64> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn GetCurrentPadding(&self) -> WinResult<u32> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn IsFormatSupported(
        &self,
        _: AUDCLNT_SHAREMODE,
        _: *const WAVEFORMATEX,
        _: *mut *mut WAVEFORMATEX,
    ) -> windows::core::HRESULT {
        windows::Win32::Foundation::E_NOTIMPL
    }
    fn GetMixFormat(&self) -> WinResult<*mut WAVEFORMATEX> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn GetDevicePeriod(&self, _: *mut i64, _: *mut i64) -> WinResult<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn Start(&self) -> WinResult<()> {
        unsafe { SetEvent(self.stop) }
    }
    fn Stop(&self) -> WinResult<()> {
        // Hold the real published bridge alive throughout simulated native
        // cleanup. Record outside-FFI assertions so failures cannot unwind COM.
        if let Ok(Ok(ready)) = self.ready.borrow().try_recv() {
            self.observed_finished
                .borrow_mut()
                .push(ready.observer.producer_finished());
            std::thread::sleep(std::time::Duration::from_millis(20));
            self.observed_finished
                .borrow_mut()
                .push(ready.observer.producer_finished());
        }
        Ok(())
    }
    fn Reset(&self) -> WinResult<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn SetEventHandle(&self, _: HANDLE) -> WinResult<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    }
    fn GetService(
        &self,
        riid: *const windows::core::GUID,
        ppv: *mut *mut core::ffi::c_void,
    ) -> WinResult<()> {
        // SAFETY: run() requests this service with valid IID/output pointers.
        unsafe {
            if *riid != IAudioCaptureClient::IID {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_NOINTERFACE,
                ));
            }
            *ppv = self.capture.clone().into_raw();
        }
        Ok(())
    }
}

#[test]
fn producer_retirement_is_visible_before_and_during_native_stop_cleanup() {
    let stop = StopSignal::new().unwrap();
    let audio = OwnedHandle::event().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let observed = Rc::new(RefCell::new(Vec::new()));
    let capture: IAudioCaptureClient = FakeCapture {
        data: Rc::new(UnsafeCell::new([0.0; 4])),
        releases: Rc::new(RefCell::new(Vec::new())),
        frames: 0,
        flags: 0,
        null_buffer: true,
    }
    .into();
    let client: IAudioClient = RetiringClient {
        stop: handle(&stop.event),
        capture,
        ready: Rc::new(RefCell::new(receiver)),
        observed_finished: Rc::clone(&observed),
    }
    .into();
    let mut report = CaptureReport::new(
        crate::capture::CaptureSource::Physical,
        std::time::Duration::from_secs(1),
    );
    let status = stream::run(
        stream::StreamInput {
            client: &client,
            audio: audio.0,
            target: None,
            stop: &stop.event,
            duration: std::time::Duration::from_secs(1),
            sample_rate: 48000,
            channels: 2,
        },
        &sender,
        &mut report,
    )
    .unwrap();
    assert_eq!(status, crate::capture::CaptureStatus::Stopped);
    assert!(report.stop_succeeded);
    assert_eq!(*observed.borrow(), [true, true]);
}

#[test]
fn capture_stop_is_responsive_without_audio_and_has_priority() {
    let stop = StopSignal::new().unwrap().event;
    let audio = OwnedHandle::event().unwrap();
    assert_eq!(wait(handle(&stop), audio.0, 0).unwrap(), Wake::Timeout);
    unsafe {
        SetEvent(handle(&stop)).unwrap();
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
}

#[test]
fn linked_render_and_capture_stop_without_waiting_for_worker_cleanup() {
    let session = CaptureSession {
        stop: StopSignal::new().unwrap().event,
        worker: None,
    };
    let peer = session.stop_signal();
    let audio = OwnedHandle::event().unwrap();
    peer.request_stop().unwrap();
    assert_eq!(
        wait(handle(&session.stop), audio.0, 100).unwrap(),
        Wake::Stop
    );
}

#[test]
fn native_capture_format_requires_supported_rate_float_and_channel_order() {
    let base = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    assert!(validate_format(base, true, Some(32), Some(3)).is_ok());
    let mono = WAVEFORMATEX {
        nChannels: 1,
        nBlockAlign: 4,
        nAvgBytesPerSec: 192000,
        ..base
    };
    assert!(validate_format(mono, true, None, Some(4)).is_ok());
    let rate = WAVEFORMATEX {
        nSamplesPerSec: 44100,
        nAvgBytesPerSec: 352800,
        ..base
    };
    assert!(validate_format(rate, true, None, None).is_ok());
    for bad in [
        WAVEFORMATEX {
            nSamplesPerSec: 96000,
            ..base
        },
        WAVEFORMATEX {
            nChannels: 6,
            ..base
        },
        WAVEFORMATEX {
            nBlockAlign: 4,
            ..base
        },
        WAVEFORMATEX {
            nAvgBytesPerSec: 1,
            ..base
        },
    ] {
        assert!(validate_format(bad, true, None, None).is_err());
    }
    assert!(validate_format(base, false, None, None).is_err());
    assert!(validate_format(base, true, Some(24), None).is_err());
    assert!(validate_format(base, true, Some(32), Some(12)).is_err());
}

#[implement(IAudioCaptureClient)]
struct FakeCapture {
    data: Rc<UnsafeCell<[f32; 4]>>,
    releases: Rc<RefCell<Vec<u32>>>,
    frames: u32,
    flags: u32,
    null_buffer: bool,
}
impl IAudioCaptureClient_Impl for FakeCapture_Impl {
    fn GetBuffer(
        &self,
        data: *mut *mut u8,
        frames: *mut u32,
        flags: *mut u32,
        position: *mut u64,
        qpc: *mut u64,
    ) -> WinResult<()> {
        // SAFETY: transfer_packet provides valid outputs on this test thread.
        unsafe {
            *data = if self.null_buffer {
                std::ptr::null_mut()
            } else {
                self.data.get().cast()
            };
            *frames = self.frames;
            *flags = self.flags;
            *position = 0;
            *qpc = 1;
        }
        Ok(())
    }
    fn ReleaseBuffer(&self, frames: u32) -> WinResult<()> {
        self.releases.borrow_mut().push(frames);
        Ok(())
    }
    fn GetNextPacketSize(&self) -> WinResult<u32> {
        Ok(self.frames)
    }
}

#[test]
fn capture_lease_releases_complete_packet_on_success_silence_and_validation_failure() {
    for (frames, flags, null_buffer, fails) in [
        (2, 0, false, false),
        (2, SILENT, true, false),
        (2, 0, true, true),
        (3, 0, false, true),
        (0, 0, true, false),
    ] {
        let releases = Rc::new(RefCell::new(Vec::new()));
        let client: IAudioCaptureClient = FakeCapture {
            data: Rc::new(UnsafeCell::new([0.25, -0.25, 0.5, -0.5])),
            releases: Rc::clone(&releases),
            frames,
            flags,
            null_buffer,
        }
        .into();
        let (mut input, _source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        assert_eq!(transfer_packet(&client, 2, &mut input).is_err(), fails);
        assert_eq!(
            *releases.borrow(),
            if frames == 0 { vec![] } else { vec![frames] }
        );
        assert_eq!(
            observer.snapshot().captured_frames,
            if fails { 0 } else { frames as u64 }
        );
    }
}
