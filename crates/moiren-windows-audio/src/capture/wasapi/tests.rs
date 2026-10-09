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
    core::{Result as WinResult, implement},
};

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
