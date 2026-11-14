use moiren_windows_audio::clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge};

mod common;

fn signal(frame: u64, channel: usize) -> f32 {
    let value = frame
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let value = value.rotate_left(channel as u32 * 17);
    ((value >> 40) as i32 - (1 << 23)) as f32 / (1 << 24) as f32
}

fn push(ingress: &mut moiren_windows_audio::clock_bridge::CaptureIngress, frames: usize) {
    let samples: Vec<u8> = (0..frames)
        .flat_map(|frame| {
            [signal(frame as u64, 0), signal(frame as u64, 1)]
                .into_iter()
                .flat_map(f32::to_le_bytes)
        })
        .collect();
    ingress
        .push_packet(
            &samples,
            CapturePacket {
                frames,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
}

#[test]
fn actual_interpolation_spans_reconstruct_each_sample_across_read_and_servo_boundaries() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 2048,
        target_fill_frames: 32,
        trim_on_prime: false,
        detect_position_gaps: false,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    push(&mut ingress, 1024);
    let mut audio = Vec::new();
    for frames in [17, 43, 5, 64, 31, 32, 33, 1, 3, 5, 7, 16] {
        let mut block = vec![0.0; frames * 2];
        assert_eq!(
            source
                .read_interleaved(&mut block)
                .unwrap()
                .transferred_frames,
            frames
        );
        audio.extend(block);
    }
    // Use serialized native diagnostics: missing observations must fail as
    // missing evidence, rather than accepting an affine waveform fit.
    let diagnostics = serde_json::to_value(observer.interpolation_history().unwrap()).unwrap();
    let original = observer.interpolation_history().unwrap();
    let encoded = serde_json::to_string(&original).unwrap();
    let decoded: moiren_windows_audio::latency::interpolation::InterpolationHistorySnapshot =
        serde_json::from_str(&encoded).unwrap();
    for (before, after) in original.events.iter().zip(&decoded.events) {
        assert_eq!(
            before.span.phase.to_bits(),
            after.span.phase.to_bits(),
            "phase must survive actual JSON text export"
        );
        assert_eq!(
            before.span.step.to_bits(),
            after.span.step.to_bits(),
            "step must survive actual JSON text export"
        );
    }
    let spans = diagnostics["events"]
        .as_array()
        .expect("actual interpolation observations are missing");
    let mut reconstructed = Vec::new();
    for event in spans {
        let span = &event["span"];
        let frames = span["frames"].as_u64().unwrap() as usize;
        assert!((1..=32).contains(&frames));
        assert_eq!(
            span["output_start_frame"].as_u64().unwrap() as usize,
            reconstructed.len() / 2
        );
        assert_eq!(span["fade_remaining"].as_u64(), Some(0));
        let input = span["input_start_frame"].as_u64().unwrap();
        let phase = f64::from_bits(span["phase_bits"].as_str().unwrap().parse().unwrap());
        let step = f64::from_bits(span["step_bits"].as_str().unwrap().parse().unwrap());
        assert!(
            step > 1.0,
            "the actual nonzero servo correction is retained"
        );
        // Replay the recorded cursor, not a second invocation of ClockSource.
        let mut cursor = phase;
        let mut whole = input;
        for _ in 0..frames {
            while cursor >= 1.0 {
                cursor -= 1.0;
                whole += 1;
            }
            for channel in 0..2 {
                let a = f64::from(signal(whole, channel));
                let b = f64::from(signal(whole + 1, channel));
                reconstructed.push((a * (1.0 - cursor) + b * cursor) as f32);
            }
            cursor += step;
        }
    }
    assert_eq!(reconstructed, audio);
}

#[test]
fn opposite_extreme_finite_samples_replay_the_weighted_sum_without_subtraction_overflow() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: 44100,
        capacity_frames: 256,
        target_fill_frames: 4,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    let pcm: Vec<[f32; 2]> = (0..128)
        .map(|index| {
            let value = if index % 2 == 0 { f32::MAX } else { -f32::MAX };
            [value, -value]
        })
        .collect();
    let bytes: Vec<u8> = pcm
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    ingress
        .push_packet(
            &bytes,
            CapturePacket {
                frames: pcm.len(),
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    let mut actual = [0.0; 128];
    source.read_interleaved(&mut actual).unwrap();
    let history = observer.interpolation_history().unwrap();
    for event in history.events {
        let span = event.span;
        let mut phase = span.phase;
        let mut index = span.input_start_frame as usize;
        for offset in 0..span.frames as usize {
            while phase >= 1.0 {
                phase -= 1.0;
                index += 1;
            }
            for (channel, (&a, &b)) in pcm[index].iter().zip(&pcm[index + 1]).enumerate() {
                let expected = (f64::from(a) * (1.0 - phase) + f64::from(b) * phase) as f32;
                let output = (span.output_start_frame as usize + offset) * 2 + channel;
                assert!(actual[output].is_finite());
                assert_eq!(actual[output], expected);
            }
            phase += span.step;
        }
    }
}

#[test]
fn priming_and_underrun_tails_remain_gaps_in_the_requested_output_timeline() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 10])
            .unwrap()
            .transferred_frames,
        0
    );
    push(&mut ingress, 8);
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 24])
            .unwrap()
            .transferred_frames,
        7
    );
    push(&mut ingress, 8);
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 6])
            .unwrap()
            .transferred_frames,
        3
    );
    source.read_interleaved(&mut [0.0; 24]).unwrap();
    source.read_interleaved(&mut [0.0; 10]).unwrap();
    let history = observer.interpolation_history().unwrap();
    assert!(!history.saturated);
    assert_eq!(history.requested_frames, 37);
    assert_eq!((history.started_reads, history.completed_reads), (5, 5));
    assert_eq!(history.events.len(), 3);
    let a = history.events[0].span;
    let b = history.events[1].span;
    assert_eq!((a.read_sequence, a.output_start_frame, a.frames), (2, 5, 7));
    assert_eq!(
        (b.read_sequence, b.output_start_frame, b.frames),
        (3, 17, 3)
    );
    assert_eq!((a.input_start_frame, b.input_start_frame), (0, 8));
    assert!(!history.writer_finished);
    drop(source);
    assert!(observer.interpolation_history().unwrap().writer_finished);
}

#[test]
fn downsampling_splits_actual_pair_generation_skips_without_resetting_audio() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: 96000,
        output_sample_rate: 48000,
        capacity_frames: 64,
        target_fill_frames: 2,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    push(&mut ingress, 2);
    ingress
        .push_packet(
            &[0; 64],
            CapturePacket {
                frames: 8,
                flags: moiren_windows_audio::stats::DISCONTINUITY,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    source.read_interleaved(&mut [0.0; 6]).unwrap();
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.events.len(), 2);
    assert_eq!(history.events[0].span.frames, 1);
    assert_eq!(history.events[1].span.frames, 2);
    assert_eq!(history.events[1].span.input_start_frame, 2);
    assert_eq!(history.events[1].span.output_start_frame, 1);
    assert_ne!(
        history.events[0].span.generation,
        history.events[1].span.generation
    );
    assert_eq!(observer.snapshot().resets, 0);
}

#[test]
fn diagnostic_frontiers_exclude_empty_invalid_and_unmade_reads_without_extra_clock_calls() {
    let (_ingress, mut source, observer) = capture_bridge(Default::default()).unwrap();
    let mut calls = 0;
    source
        .read_interleaved_with_publication_clock(&mut [], || {
            calls += 1;
            Some(1)
        })
        .unwrap();
    assert!(
        source
            .read_interleaved_with_publication_clock(&mut [0.0], || {
                calls += 1;
                Some(1)
            })
            .is_err()
    );
    assert_eq!(calls, 0);
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 66], || {
            calls += 1;
            Some(1)
        })
        .unwrap();
    assert_eq!(calls, 2);
    let history = observer.interpolation_history().unwrap();
    assert!(history.events.is_empty());
    assert_eq!(history.requested_frames, 33);
    assert_eq!((history.started_reads, history.completed_reads), (1, 1));
}

#[test]
fn startup_trim_and_unequal_sample_rates_keep_native_input_indices() {
    for (input_sample_rate, output_sample_rate) in [(44100, 96000), (96000, 44100), (44100, 48000)]
    {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            input_sample_rate,
            output_sample_rate,
            capacity_frames: 2048,
            target_fill_frames: 1024,
            max_correction_ppm: 0.0,
            ..Default::default()
        })
        .unwrap();
        push(&mut ingress, 1536);
        let mut actual = [0.0; 64];
        assert_eq!(
            source
                .read_interleaved(&mut actual)
                .unwrap()
                .transferred_frames,
            32
        );
        let history = observer.interpolation_history().unwrap();
        assert_eq!(history.events.len(), 1);
        let span = history.events[0].span;
        assert_eq!(span.input_start_frame, 512);
        assert_eq!(span.phase, 0.0);
        assert_eq!(
            span.step,
            f64::from(input_sample_rate) / f64::from(output_sample_rate)
        );
        let mut phase = 0.0;
        let mut index = 512;
        for pair in actual.as_chunks::<2>().0 {
            while phase >= 1.0 {
                phase -= 1.0;
                index += 1;
            }
            for (channel, actual) in pair.iter().enumerate() {
                let expected = (f64::from(signal(index, channel)) * (1.0 - phase)
                    + f64::from(signal(index + 1, channel)) * phase)
                    as f32;
                assert_eq!(*actual, expected);
            }
            phase += span.step;
        }
    }
}

#[test]
fn recovery_fade_and_generation_boundaries_are_observed_without_cross_interpolation() {
    use moiren_windows_audio::latency::PreparedInputQueue;
    let config = ClockBridgeConfig {
        max_correction_ppm: 0.0,
        detect_position_gaps: false,
        ..ClockBridgeConfig::with_prepared_input(
            48000,
            2,
            PreparedInputQueue {
                capacity_frames: 256,
                prime_frames: 64,
                servo_reference_frames: 64,
                low_water_frames: 16,
                high_water_frames: 128,
                modeled_minimum_frames: Some(64),
            },
        )
    };
    let (mut ingress, mut source, observer) = capture_bridge(config).unwrap();
    push(&mut ingress, 64);
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    push(&mut ingress, 128);
    let mut output = [0.0; 64];
    assert_eq!(
        source
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        32
    );
    let history = observer.interpolation_history().unwrap();
    let span = history.events.last().unwrap().span;
    assert_eq!(span.fade_remaining, 32);
    assert_eq!(span.input_start_frame, 128);
    for (frame, pair) in output.as_chunks::<2>().0.iter().enumerate() {
        // The second push starts a fresh deterministic signal at accepted
        // index64. Recovery drops to accepted128, so source signal index64.
        for (channel, actual) in pair.iter().enumerate() {
            let expected =
                (f64::from(signal(64 + frame as u64, channel)) * (frame + 1) as f64 / 32.0) as f32;
            assert_eq!(*actual, expected);
        }
    }
    assert_eq!(observer.snapshot().backlog_recoveries, 1);

    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 64,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    push(&mut ingress, 8);
    ingress
        .push_packet(
            &[0; 64],
            CapturePacket {
                frames: 8,
                flags: moiren_windows_audio::stats::DISCONTINUITY,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 32])
            .unwrap()
            .transferred_frames,
        7
    );
    source.read_interleaved(&mut [0.0; 4]).unwrap();
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.events.len(), 2);
    assert_ne!(
        history.events[0].span.generation,
        history.events[1].span.generation
    );
    assert_eq!(history.events[1].span.input_start_frame, 9);
    assert_eq!(history.events[1].span.output_start_frame, 16);
    assert!(!history.saturated);
}

#[test]
fn bounded_suffix_publication_overwrite_and_recovery_allocate_and_free_nothing() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 2048,
        target_fill_frames: 32,
        trim_on_prime: false,
        ..Default::default()
    })
    .unwrap();
    push(&mut ingress, 1024);
    common::assert_allocates_nothing(|| {
        for _ in 0..600 {
            source.read_interleaved(&mut [0.0; 2]).unwrap();
        }
        source.read_interleaved(&mut [0.0; 1024]).unwrap();
    });
    let history = observer.interpolation_history().unwrap();
    assert!(!history.saturated);
    assert_eq!(history.events.len(), 256);
    assert!(history.overwritten_events > 0);
    assert_eq!(
        history.events.last().unwrap().sequence,
        history.total_events
    );
    assert_eq!(history.events[0].sequence, history.overwritten_events + 1);
    assert!(observer.snapshot().underrun_frames > 0);
}
