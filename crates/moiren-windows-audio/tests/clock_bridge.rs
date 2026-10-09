use moiren_windows_audio::{
    clock_bridge::*,
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

#[test]
fn virtual_zero_positions_do_not_create_gaps_but_native_flags_still_reset() {
    let (mut ingress, _source, observer) = capture_bridge(ClockBridgeConfig {
        detect_position_gaps: false,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    let bytes = [0u8; 480 * 2 * 4];
    for index in 0..5 {
        ingress
            .push_packet(
                &bytes,
                CapturePacket {
                    frames: 480,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1 + index * 100000,
                },
            )
            .unwrap();
    }
    assert_eq!(observer.snapshot().discontinuities, 0);
    ingress
        .push_packet(
            &bytes,
            CapturePacket {
                frames: 480,
                flags: DISCONTINUITY,
                device_position_frames: 0,
                qpc_100ns: 600001,
            },
        )
        .unwrap();
    assert_eq!(observer.snapshot().discontinuities, 1);
}

struct CountingAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|t| {
            if t.get() {
                let _ = COUNTS.try_with(|c| {
                    let (a, d) = c.get();
                    c.set((a + 1, d));
                });
            }
        });
        // SAFETY: Forward the caller's unchanged allocation contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACK.try_with(|t| {
            if t.get() {
                let _ = COUNTS.try_with(|c| {
                    let (a, d) = c.get();
                    c.set((a, d + 1));
                });
            }
        });
        // SAFETY: The allocation originated from this System allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn config() -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    }
}
fn packet(frames: usize, position: u64, flags: u32) -> CapturePacket {
    CapturePacket {
        frames,
        flags,
        device_position_frames: position,
        qpc_100ns: 1,
    }
}
fn bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn mono_maps_to_stereo_and_silent_packet_ignores_payload() {
    let (mut input, mut source, observer) = capture_bridge(ClockBridgeConfig {
        input_channels: 1,
        ..config()
    })
    .unwrap();
    input
        .push_packet(&bytes(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]), packet(6, 0, 0))
        .unwrap();
    let mut out = [99.0; 8];
    assert_eq!(
        source
            .read_interleaved(&mut out)
            .unwrap()
            .transferred_frames,
        4
    );
    assert_eq!(out, [1.0, 1.0, -1.0, -1.0, 2.0, 2.0, -2.0, -2.0]);
    input.push_packet(&[], packet(8, 6, SILENT)).unwrap();
    let mut tail = [99.0; 12];
    source.read_interleaved(&mut tail).unwrap();
    assert_eq!(&tail[4..], &[0.0; 8]);
    assert_eq!(observer.snapshot().silent_frames, 8);
}

#[test]
fn invalid_packets_and_empty_requests_do_not_consume_or_advance() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    assert!(input.push_packet(&[0; 7], packet(1, 0, 0)).is_err());
    assert!(source.read_interleaved(&mut [0.0; 3]).is_err());
    assert_eq!(
        source.read_interleaved(&mut []).unwrap().transferred_frames,
        0
    );
    assert_eq!(observer.snapshot().captured_frames, 0);
    input
        .push_packet(&bytes(&[1.0; 16]), packet(8, 0, 0))
        .unwrap();
    let mut out = [0.0; 4];
    source.read_interleaved(&mut out).unwrap();
    assert_eq!(out, [1.0; 4]);
}

#[test]
fn nominal_src_keeps_fractional_phase_across_variable_demands() {
    let (mut input, mut source, _) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: 44_100,
        ..config()
    })
    .unwrap();
    let ramp: Vec<f32> = (0..32).flat_map(|i| [i as f32, -(i as f32)]).collect();
    input.push_packet(&bytes(&ramp), packet(32, 0, 0)).unwrap();
    let mut index = 0;
    for frames in [1, 3, 11, 5] {
        let mut out = vec![99.0; frames * 2];
        assert_eq!(
            source
                .read_interleaved(&mut out)
                .unwrap()
                .transferred_frames,
            frames
        );
        for pair in out.as_chunks::<2>().0 {
            let expected = index as f32 * 44_100.0 / 48_000.0;
            assert!((pair[0] - expected).abs() < 0.00001);
            assert!((pair[1] + expected).abs() < 0.00001);
            index += 1;
        }
    }
}

#[test]
fn underrun_clears_tail_and_reprime_never_replays_old_samples() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    let mut out = [99.0; 8];
    assert_eq!(
        source
            .read_interleaved(&mut out)
            .unwrap()
            .transferred_frames,
        0
    );
    assert_eq!(out, [0.0; 8]);
    input
        .push_packet(&bytes(&[1.0; 12]), packet(6, 0, 0))
        .unwrap();
    let mut out = [99.0; 20];
    let report = source.read_interleaved(&mut out).unwrap();
    assert_eq!(report.transferred_frames, 5);
    assert!(report.discontinuity);
    assert_eq!(&out[10..], &[0.0; 10]);
    input
        .push_packet(&bytes(&[2.0; 16]), packet(8, 6, 0))
        .unwrap();
    let mut out = [0.0; 8];
    assert_eq!(
        source
            .read_interleaved(&mut out)
            .unwrap()
            .transferred_frames,
        4
    );
    assert_eq!(out, [2.0; 8]);
    assert_eq!(observer.snapshot().underrun_frames, 5);
}

#[test]
fn discontinuities_do_not_interpolate_across_generations() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    input
        .push_packet(&bytes(&[1.0; 12]), packet(6, 0, 0))
        .unwrap();
    input
        .push_packet(&bytes(&[2.0; 16]), packet(8, 6, DISCONTINUITY))
        .unwrap();
    let mut out = [99.0; 20];
    let report = source.read_interleaved(&mut out).unwrap();
    assert_eq!(report.transferred_frames, 5);
    assert_eq!(&out[..10], &[1.0; 10]);
    assert_eq!(&out[10..], &[0.0; 10]);
    let mut out = [0.0; 8];
    source.read_interleaved(&mut out).unwrap();
    assert_eq!(out, [2.0; 8]);
    assert_eq!(observer.snapshot().discontinuities, 1);
}

#[test]
fn overflow_timestamp_errors_nonfinite_samples_and_producer_exit_are_visible() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    assert_eq!(
        input
            .push_packet(&bytes(&[f32::NAN; 80]), packet(40, 0, TIMESTAMP_ERROR))
            .unwrap(),
        32
    );
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.dropped_frames, 8);
    assert_eq!(snapshot.timestamp_errors, 1);
    assert_eq!(snapshot.nonfinite_samples, 64);
    drop(input);
    let mut out = [99.0; 80];
    let report = source.read_interleaved(&mut out).unwrap();
    assert_eq!(report.transferred_frames, 31);
    assert_eq!(out, [0.0; 80]);
    assert!(observer.producer_finished());
}

#[test]
fn invalid_layouts_rates_controller_bounds_and_budget_are_rejected() {
    for bad in [
        ClockBridgeConfig {
            input_sample_rate: 96_000,
            ..config()
        },
        ClockBridgeConfig {
            input_channels: 3,
            ..config()
        },
        ClockBridgeConfig {
            target_fill_frames: 0,
            ..config()
        },
        ClockBridgeConfig {
            target_fill_frames: 32,
            ..config()
        },
        ClockBridgeConfig {
            max_correction_ppm: f64::NAN,
            ..config()
        },
        ClockBridgeConfig {
            max_correction_ppm: 10_000.0,
            ..config()
        },
        ClockBridgeConfig {
            byte_budget: 1,
            ..config()
        },
    ] {
        assert!(capture_bridge(bad).is_err());
    }
}

#[test]
fn adaptive_bridge_handles_both_drift_signs_and_packet_jitter() {
    let seconds = std::env::var("MOIREN_CLOCK_SIM_SECONDS")
        .ok()
        .map(|v| {
            v.parse::<usize>()
                .expect("simulation duration is an integer")
        })
        .unwrap_or(120);
    assert!((120..=7200).contains(&seconds));
    for sample_rate in [44_100, 48_000] {
        for drift_ppm in [-1000.0, 1000.0] {
            let cfg = ClockBridgeConfig {
                input_sample_rate: sample_rate,
                ..ClockBridgeConfig::default()
            };
            let (mut input, mut source, observer) = capture_bridge(cfg).unwrap();
            let payload = bytes(&[0.25; 2048]);
            let mut position = 0u64;
            input
                .push_packet(&payload, packet(1024, position, 0))
                .unwrap();
            position += 1024;
            input
                .push_packet(&payload, packet(1024, position, 0))
                .unwrap();
            position += 1024;
            let mut incoming = 0.0f64;
            let mut out = [0.0; 512];
            for block in 0..(seconds * 48_000 / 256) {
                incoming +=
                    256.0 * f64::from(sample_rate) / 48_000.0 * (1.0 + drift_ppm / 1_000_000.0);
                // A missing packet is delivered with the next one; mean rate is unchanged.
                if block % 7 != 0 {
                    let frames = incoming.floor() as usize;
                    incoming -= frames as f64;
                    input
                        .push_packet(&payload[..frames * 8], packet(frames, position, 0))
                        .unwrap();
                    position += frames as u64;
                }
                assert_eq!(
                    source
                        .read_interleaved(&mut out)
                        .unwrap()
                        .transferred_frames,
                    256
                );
            }
            let snapshot = observer.snapshot();
            assert_eq!(snapshot.underrun_frames, 0, "{drift_ppm}: {snapshot:?}");
            assert_eq!(snapshot.dropped_frames, 0);
            assert!(snapshot.correction_ppm * drift_ppm > 0.0, "{snapshot:?}");
            assert!(
                (snapshot.correction_ppm - drift_ppm).abs() < 300.0,
                "{snapshot:?}"
            );
            assert!(
                snapshot.fill_frames.abs_diff(cfg.target_fill_frames) < 800,
                "{snapshot:?}"
            );
            eprintln!(
                "native={sample_rate}, drift={drift_ppm} ppm, seconds={seconds}, fill={}, correction={:.2} ppm, underrun={}, dropped={}",
                snapshot.fill_frames,
                snapshot.correction_ppm,
                snapshot.underrun_frames,
                snapshot.dropped_frames
            );
        }
    }
}

#[test]
fn ingress_resampling_underrun_and_reset_allocate_and_free_nothing() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    let payload = bytes(&[0.25; 64]);
    let mut out = [99.0; 64];
    COUNTS.with(|c| c.set((0, 0)));
    TRACK.with(|t| t.set(true));
    input.push_packet(&payload, packet(32, 0, 0)).unwrap();
    source.read_interleaved(&mut out).unwrap();
    source.read_interleaved(&mut out).unwrap();
    input
        .push_packet(&payload, packet(32, 32, DISCONTINUITY))
        .unwrap();
    source.read_interleaved(&mut out).unwrap();
    let snapshot = observer.snapshot();
    TRACK.with(|t| t.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert!(snapshot.underrun_frames > 0);
}

#[test]
fn startup_backlog_is_trimmed_to_target_instead_of_winding_up_drift_controller() {
    let cfg = ClockBridgeConfig::default();
    let (mut input, mut source, observer) = capture_bridge(cfg).unwrap();
    let ramp: Vec<f32> = (0..6000).flat_map(|i| [i as f32, -(i as f32)]).collect();
    input
        .push_packet(&bytes(&ramp), packet(6000, 0, 0))
        .unwrap();
    let mut out = [99.0; 512];
    assert_eq!(
        source
            .read_interleaved(&mut out)
            .unwrap()
            .transferred_frames,
        256
    );
    assert_eq!(out[0], (6000 - cfg.target_fill_frames) as f32);
    let snapshot = observer.snapshot();
    assert_eq!(
        snapshot.prime_discarded_frames,
        (6000 - cfg.target_fill_frames) as u64
    );
    assert_eq!(snapshot.correction_ppm, 0.0);
}
