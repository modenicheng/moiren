use moiren_engine::{boundary::*, buffer::*, processor::ProcessContext, sample::ProcessingSample};

fn context(frames: usize) -> ProcessContext {
    ProcessContext {
        timeline_epoch: 1,
        timeline_start: 0,
        frames,
        processing_sr: 48_000.0,
    }
}

#[test]
fn bridge_endpoints_move_to_distinct_threads_and_keep_channels_aligned() {
    let (mut writer, mut reader) = audio_bridge::<f32>(2, 3, 1024).unwrap();
    let producer = std::thread::spawn(move || {
        let data: Vec<f32> = (0..128)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect();
        let mut offset = 0;
        while offset < data.len() {
            let accepted = writer
                .write_interleaved(&data[offset..])
                .unwrap()
                .transferred_frames;
            offset += accepted * 2;
            std::thread::yield_now();
        }
        writer
    });
    let consumer = std::thread::spawn(move || {
        let mut frame = [0.0; 2];
        let mut received = Vec::new();
        while received.len() < 256 {
            if reader
                .read_interleaved(&mut frame)
                .unwrap()
                .transferred_frames
                == 1
            {
                received.extend(frame);
            }
            std::thread::yield_now();
        }
        (reader, received)
    });
    let _writer = producer.join().unwrap();
    let (_reader, samples) = consumer.join().unwrap();
    for (frame, samples) in samples.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(*samples, [frame as f32, -(frame as f32)]);
    }
}

#[test]
fn interleaved_bridge_preserves_complete_frames_through_wrap_and_shortfall() {
    fn check<S: ProcessingSample>() {
        let (mut writer, mut reader) = audio_bridge::<S>(2, 3, 1024).unwrap();
        let samples: Vec<_> = [1.0, 10.0, 2.0, 20.0, 3.0, 30.0, 4.0, 40.0]
            .map(S::from_f64)
            .into();
        let report = writer.write_interleaved(&samples).unwrap();
        assert_eq!(report.transferred_frames, 3);
        assert_eq!(report.xruns, 1);
        let mut first = [S::ZERO; 4];
        assert_eq!(
            reader
                .read_interleaved(&mut first)
                .unwrap()
                .transferred_frames,
            2
        );
        assert_eq!(first.map(S::to_f64), [1.0, 10.0, 2.0, 20.0]);
        assert_eq!(
            writer
                .write_interleaved(&samples[6..])
                .unwrap()
                .transferred_frames,
            1
        );
        let mut tail = [S::from_f64(99.0); 6];
        let report = reader.read_interleaved(&mut tail).unwrap();
        assert_eq!(report.transferred_frames, 2);
        assert_eq!(report.xruns, 1);
        assert_eq!(tail.map(S::to_f64), [3.0, 30.0, 4.0, 40.0, 0.0, 0.0]);
        assert_eq!(
            writer.write_interleaved(&[]).unwrap(),
            BoundaryReport::default()
        );
        assert_eq!(
            reader.read_interleaved(&mut []).unwrap(),
            BoundaryReport::default()
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn bridge_rejects_invalid_shapes_sizes_and_budgets_without_consuming_audio() {
    assert!(matches!(
        audio_bridge::<f32>(0, 3, 1024),
        Err(BridgeError::InvalidLayout)
    ));
    assert!(matches!(
        audio_bridge::<f32>(2, 0, 1024),
        Err(BridgeError::InvalidLayout)
    ));
    assert!(matches!(
        audio_bridge::<f32>(2, usize::MAX, usize::MAX),
        Err(BridgeError::SizeOverflow)
    ));
    assert!(matches!(
        audio_bridge::<f64>(2, 4, 63),
        Err(BridgeError::BudgetExceeded)
    ));
    let (mut writer, mut reader) = audio_bridge::<f32>(2, 2, 16).unwrap();
    assert_eq!(writer.capacity_frames(), 2);
    assert_eq!(reader.capacity_frames(), 2);
    assert_eq!(writer.channel_count(), 2);
    assert_eq!(reader.channel_count(), 2);
    assert_eq!(
        writer.write_interleaved(&[1.0]),
        Err(BridgeError::InvalidInterleaved)
    );
    writer.write_interleaved(&[1.0, 2.0]).unwrap();
    assert_eq!(
        reader.read_interleaved(&mut [99.0; 3]),
        Err(BridgeError::InvalidInterleaved)
    );
    let mut result = [0.0; 2];
    reader.read_interleaved(&mut result).unwrap();
    assert_eq!(result, [1.0, 2.0]);
}

#[test]
fn abandoned_writer_drains_then_silences_and_abandoned_reader_discards() {
    let (mut writer, mut reader) = audio_bridge::<f64>(1, 4, 32).unwrap();
    writer.write_interleaved(&[1.0, 2.0]).unwrap();
    drop(writer);
    let mut result = [99.0; 4];
    let report = reader.read_interleaved(&mut result).unwrap();
    assert_eq!(report.transferred_frames, 2);
    assert!(report.discontinuity);
    assert_eq!(result, [1.0, 2.0, 0.0, 0.0]);
    let (mut writer, reader) = audio_bridge::<f64>(1, 4, 32).unwrap();
    drop(reader);
    let report = writer.write_interleaved(&[1.0, 2.0]).unwrap();
    assert_eq!(report.transferred_frames, 0);
    assert!(report.discontinuity);
}

#[test]
fn planar_bridge_supports_multichannel_fan_out_without_modifying_input() {
    fn check<S: ProcessingSample>() {
        let (mut writer, mut reader) = audio_bridge::<S>(3, 3, 4096).unwrap();
        let mut arena = BufferArena::<S>::new(
            &[BufferSlotLayout {
                channels: 3,
                capacity_frames: 4,
            }; 2],
            4096,
        )
        .unwrap();
        let a = arena.slot(0).unwrap();
        let b = arena.slot(1).unwrap();
        let seed = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot: a }])
            .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot: a }])
            .unwrap();
        let write = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot: b }])
            .unwrap();
        arena
            .with_io(&seed, 0..4, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                for (ch, samples) in outputs.get_mut(0).unwrap().channels_mut().enumerate() {
                    for (frame, sample) in samples.iter_mut().enumerate() {
                        *sample = S::from_f64((ch * 10 + frame + 1) as f64);
                    }
                }
            })
            .unwrap();
        arena
            .with_io(&read, 0..4, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                assert_eq!(
                    writer
                        .write(&context(4), inputs.get(0).unwrap())
                        .transferred_frames,
                    3
                );
            })
            .unwrap();
        arena
            .with_io(&write, 0..4, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                assert_eq!(
                    reader
                        .read(&context(4), outputs.get_mut(0).unwrap())
                        .transferred_frames,
                    3
                );
            })
            .unwrap();
        let verify = arena
            .prepare_io(&[
                PortAccess::Read { port: 0, slot: a },
                PortAccess::Read { port: 1, slot: b },
            ])
            .unwrap();
        arena
            .with_io(&verify, 0..4, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                for ch in 0..3 {
                    for frame in 0..4 {
                        let expected = (ch * 10 + frame + 1) as f64;
                        assert_eq!(inputs.get(0).unwrap().channel(ch)[frame].to_f64(), expected);
                        assert_eq!(
                            inputs.get(1).unwrap().channel(ch)[frame].to_f64(),
                            if frame < 3 { expected } else { 0.0 }
                        );
                    }
                }
            })
            .unwrap();
    }
    check::<f32>();
    check::<f64>();
}
