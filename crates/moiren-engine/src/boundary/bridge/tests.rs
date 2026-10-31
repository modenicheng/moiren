use super::*;

#[test]
fn exact_write_refuses_reader_abandoned_after_admission() {
    fn check<S: ProcessingSample>() {
        let (mut writer, reader) = audio_bridge::<S>(2, 2, 1024).unwrap();
        assert_eq!(
            writer.write_interleaved_exact_before_push(&[S::from_f64(0.25); 4], || {
                drop(reader);
            }),
            Ok(false)
        );
        assert!(writer.tx.is_abandoned());
        assert_eq!(writer.available_frames(), 2);
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn exact_write_empty_after_admission_keeps_zero_frame_success() {
    let (mut writer, reader) = audio_bridge::<f32>(2, 2, 1024).unwrap();
    assert_eq!(
        writer.write_interleaved_exact_before_push(&[], || drop(reader)),
        Ok(true)
    );
    assert_eq!(writer.write_interleaved_exact(&[]), Ok(false));
}

#[test]
fn exact_write_preserves_complete_packets_and_refuses_full_or_malformed_input() {
    fn check<S: ProcessingSample>() {
        let (mut writer, mut reader) = audio_bridge::<S>(2, 3, 1024).unwrap();
        let first = [1.0, 10.0, 2.0, 20.0].map(S::from_f64);
        let tail = [3.0, 30.0, 4.0, 40.0].map(S::from_f64);
        assert_eq!(writer.write_interleaved_exact(&first), Ok(true));
        assert_eq!(writer.write_interleaved_exact(&[]), Ok(true));
        assert_eq!(writer.write_interleaved_exact(&tail), Ok(false));
        assert_eq!(
            writer.write_interleaved_exact(&[S::ZERO]),
            Err(BridgeError::InvalidInterleaved)
        );
        assert_eq!(reader.available_frames(), 2);
        let mut samples = [S::ZERO; 4];
        assert_eq!(
            reader
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            2
        );
        assert_eq!(samples.map(S::to_f64), [1.0, 10.0, 2.0, 20.0]);

        // Publishing the next packet crosses the ring's physical wrap boundary.
        assert_eq!(writer.write_interleaved_exact(&tail), Ok(true));
        assert_eq!(
            reader
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            2
        );
        assert_eq!(samples.map(S::to_f64), [3.0, 30.0, 4.0, 40.0]);
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn exact_write_already_abandoned_preserves_rejection_and_shape_validation() {
    let (mut writer, reader) = audio_bridge::<f32>(2, 2, 1024).unwrap();
    drop(reader);
    assert_eq!(writer.write_interleaved_exact(&[1.0; 4]), Ok(false));
    assert_eq!(writer.write_interleaved_exact(&[]), Ok(false));
    assert_eq!(
        writer.write_interleaved_exact(&[1.0]),
        Err(BridgeError::InvalidInterleaved)
    );
    assert_eq!(writer.available_frames(), 2);
}
