use super::*;
use crate::{
    buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    signal::ProcessSignals,
};
use serde::de::value::{Error as ValueError, MapDeserializer};
use std::mem::size_of;

pub(super) fn settings(capacity_frames: usize) -> RecorderSettings {
    RecorderSettings { capacity_frames }
}

pub(super) fn observe<S: ProcessingSample>(recorder: &mut Recorder, channels: &[&[f64]]) {
    let frames = channels[0].len();
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels: channels.len(),
            capacity_frames: frames,
        }],
        channels.len() * frames * size_of::<S>(),
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    arena
        .with_io(&write, 0..frames, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!("expected separate IO");
            };
            let mut block = outputs.get_mut(0).unwrap();
            for (destination, source) in block.channels_mut().zip(channels) {
                assert_eq!(destination.len(), source.len());
                for (destination, &source) in destination.iter_mut().zip(*source) {
                    *destination = S::from_f64(source);
                }
            }
        })
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    RtObserver::<S>::validate_inputs(recorder, &read).unwrap();
    arena
        .with_io(&read, 0..frames, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                panic!("expected read-only IO");
            };
            recorder.observe(
                &ProcessContext {
                    timeline_epoch: 1,
                    timeline_start: 0,
                    frames,
                    processing_sr: 48_000.0,
                },
                inputs,
                ProcessSignals::empty(&mut ()),
            );
        })
        .unwrap();
}

#[test]
fn settings_default_and_serde_contract() {
    fn require_serde<
        T: std::fmt::Debug
            + Clone
            + Copy
            + PartialEq
            + serde::Serialize
            + for<'de> serde::Deserialize<'de>,
    >() {
    }
    require_serde::<RecorderSettings>();
    let default = RecorderSettings::default();
    assert_eq!(default.capacity_frames, 48_000);
    assert!(default.valid());

    let empty: [(&str, usize); 0] = [];
    let decoded =
        RecorderSettings::deserialize(MapDeserializer::<_, ValueError>::new(empty.into_iter()))
            .unwrap();
    assert_eq!(decoded, default);
    let decoded = RecorderSettings::deserialize(MapDeserializer::<_, ValueError>::new(
        [("capacity_frames", 12)].into_iter(),
    ))
    .unwrap();
    assert_eq!(decoded, settings(12));
    assert!(
        RecorderSettings::deserialize(MapDeserializer::<_, ValueError>::new(
            [("typo", 12usize)].into_iter(),
        ))
        .is_err()
    );
}

#[test]
fn rejects_invalid_channels_capacity_and_excessive_memory_before_allocating() {
    for channels in [0, 257, usize::MAX] {
        assert!(matches!(
            recorder(channels, settings(1)),
            Err(ProcessorError::InvalidIo)
        ));
    }
    for capacity in [0, RecorderSettings::MAX_CAPACITY_FRAMES + 1, usize::MAX] {
        assert!(!settings(capacity).valid());
        assert!(matches!(
            recorder(1, settings(capacity)),
            Err(ProcessorError::InvalidIo)
        ));
    }
    assert!(matches!(
        recorder(256, settings(RecorderSettings::MAX_CAPACITY_FRAMES)),
        Err(ProcessorError::InvalidIo)
    ));
    let (_, reader) = recorder(256, settings(1)).unwrap();
    assert_eq!(reader.channel_count(), 256);
    assert_eq!(reader.capacity_frames(), 1);
}

#[test]
fn validates_exactly_one_read_only_input_with_matching_channels() {
    let (recorder, _) = recorder(2, settings(4)).unwrap();
    let arena = BufferArena::<f32>::new(
        &[
            BufferSlotLayout {
                channels: 2,
                capacity_frames: 4,
            },
            BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            },
        ],
        4096,
    )
    .unwrap();
    let stereo = arena.slot(0).unwrap();
    let mono = arena.slot(1).unwrap();
    for ports in [
        vec![],
        vec![PortAccess::Read {
            port: 1,
            slot: stereo,
        }],
        vec![PortAccess::Read {
            port: 0,
            slot: mono,
        }],
        vec![
            PortAccess::Read {
                port: 0,
                slot: stereo,
            },
            PortAccess::Read {
                port: 1,
                slot: mono,
            },
        ],
        vec![
            PortAccess::Read {
                port: 0,
                slot: stereo,
            },
            PortAccess::Write {
                port: 0,
                slot: mono,
            },
        ],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: stereo,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtObserver::<f32>::validate_inputs(&recorder, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
}

fn exact_pcm<S: ProcessingSample>() {
    let (mut recorder, mut reader) = recorder(2, settings(8)).unwrap();
    let left = [0.0, -0.0, 1.0 / 3.0, f64::INFINITY];
    let right = [2.0, -3.5, 0.123_456_789, f64::NEG_INFINITY];
    observe::<S>(&mut recorder, &[&left, &right]);
    let mut output = [99.0; 11];
    assert_eq!(reader.read_interleaved(&mut output), 4);
    for frame in 0..4 {
        assert_eq!(output[frame * 2].to_bits(), (left[frame] as f32).to_bits());
        assert_eq!(
            output[frame * 2 + 1].to_bits(),
            (right[frame] as f32).to_bits()
        );
    }
    assert_eq!(&output[8..], &[99.0; 3]);
    assert_eq!(reader.read_interleaved(&mut output), 0);
    assert_eq!(
        reader.stats(),
        RecorderStats {
            accepted_frames: 4,
            read_frames: 4,
            dropped_frames: 0,
            state: RecorderState::Recording,
        }
    );
}

#[test]
fn records_exact_interleaved_f32_without_clipping() {
    exact_pcm::<f32>();
}

#[test]
fn converts_f64_to_exact_interleaved_f32_without_clipping() {
    exact_pcm::<f64>();
}

#[test]
fn overflow_drops_whole_blocks_and_queue_wrap_keeps_channel_alignment() {
    let (mut recorder, mut reader) = recorder(2, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0, 3.0], &[11.0, 12.0, 13.0]]);
    observe::<f32>(&mut recorder, &[&[90.0, 91.0], &[99.0, 98.0]]);
    let mut prefix = [0.0; 4];
    assert_eq!(reader.read_interleaved(&mut prefix), 2);
    assert_eq!(prefix, [1.0, 11.0, 2.0, 12.0]);
    observe::<f32>(&mut recorder, &[&[4.0, 5.0, 6.0], &[14.0, 15.0, 16.0]]);
    let mut output = [0.0; 8];
    assert_eq!(reader.read_interleaved(&mut output), 4);
    assert_eq!(output, [3.0, 13.0, 4.0, 14.0, 5.0, 15.0, 6.0, 16.0]);
    observe::<f32>(&mut recorder, &[&[0.0; 5], &[1.0; 5]]);
    assert_eq!(reader.queued_frames(), 0);
    assert_eq!(reader.stats().accepted_frames, 6);
    assert_eq!(reader.stats().read_frames, 6);
    assert_eq!(reader.stats().dropped_frames, 7);
}

#[test]
fn stop_start_preserves_queued_audio_and_does_not_count_stopped_frames_as_drops() {
    let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
    assert_eq!(reader.state(), RecorderState::Recording);
    assert!(reader.is_recording());
    observe::<f64>(&mut recorder, &[&[1.0]]);
    reader.stop();
    reader.stop();
    assert_eq!(reader.state(), RecorderState::Stopped);
    assert!(!reader.is_recording());
    observe::<f64>(&mut recorder, &[&[2.0, 3.0]]);
    reader.start();
    observe::<f64>(&mut recorder, &[&[4.0]]);
    let mut output = [0.0; 4];
    assert_eq!(reader.read_interleaved(&mut output), 2);
    assert_eq!(output, [1.0, 4.0, 0.0, 0.0]);
    assert_eq!(reader.stats().accepted_frames, 2);
    assert_eq!(reader.stats().dropped_frames, 0);
}

#[test]
fn reads_only_complete_frames_and_reports_disconnection_without_discarding_audio() {
    let (mut recorder, mut reader) = recorder(2, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0], &[3.0, 4.0]]);
    drop(recorder);
    assert_eq!(reader.state(), RecorderState::Disconnected);
    assert!(!reader.is_recording());
    assert_eq!(reader.read_interleaved(&mut [0.0]), 0);
    assert_eq!(reader.queued_frames(), 2);
    let mut output = [9.0; 3];
    assert_eq!(reader.read_interleaved(&mut output), 1);
    assert_eq!(output, [1.0, 3.0, 9.0]);
    assert_eq!(reader.stats().read_frames, 1);
    assert_eq!(reader.queued_frames(), 1);
}
