use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge, native_pcm::NativePcmConfig},
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

mod common;

fn packet(frames: usize, flags: u32) -> CapturePacket {
    CapturePacket {
        frames,
        flags,
        device_position_frames: 400,
        qpc_100ns: 900,
    }
}

#[test]
fn native_pcm_retains_raw_bits_and_exact_offered_accepted_frontiers() {
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        input_channels: 1,
        capacity_frames: 4,
        target_fill_frames: 2,
        detect_position_gaps: false,
        native_pcm: Some(NativePcmConfig {
            capacity_frames: 16,
        }),
        ..Default::default()
    })
    .unwrap();
    let mut reader = observer.take_native_pcm_reader().unwrap();
    assert!(observer.take_native_pcm_reader().is_none());
    let bits = [
        0x8000_0000u32,
        0x7fc0_1234,
        0x7f80_0000,
        0x3f80_0000,
        0xbf80_0000,
    ];
    let bytes: Vec<_> = bits.iter().flat_map(|bits| bits.to_le_bytes()).collect();
    assert_eq!(ingress.push_packet(&bytes, packet(5, 0)).unwrap(), 4);
    assert_eq!(ingress.push_packet(&[], packet(2, SILENT)).unwrap(), 0);
    for (offset, bits) in bits.into_iter().enumerate() {
        let frame = reader.pop().unwrap();
        assert_eq!(frame.sample_bits, [bits, 0]);
        assert_eq!(frame.packet_sequence, 1);
        assert_eq!(frame.packet_frame_offset, offset as u32);
        assert_eq!(frame.packet_frames, 5);
        assert_eq!(frame.packet_accepted_frames, 4);
        assert_eq!(frame.offered_before_frames, 0);
        assert_eq!(frame.accepted_before_frames, 0);
        assert_eq!(frame.generation, 1);
        assert_eq!(frame.device_position_frames, 400);
        assert_eq!(frame.qpc_100ns, 900);
    }
    for offset in 0..2 {
        let frame = reader.pop().unwrap();
        assert_eq!(frame.sample_bits, [0, 0]);
        assert_eq!(frame.packet_sequence, 2);
        assert_eq!(frame.packet_frame_offset, offset);
        assert_eq!(frame.offered_before_frames, 5);
        assert_eq!(frame.accepted_before_frames, 4);
        assert_eq!(frame.packet_accepted_frames, 0);
        assert_eq!(frame.generation, 2);
        assert_eq!(frame.flags, SILENT);
    }
    assert!(!reader.is_finished());
    drop(ingress);
    assert!(reader.is_finished());
    let summary = reader.snapshot();
    assert_eq!(summary.offered_frames, 7);
    assert_eq!(summary.accepted_frames, 4);
    assert_eq!(summary.stored_frames, 7);
    assert_eq!(summary.lost_frames, 0);
    assert!(summary.writer_finished);
    drop(source);
}

#[test]
fn native_pcm_full_or_abandoned_never_changes_audio_or_allocates() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 2,
        trim_on_prime: false,
        detect_position_gaps: false,
        native_pcm: Some(NativePcmConfig { capacity_frames: 2 }),
        ..Default::default()
    })
    .unwrap();
    let mut reader = observer.take_native_pcm_reader().unwrap();
    common::assert_allocates_nothing(|| {
        assert_eq!(ingress.push_packet(&[0; 16], packet(2, 0)).unwrap(), 2);
        assert_eq!(ingress.push_packet(&[0; 24], packet(3, 0)).unwrap(), 3);
    });
    assert_eq!(reader.snapshot().lost_frames, 3);
    assert_eq!(reader.snapshot().overflow_packets, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    common::assert_allocates_nothing(|| {
        assert_eq!(ingress.push_packet(&[0; 8], packet(1, 0)).unwrap(), 1);
    });
    let frame = reader.pop().unwrap();
    assert_eq!(frame.packet_sequence, 3);
    assert_eq!(frame.offered_before_frames, 5);
    assert_eq!(frame.accepted_before_frames, 5);
    drop(reader);
    common::assert_allocates_nothing(|| {
        assert_eq!(ingress.push_packet(&[0; 16], packet(2, 0)).unwrap(), 2);
        assert_eq!(
            source
                .read_interleaved(&mut [0.0; 8])
                .unwrap()
                .transferred_frames,
            4
        );
    });
    assert_eq!(observer.snapshot().written_frames, 8);
    assert_eq!(observer.snapshot().dropped_frames, 0);
}

#[test]
fn native_pcm_invalid_packet_does_not_advance_and_flags_preserve_generation() {
    let (mut ingress, _, observer) = capture_bridge(ClockBridgeConfig {
        native_pcm: Some(NativePcmConfig { capacity_frames: 4 }),
        ..Default::default()
    })
    .unwrap();
    let mut reader = observer.take_native_pcm_reader().unwrap();
    assert!(ingress.push_packet(&[0; 7], packet(1, 0)).is_err());
    assert_eq!(reader.snapshot().packets, 0);
    ingress.push_packet(&[], packet(0, 0)).unwrap();
    let flags = DISCONTINUITY | TIMESTAMP_ERROR | SILENT;
    ingress.push_packet(&[], packet(1, flags)).unwrap();
    drop(ingress);
    assert!(!reader.is_finished(), "closure does not erase pending PCM");
    let frame = reader.pop().unwrap();
    assert_eq!(frame.packet_sequence, 1);
    assert_eq!(frame.flags, flags);
    assert_eq!(frame.generation, 2);
    assert!(reader.is_finished());
}

#[test]
fn native_pcm_disabled_and_invalid_config_are_explicit() {
    let (_, _, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    assert!(observer.take_native_pcm_reader().is_none());
    for capacity_frames in [0, usize::MAX, 8 * 1024 * 1024] {
        common::assert_allocates_nothing(|| {
            assert!(NativePcmConfig { capacity_frames }.validate().is_err());
            assert!(
                capture_bridge(ClockBridgeConfig {
                    native_pcm: Some(NativePcmConfig { capacity_frames }),
                    ..Default::default()
                })
                .is_err()
            );
        });
    }
}

#[test]
fn native_pcm_tap_larger_than_packet_budget_refuses_whole_packet_not_audio() {
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 64,
        target_fill_frames: 4,
        native_pcm: Some(NativePcmConfig { capacity_frames: 3 }),
        ..Default::default()
    })
    .unwrap();
    let mut reader = observer.take_native_pcm_reader().unwrap();
    common::assert_allocates_nothing(|| {
        assert_eq!(ingress.push_packet(&[], packet(8, SILENT)).unwrap(), 8);
    });
    drop(ingress);
    assert!(reader.pop().is_none());
    assert!(reader.is_finished());
    let summary = reader.snapshot();
    assert_eq!(summary.offered_frames, 8);
    assert_eq!(summary.accepted_frames, 8);
    assert_eq!(summary.stored_frames, 0);
    assert_eq!(summary.lost_frames, 8);
    assert_eq!(summary.overflow_packets, 1);
    assert_eq!(observer.snapshot().dropped_frames, 0);
    drop(source);
}

fn replay(
    frames: &[moiren_windows_audio::clock_bridge::native_pcm::NativePcmFrame],
    channels: usize,
    spans: &[moiren_windows_audio::latency::interpolation::InterpolationEvent],
    requested: usize,
) -> Vec<f32> {
    let mut accepted = Vec::new();
    for &frame in frames {
        if let Some(samples) = frame.accepted_samples(channels) {
            assert_eq!(
                frame.accepted_before_frames + u64::from(frame.packet_frame_offset),
                accepted.len() as u64
            );
            accepted.push((samples, frame.generation));
        }
    }
    let mut replay = vec![0.0; requested * 2];
    for event in spans {
        let span = event.span;
        let mut phase = span.phase;
        let mut input = span.input_start_frame as usize;
        let mut fade = span.fade_remaining;
        for offset in 0..span.frames as usize {
            while phase >= 1.0 {
                phase -= 1.0;
                input += 1;
            }
            assert_eq!(accepted[input].1, span.generation);
            assert_eq!(accepted[input + 1].1, span.generation);
            let gain = if fade > 0 {
                let g = f64::from(33 - fade) / 32.0;
                fade -= 1;
                g
            } else {
                1.0
            };
            for channel in 0..2 {
                let value = (f64::from(accepted[input].0[channel]) * (1.0 - phase)
                    + f64::from(accepted[input + 1].0[channel]) * phase)
                    * gain;
                replay[(span.output_start_frame as usize + offset) * 2 + channel] = value as f32;
            }
            phase += span.step;
        }
    }
    replay
}

#[test]
fn native_pcm_samples_match_actual_src_across_dropped_native_tails() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        input_channels: 1,
        input_sample_rate: 44_100,
        capacity_frames: 64,
        target_fill_frames: 4,
        detect_position_gaps: false,
        trim_on_prime: false,
        native_pcm: Some(NativePcmConfig {
            capacity_frames: 256,
        }),
        ..Default::default()
    })
    .unwrap();
    let mut reader = observer.take_native_pcm_reader().unwrap();
    let bytes: Vec<_> = (0..80u32)
        .flat_map(|index| {
            if index == 1 {
                0x7fc0_1234u32.to_le_bytes()
            } else {
                (index as f32 * 0.002 - 0.2).to_le_bytes()
            }
        })
        .collect();
    ingress.push_packet(&bytes, packet(80, 0)).unwrap();
    let mut audio = vec![0.0; 74];
    source.read_interleaved(&mut audio).unwrap();
    ingress.push_packet(&bytes[..80], packet(20, 0)).unwrap();
    let mut tail = vec![0.0; 40];
    source.read_interleaved(&mut tail).unwrap();
    audio.extend(tail);
    let mut frames = Vec::new();
    while let Some(frame) = reader.pop() {
        frames.push(frame);
    }
    assert_eq!(
        replay(
            &frames,
            1,
            &observer.interpolation_history().unwrap().events,
            audio.len() / 2
        ),
        audio,
        "actual accepted PCM and cursor recurrence must reconstruct samples exactly"
    );
    assert!(observer.snapshot().dropped_frames > 0);
}

#[test]
fn native_pcm_replays_backlog_fade_silence_and_generation_boundaries() {
    use moiren_windows_audio::latency::PreparedInputQueue;
    let config = ClockBridgeConfig {
        native_pcm: Some(NativePcmConfig {
            capacity_frames: 512,
        }),
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::with_prepared_input(
            48_000,
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
    let mut reader = observer.take_native_pcm_reader().unwrap();
    let bytes: Vec<_> = (0..128)
        .flat_map(|i| {
            [i as f32 / 256.0, -0.3]
                .into_iter()
                .flat_map(f32::to_le_bytes)
        })
        .collect();
    ingress.push_packet(&bytes[..512], packet(64, 0)).unwrap();
    let mut audio = vec![0.0; 2];
    source.read_interleaved(&mut audio).unwrap();
    ingress.push_packet(&bytes, packet(128, 0)).unwrap();
    let mut block = vec![0.0; 128];
    source.read_interleaved(&mut block).unwrap();
    audio.extend(block);
    ingress
        .push_packet(&[], packet(64, SILENT | DISCONTINUITY))
        .unwrap();
    for _ in 0..3 {
        let mut block = vec![0.0; 64];
        source.read_interleaved(&mut block).unwrap();
        audio.extend(block);
    }
    drop(ingress);
    let mut frames = Vec::new();
    while let Some(frame) = reader.pop() {
        frames.push(frame);
    }
    let history = observer.interpolation_history().unwrap();
    assert!(
        history
            .events
            .iter()
            .any(|event| event.span.fade_remaining == 32)
    );
    assert!(
        history
            .events
            .iter()
            .any(|event| event.span.generation == 2)
    );
    assert_eq!(replay(&frames, 2, &history.events, audio.len() / 2), audio);
    assert_eq!(reader.snapshot().lost_frames, 0);
    assert!(reader.is_finished());
}
