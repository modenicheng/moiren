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

#[test]
fn prepared_bridge_keeps_independent_capacity_prime_servo_and_watermarks() {
    let prepared = moiren_windows_audio::latency::PreparedInputQueue {
        capacity_frames: 64,
        prime_frames: 8,
        servo_reference_frames: 16,
        low_water_frames: 4,
        high_water_frames: 48,
        modeled_minimum_frames: Some(8),
    };
    let config = ClockBridgeConfig {
        trim_on_prime: false,
        ..ClockBridgeConfig::with_prepared_input(48000, 2, prepared)
    };
    assert_eq!(config.capacity_frames, 64);
    assert_eq!(config.target_fill_frames, 8);
    let (mut ingress, mut source, observer) = capture_bridge(config).unwrap();
    let packet = vec![0u8; 16 * 2 * 4];
    ingress
        .push_packet(
            &packet,
            CapturePacket {
                frames: 16,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    assert_eq!(observer.snapshot().fill_frames, 16);
    assert_eq!(observer.snapshot().correction_ppm, 0.0);

    let (mut ingress, mut source, observer) = capture_bridge(config).unwrap();
    ingress
        .push_packet(
            &packet[..8 * 2 * 4],
            CapturePacket {
                frames: 8,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 2])
            .unwrap()
            .transferred_frames,
        1
    );
    assert!(observer.snapshot().correction_ppm < 0.0);

    for invalid in [
        moiren_windows_audio::latency::PreparedInputQueue {
            low_water_frames: 9,
            ..prepared
        },
        moiren_windows_audio::latency::PreparedInputQueue {
            high_water_frames: 16,
            ..prepared
        },
    ] {
        assert!(capture_bridge(ClockBridgeConfig::with_prepared_input(48000, 2, invalid)).is_err());
    }
    let mut mismatched = ClockBridgeConfig::with_prepared_input(48000, 2, prepared);
    mismatched.capacity_frames += 1;
    assert!(capture_bridge(mismatched).is_err());
}

#[test]
fn prepared_input_rejects_targets_below_the_modeled_minimum() {
    for (prime_frames, servo_reference_frames, minimum) in [(8, 16, 9), (16, 8, 9), (8, 16, 0)] {
        let prepared = moiren_windows_audio::latency::PreparedInputQueue {
            capacity_frames: 64,
            prime_frames,
            servo_reference_frames,
            low_water_frames: 4,
            high_water_frames: 48,
            modeled_minimum_frames: Some(minimum),
        };
        assert!(
            capture_bridge(ClockBridgeConfig::with_prepared_input(48000, 2, prepared)).is_err()
        );
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
    let payload = bytes(&ramp);
    input.push_packet(&payload, packet(6000, 0, 0)).unwrap();
    let mut out = [99.0; 512];
    assert_eq!(
        source
            .read_interleaved(&mut out[..2])
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(observer.snapshot().correction_ppm, 0.0);
    assert_eq!(
        source
            .read_interleaved(&mut out[2..])
            .unwrap()
            .transferred_frames,
        255
    );
    assert_eq!(out[0], (6000 - cfg.target_fill_frames) as f32);
    let snapshot = observer.snapshot();
    assert_eq!(
        snapshot.prime_discarded_frames,
        (6000 - cfg.target_fill_frames) as u64
    );
    let start = 6000 - cfg.target_fill_frames;
    let (mut baseline_input, mut baseline_source, baseline_observer) = capture_bridge(cfg).unwrap();
    baseline_input
        .push_packet(
            &payload[start * 8..],
            packet(cfg.target_fill_frames, start as u64, 0),
        )
        .unwrap();
    let mut baseline_output = [99.0; 512];
    baseline_source
        .read_interleaved(&mut baseline_output)
        .unwrap();
    assert_eq!(out, baseline_output);
    assert_eq!(
        snapshot.correction_ppm,
        baseline_observer.snapshot().correction_ppm
    );
    assert_eq!(baseline_observer.snapshot().prime_discarded_frames, 0);
    assert_eq!(snapshot.backlog_discarded_frames, 0);
    assert_eq!(snapshot.backlog_recoveries, 0);
}

fn recovery_config(trim_on_prime: bool, high_water_frames: u32) -> ClockBridgeConfig {
    ClockBridgeConfig {
        trim_on_prime,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::with_prepared_input(
            48000,
            2,
            moiren_windows_audio::latency::PreparedInputQueue {
                capacity_frames: 256,
                prime_frames: 64,
                servo_reference_frames: 64,
                low_water_frames: 16,
                high_water_frames,
                modeled_minimum_frames: Some(64),
            },
        )
    }
}

fn ramp_bytes(start: usize, end: usize) -> Vec<u8> {
    bytes(
        &(start..end)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect::<Vec<_>>(),
    )
}

fn running_recovery_bridge(
    config: ClockBridgeConfig,
) -> (CaptureIngress, ClockSource, BridgeObserver) {
    let (mut input, mut source, observer) = capture_bridge(config).unwrap();
    input
        .push_packet(&ramp_bytes(0, 64), packet(64, 0, 0))
        .unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 2])
            .unwrap()
            .transferred_frames,
        1
    );
    (input, source, observer)
}

#[test]
fn running_backlog_recovery_discards_old_prefix_and_fades_latest_inventory() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 128));
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let mut output = [0.0; 64];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 32);
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 0);
    for (index, samples) in output.as_chunks::<2>().0.iter().enumerate() {
        let expected = (192 + index) as f32 * (index + 1) as f32 / 32.0;
        assert_eq!(*samples, [expected, -expected]);
    }
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.resets, 1);
    assert_eq!(snapshot.dropped_frames, 0);
    assert_eq!(snapshot.discontinuities, 0);
    assert_eq!(snapshot.correction_ppm, 0.0);
    assert_eq!(snapshot.backlog_discarded_frames, 192);
    assert_eq!(snapshot.backlog_recoveries, 1);
    assert_eq!(snapshot.prime_discarded_frames, 0);
    assert_eq!(snapshot.fill_frames, 64);
    let mut next = [0.0; 2];
    assert!(!source.read_interleaved(&mut next).unwrap().discontinuity);
    assert_eq!(next, [224.0, -224.0]);
    assert_eq!(observer.snapshot().resets, 1);
    assert_eq!(observer.snapshot().backlog_recoveries, 1);
}

#[test]
fn running_backlog_high_water_is_inclusive_and_lower_fill_keeps_continuity() {
    for (incoming, expected, recovered) in [(63, 1.0, false), (64, 2.0, true)] {
        let (mut input, mut source, _) = running_recovery_bridge(recovery_config(true, 128));
        input
            .push_packet(&ramp_bytes(64, 64 + incoming), packet(incoming, 64, 0))
            .unwrap();
        let mut output = [0.0; 2];
        let report = source.read_interleaved(&mut output).unwrap();
        assert_eq!(report.transferred_frames, 1);
        assert_eq!(report.discontinuity, recovered);
        assert_eq!(report.xruns, 0);
        assert_eq!(output, [expected, -expected]);
    }
}

#[test]
fn running_backlog_offline_policy_preserves_every_queued_sample() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(false, 128));
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let mut output = [0.0; 64];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 32);
    assert!(!report.discontinuity);
    for (index, samples) in output.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(*samples, [(index + 1) as f32, -((index + 1) as f32)]);
    }
    assert_eq!(observer.snapshot().resets, 0);
    assert_eq!(observer.snapshot().dropped_frames, 0);
    assert_eq!(observer.snapshot().backlog_discarded_frames, 0);
    assert_eq!(observer.snapshot().backlog_recoveries, 0);
}

#[test]
fn running_backlog_one_frame_discard_retains_the_pending_interpolation_frame() {
    let (mut input, mut source, _) = running_recovery_bridge(recovery_config(true, 65));
    input
        .push_packet(&ramp_bytes(64, 65), packet(1, 64, 0))
        .unwrap();
    let mut output = [0.0; 64];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 32);
    assert!(report.discontinuity);
    for (index, samples) in output.as_chunks::<2>().0.iter().enumerate() {
        let expected = (index + 1) as f32 * (index + 1) as f32 / 32.0;
        assert_eq!(*samples, [expected, -expected]);
    }
}

#[test]
fn running_backlog_fade_spans_engine_segments_without_restarting() {
    let mut outputs = Vec::new();
    for segments in [&[32][..], &[1, 3, 5, 7, 16][..]] {
        let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 128));
        input
            .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
            .unwrap();
        let mut output = [0.0; 64];
        let mut offset = 0;
        for &frames in segments {
            let report = source
                .read_interleaved(&mut output[offset * 2..(offset + frames) * 2])
                .unwrap();
            assert_eq!(report.transferred_frames, frames);
            assert_eq!(report.discontinuity, offset == 0);
            offset += frames;
        }
        assert_eq!(observer.snapshot().resets, 1);
        outputs.push(output);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn running_backlog_recovery_never_interpolates_across_a_generation_boundary() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 128));
    input
        .push_packet(&bytes(&[1.0; 129 * 2]), packet(129, 64, 0))
        .unwrap();
    input
        .push_packet(&bytes(&[9.0; 63 * 2]), packet(63, 193, DISCONTINUITY))
        .unwrap();
    let mut output = [99.0; 16];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 0);
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 1);
    assert_eq!(output, [0.0; 16]);
    input
        .push_packet(&bytes(&[9.0; 4]), packet(2, 256, 0))
        .unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        8
    );
    assert_eq!(output, [9.0; 16]);
    assert_eq!(observer.snapshot().discontinuities, 1);
    assert_eq!(observer.snapshot().underrun_frames, 0);
}

#[test]
fn running_backlog_recovery_and_fade_allocate_and_free_nothing() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 128));
    let payload = ramp_bytes(64, 256);
    let mut output = [0.0; 64];
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    input.push_packet(&payload, packet(192, 64, 0)).unwrap();
    let report = source.read_interleaved(&mut output).unwrap();
    let snapshot = observer.snapshot();
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(report.transferred_frames, 32);
    assert!(report.discontinuity);
    assert_eq!(snapshot.resets, 1);
    assert_eq!(snapshot.backlog_discarded_frames, 192);
    assert_eq!(snapshot.backlog_recoveries, 1);
}

#[test]
fn running_backlog_recovers_to_servo_reference_independently_of_prime() {
    let mut config = recovery_config(true, 128);
    config
        .prepared_input
        .as_mut()
        .unwrap()
        .servo_reference_frames = 96;
    let (mut input, mut source, observer) = running_recovery_bridge(config);
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let mut output = [0.0; 2];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.xruns, 0);
    assert!(report.discontinuity);
    assert_eq!(output, [5.0, -5.0]);
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.fill_frames, 96);
    assert_eq!(snapshot.backlog_discarded_frames, 160);
    assert_eq!(snapshot.backlog_recoveries, 1);
    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(json["backlog_discarded_frames"], 160);
    assert_eq!(json["backlog_recoveries"], 1);
}

#[test]
fn running_backlog_resets_fractional_phase_and_fades_44100_hz_samples() {
    let config = ClockBridgeConfig {
        input_sample_rate: 44_100,
        ..recovery_config(true, 128)
    };
    let (mut input, mut source, observer) = running_recovery_bridge(config);
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let mut output = [0.0; 64];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 32);
    assert_eq!(report.xruns, 0);
    assert!(report.discontinuity);
    for (index, samples) in output.as_chunks::<2>().0.iter().enumerate() {
        let expected =
            ((192.0 + index as f64 * 44_100.0 / 48_000.0) * (index + 1) as f64 / 32.0) as f32;
        assert!((samples[0] - expected).abs() < 0.00003);
        assert!((samples[1] + expected).abs() < 0.00003);
    }
    assert_eq!(observer.snapshot().backlog_discarded_frames, 192);
    assert_eq!(observer.snapshot().backlog_recoveries, 1);
}

#[test]
fn running_backlog_discards_one_snapshot_across_ring_wrap() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 256));
    input
        .push_packet(&ramp_bytes(64, 192), packet(128, 64, 0))
        .unwrap();
    let report = source.read_interleaved(&mut [0.0; 256]).unwrap();
    assert_eq!(report.transferred_frames, 128);
    assert!(!report.discontinuity);
    input
        .push_packet(&ramp_bytes(192, 384), packet(192, 192, 0))
        .unwrap();
    let mut output = [0.0; 2];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 1);
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 0);
    assert_eq!(output, [10.0, -10.0]);
    assert_eq!(observer.snapshot().backlog_discarded_frames, 192);
    assert_eq!(observer.snapshot().backlog_recoveries, 1);
    assert_eq!(observer.snapshot().dropped_frames, 0);
}

#[test]
fn running_backlog_without_prepared_watermarks_keeps_legacy_continuity() {
    let config = ClockBridgeConfig {
        prepared_input: None,
        ..recovery_config(true, 128)
    };
    let (mut input, mut source, observer) = running_recovery_bridge(config);
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let mut output = [0.0; 2];
    let report = source.read_interleaved(&mut output).unwrap();
    assert_eq!(report.transferred_frames, 1);
    assert!(!report.discontinuity);
    assert_eq!(output, [1.0, -1.0]);
    assert_eq!(observer.snapshot().backlog_discarded_frames, 0);
    assert_eq!(observer.snapshot().backlog_recoveries, 0);
}

#[test]
fn running_backlog_recovery_clears_prior_controller_correction_once() {
    let mut config = recovery_config(true, 128);
    config.max_correction_ppm = 2000.0;
    config
        .prepared_input
        .as_mut()
        .unwrap()
        .servo_reference_frames = 96;
    let (mut input, mut source, observer) = running_recovery_bridge(config);
    assert!(observer.snapshot().correction_ppm < 0.0);
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let report = source.read_interleaved(&mut [0.0; 2]).unwrap();
    assert_eq!(report.transferred_frames, 1);
    assert_eq!(report.xruns, 0);
    assert!(report.discontinuity);
    assert_eq!(observer.snapshot().correction_ppm, 0.0);
    assert_eq!(observer.snapshot().resets, 1);
    assert_eq!(observer.snapshot().backlog_recoveries, 1);
}

#[test]
fn running_backlog_empty_demand_defers_recovery_without_moving_samples() {
    let (mut input, mut source, observer) = running_recovery_bridge(recovery_config(true, 128));
    input
        .push_packet(&ramp_bytes(64, 256), packet(192, 64, 0))
        .unwrap();
    let report = source.read_interleaved(&mut []).unwrap();
    assert_eq!(report.transferred_frames, 0);
    assert!(!report.discontinuity);
    assert_eq!(observer.snapshot().backlog_recoveries, 0);
    let mut output = [0.0; 2];
    assert!(source.read_interleaved(&mut output).unwrap().discontinuity);
    assert_eq!(output, [6.0, -6.0]);
    assert_eq!(observer.snapshot().backlog_discarded_frames, 192);
}

#[test]
fn low_water_controller_guard_allocates_and_frees_nothing() {
    let mut config = recovery_config(true, 240);
    config.max_correction_ppm = 2000.0;
    config.prepared_input.as_mut().unwrap().low_water_frames = 48;
    let (mut input, mut source, observer) = running_recovery_bridge(config);
    let payload = ramp_bytes(64, 192);
    let mut output = [0.0; 320];
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    input.push_packet(&payload, packet(128, 64, 0)).unwrap();
    let report = source.read_interleaved(&mut output).unwrap();
    let snapshot = observer.snapshot();
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(report.transferred_frames, 160);
    assert_eq!(report.xruns, 0);
    assert_eq!(snapshot.low_water_guard_frames, 1);
    assert_eq!(snapshot.correction_ppm, -2000.0);
}

#[test]
fn queued_packet_stamp_acceptance_and_query_allocate_and_free_nothing() {
    let (mut input, mut source, observer) = capture_bridge(config()).unwrap();
    input
        .push_packet(
            &[],
            CapturePacket {
                frames: 8,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 1_000_000,
            },
        )
        .unwrap();
    let mut output = [0.0; 16];
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    input
        .push_packet(
            &[],
            CapturePacket {
                frames: 8,
                flags: SILENT,
                device_position_frames: 8,
                qpc_100ns: 1_001_667,
            },
        )
        .unwrap();
    let report = source.read_interleaved(&mut output).unwrap();
    let stamp = source.oldest_frame_stamp().unwrap();
    let snapshot = observer.snapshot();
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(report.transferred_frames, 8);
    assert_eq!(report.xruns, 0);
    assert_eq!(stamp.packet_position_frames, 8);
    assert_eq!(stamp.packet_qpc_100ns, 1_001_667);
    assert_eq!(stamp.packet_offset_frames, 0);
    assert_eq!(snapshot.dropped_frames, 0);
}

#[test]
fn rate_clock_windows_and_feed_forward_allocate_and_free_nothing() {
    let input_queue = moiren_windows_audio::latency::PreparedInputQueue {
        capacity_frames: 8_192,
        prime_frames: 1_024,
        servo_reference_frames: 1_024,
        low_water_frames: 480,
        high_water_frames: 8_000,
        modeled_minimum_frames: None,
    };
    let (mut input, mut source, observer) = capture_bridge(ClockBridgeConfig::with_prepared_input(
        48_000,
        2,
        input_queue,
    ))
    .unwrap();
    let mut output = [0.0; 960];
    let mut estimate = None;
    let mut xruns = 0;
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|track| track.set(true));
    for index in 0..440 {
        input
            .push_packet(
                &[],
                CapturePacket {
                    frames: 480,
                    flags: SILENT,
                    device_position_frames: index * 480,
                    qpc_100ns: 1_000_000 + index * 100_000,
                },
            )
            .unwrap();
        xruns += source.read_interleaved(&mut output).unwrap().xruns;
        estimate = source
            .oldest_frame_stamp()
            .and_then(|stamp| stamp.rate_correction_ppm);
        let _ = observer.snapshot();
    }
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(estimate, Some(0));
    assert_eq!(xruns, 0);
}
