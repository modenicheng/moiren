use super::*;
use crate::{
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    test_allocation as allocation_guard,
};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};
use windows::{
    Win32::Media::Audio::*,
    core::{Result as WinResult, implement},
};

#[implement(IAudioCaptureClient)]
struct FakeCapture {
    data: Rc<UnsafeCell<[f32; 4]>>,
    releases: Rc<RefCell<Vec<u32>>>,
    frames: u32,
    flags: u32,
    null_buffer: bool,
    fail_release: bool,
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
        if self.fail_release {
            Err(windows::Win32::Foundation::E_NOINTERFACE.into())
        } else {
            Ok(())
        }
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
            fail_release: false,
        }
        .into();
        let (mut input, _source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        assert_eq!(
            transfer_packet(
                &client,
                PacketBound::NativeCapacity(2),
                &mut input,
                None,
                None
            )
            .is_err(),
            fails
        );
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

#[test]
fn published_packet_evidence_survives_release_failure_without_healing_native_error() {
    let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
    let client: IAudioCaptureClient = FakeCapture {
        data: Rc::new(UnsafeCell::new([0.25, -0.25, 0.5, -0.5])),
        releases: releases.clone(),
        frames: 2,
        flags: 0,
        null_buffer: false,
        fail_release: true,
    }
    .into();
    let (mut input, _source, observer) = crate::clock_bridge::capture_bridge_with_clock(
        ClockBridgeConfig::default(),
        crate::latency::QpcClock::new(),
    )
    .unwrap();
    let error = transfer_packet(
        &client,
        PacketBound::NativeCapacity(2),
        &mut input,
        None,
        None,
    )
    .err()
    .unwrap();
    assert_eq!(
        error,
        CaptureError::Api {
            stage: "ReleaseBuffer(capture)",
            hresult: windows::Win32::Foundation::E_NOINTERFACE.0,
        }
    );
    assert_eq!(releases.borrow().as_slice(), [2]);
    let packets = observer.packet_history().unwrap();
    assert_eq!(packets.total_events, 1);
    assert_eq!(packets.offered_frames, 2);
    assert_eq!(packets.published_frames, 2);
    let sample = packets.events[0].sample;
    assert!(sample.qpc_before_100ns.unwrap() <= sample.qpc_after_100ns.unwrap());
    assert!(
        sample.qpc_after_100ns.unwrap() > 1,
        "GetBuffer sample stamp is not the commit clock"
    );
    assert!(
        observer.delivery_events().unwrap().events.is_empty(),
        "no successfully released drain"
    );
}

#[test]
fn capture_lease_copies_raw_evidence_before_release_without_allocating() {
    use crate::capture::{PcmEvidenceConfig, pcm_evidence_stream};
    for (frames, flags, null_buffer, fails) in [
        (2, 0, false, false),
        (2, SILENT, true, false),
        (2, 0, true, true),
        (3, 0, false, true),
        (0, 0, true, false),
    ] {
        let samples = [0.25, -0.25, f32::from_bits(0x7fc0_1234), f32::NEG_INFINITY];
        let data = Rc::new(UnsafeCell::new(samples));
        // The fake ReleaseBuffer log has its capacity before the guarded lease.
        let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
        let client: IAudioCaptureClient = FakeCapture {
            data: data.clone(),
            releases: releases.clone(),
            frames,
            flags,
            null_buffer,
            fail_release: false,
        }
        .into();
        let (mut ingress, _source, _) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 2,
                detect_position_gaps: true,
            },
            48_000,
            2,
        )
        .unwrap();
        let (result, counts) = allocation_guard::count_allocations(|| {
            transfer_packet(
                &client,
                PacketBound::NativeCapacity(2),
                &mut ingress,
                None,
                Some(&mut writer),
            )
        });
        assert_eq!(counts, (0, 0));
        assert_eq!(result.is_err(), fails);
        let expected_release = [frames];
        assert_eq!(
            releases.borrow().as_slice(),
            if frames == 0 {
                &[][..]
            } else {
                &expected_release[..]
            }
        );
        // The lease has ended; replacing native memory must not change evidence.
        unsafe { *data.get() = [99.0; 4] };
        if fails || frames == 0 {
            assert!(reader.pop().is_none());
            assert_eq!(writer.snapshot().packets, 0);
        } else {
            for offset in 0..2 {
                let frame = reader.pop().unwrap();
                assert_eq!(frame.packet_frame_offset, offset as u32);
                assert_eq!(frame.qpc_100ns, 1);
                for channel in 0..2 {
                    let expected = if flags & SILENT != 0 {
                        0.0
                    } else {
                        samples[offset * 2 + channel]
                    };
                    assert_eq!(frame.samples[channel].to_bits(), expected.to_bits());
                }
            }
            assert!(reader.pop().is_none());
        }
    }
}

#[test]
fn process_bounded_packet_checks_non_silent_length_before_pointer_or_evidence() {
    for fail_release in [false, true] {
        let frames = (PROCESS_PACKET_SAFETY_BOUND_BYTES / 8 + 1) as u32;
        let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
        let client: IAudioCaptureClient = FakeCapture {
            data: Rc::new(UnsafeCell::new([0.0; 4])),
            releases: releases.clone(),
            frames,
            flags: 0,
            null_buffer: true,
            fail_release,
        }
        .into();
        let (mut ingress, _source, observer) =
            capture_bridge(ClockBridgeConfig::default()).unwrap();
        let (result, counts) = allocation_guard::count_allocations(|| {
            transfer_packet(
                &client,
                PacketBound::ProcessSafety,
                &mut ingress,
                None,
                None,
            )
        });
        assert_eq!(counts, (0, 0));
        assert_eq!(
            result.err(),
            Some(CaptureError::ProcessPacketBound {
                frames,
                frame_bytes: 8,
                packet_bytes: Some(frames as usize * 8),
                limit_bytes: PROCESS_PACKET_SAFETY_BOUND_BYTES,
                release_hresult: fail_release
                    .then_some(windows::Win32::Foundation::E_NOINTERFACE.0),
            })
        );
        assert_eq!(releases.borrow().as_slice(), [frames]);
        assert_eq!(observer.snapshot().captured_frames, 0);
    }
}

#[test]
fn process_bounded_packet_copies_real_non_silent_samples_without_allocating() {
    let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
    let client: IAudioCaptureClient = FakeCapture {
        data: Rc::new(UnsafeCell::new([0.25, -0.25, 0.5, -0.5])),
        releases: releases.clone(),
        frames: 2,
        flags: 0,
        null_buffer: false,
        fail_release: false,
    }
    .into();
    let (mut ingress, mut source, _) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: 2,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    let (result, counts) = allocation_guard::count_allocations(|| {
        transfer_packet(
            &client,
            PacketBound::ProcessSafety,
            &mut ingress,
            None,
            None,
        )
    });
    assert_eq!(counts, (0, 0));
    assert_eq!(result.unwrap().accepted_frames, 2);
    assert_eq!(releases.borrow().as_slice(), [2]);
    let mut pcm = [0.0; 2];
    assert_eq!(
        source
            .read_interleaved(&mut pcm)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(pcm, [0.25, -0.25]);
}
