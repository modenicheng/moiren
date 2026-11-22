use super::*;
use moiren_windows_audio::vad::{Channels, encode_interleaved};

fn encode(samples: &[f32], format: SampleFormat) -> Vec<u8> {
    let mut encoded = vec![0; samples.len() * format.bytes_per_sample()];
    encode_interleaved(samples, &mut encoded, format, Channels::Stereo).unwrap();
    encoded
}

#[test]
fn source_write_witness_actual_copy_confirms_nonzero_frame_not_preroll_prefix() {
    for format in [SampleFormat::Pcm16, SampleFormat::Float32] {
        let samples = [0., 0., 0.25, -0.25, 0.5, -0.5];
        let encoded = encode(&samples, format);
        let cursor = SourceProgress {
            native_submitted_frames: 1056,
            acknowledged_source_frames: 100,
            ..Default::default()
        };
        let mut witness = WriteWitness::default();
        let packet = witness
            .prepare(format, &samples, &encoded, 0, &cursor)
            .unwrap();
        let mut destination = vec![0; encoded.len()];
        let mut releases = 0;
        let mut ticks = 10;
        // SAFETY: exact owned writable destination, disjoint from encoded;
        // packet came from prepare for this slice and the clock cannot panic.
        unsafe {
            copy_release(
                &encoded,
                destination.as_mut_ptr(),
                Some((&mut witness, packet)),
                || {
                    ticks += 1;
                    Some(ticks)
                },
                || {
                    releases += 1;
                    Ok::<_, i32>(())
                },
                |e| *e,
            )
        }
        .unwrap();
        assert_eq!(releases, 1);
        assert_eq!(destination, encoded);
        assert!(witness.attempted && !witness.qualified && witness.failed_attempt.is_none());
        let confirmed = witness.confirmed.as_ref().unwrap();
        assert_eq!(confirmed.source_frame, 101);
        assert_eq!(confirmed.native_submission_frame, 1057);
        assert_eq!(confirmed.nonzero_frame_offset, 1);
        assert_eq!(confirmed.destination.unwrap(), confirmed.encoded);
        assert_eq!(
            (
                confirmed.before_copy_qpc,
                confirmed.after_copy_qpc,
                confirmed.release_return_qpc
            ),
            (Some(11), Some(12), Some(13))
        );
        assert_eq!(confirmed.release_hresult, Some(0));
        assert!(
            witness
                .prepare(format, &samples, &encoded, 0, &cursor)
                .is_none()
        );
    }
}

#[test]
fn source_write_witness_release_failure_retains_error_and_does_not_advance_cursor() {
    let plan = super::super::source::SourceSchedule {
        schema_version: 1,
        schedule_id: "witness".into(),
        waveform_version: super::super::source::WAVEFORM_VERSION.into(),
        sample_rate: 48_000,
        channels: 2,
        marker_frames: 32,
        marker_amplitude: 0.1,
        total_source_frames: 32,
        starts: vec![0],
    }
    .prepare(1)
    .unwrap();
    let mut source = super::super::source::SourceCursor::new(Some(plan));
    let before = source.progress.clone();
    let mut witness = WriteWitness::default();
    let mut releases = 0;
    let error = source
        .submit(&mut [0.; 64], false, |samples| {
            let encoded = encode(samples, SampleFormat::Pcm16);
            let packet = witness
                .prepare(SampleFormat::Pcm16, samples, &encoded, 0, &before)
                .unwrap();
            let mut destination = vec![0; encoded.len()];
            // SAFETY: valid exact disjoint storage and prepared packet.
            let result = unsafe {
                copy_release(
                    &encoded,
                    destination.as_mut_ptr(),
                    Some((&mut witness, packet)),
                    || None,
                    || {
                        releases += 1;
                        Err(-123)
                    },
                    |e| *e,
                )
            };
            assert_eq!(result, Err(SubmitError::Release(-123)));
            anyhow::bail!("original ReleaseBuffer error -123")
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "original ReleaseBuffer error -123");
    assert_eq!(source.progress, before);
    assert_eq!(releases, 1);
    assert!(witness.confirmed.is_none());
    let failed = witness.failed_attempt.as_ref().unwrap();
    assert_eq!(failed.failure, Some("release_failed"));
    assert_eq!(failed.release_hresult, Some(-123));
    assert_eq!(failed.destination.unwrap(), failed.encoded);
    assert!(
        witness
            .prepare(SampleFormat::Pcm16, &[0.25; 2], &[1; 4], 0, &before)
            .is_none()
    );
}

#[test]
fn source_write_witness_null_always_releases_and_preserves_release_error_precedence() {
    for release_error in [None, Some(-456)] {
        let samples = [0.25; 2];
        let encoded = encode(&samples, SampleFormat::Pcm16);
        let mut witness = WriteWitness::default();
        let packet = witness
            .prepare(
                SampleFormat::Pcm16,
                &samples,
                &encoded,
                0,
                &SourceProgress::default(),
            )
            .unwrap();
        let mut releases = 0;
        // SAFETY: null is explicitly supported; no pointer dereference occurs.
        let result = unsafe {
            copy_release(
                &encoded,
                std::ptr::null_mut(),
                Some((&mut witness, packet)),
                || None,
                || {
                    releases += 1;
                    release_error.map_or(Ok(()), Err)
                },
                |e| *e,
            )
        };
        assert_eq!(releases, 1);
        assert_eq!(
            result,
            Err(release_error.map_or(SubmitError::NullBuffer, SubmitError::Release))
        );
        assert!(witness.attempted && witness.confirmed.is_none());
        let failed = witness.failed_attempt.unwrap();
        assert!(failed.destination.is_none());
        assert_eq!(failed.release_hresult, Some(release_error.unwrap_or(0)));
        assert_eq!(
            failed.failure,
            Some(if release_error.is_some() {
                "release_failed"
            } else {
                "null_destination"
            })
        );
    }
}

#[test]
fn source_write_witness_default_copy_has_no_diagnostic_clocks() {
    let encoded = [1, 2, 3, 4];
    let mut destination = [0; 4];
    let mut releases = 0;
    // SAFETY: exact owned disjoint storage, no diagnostic packet.
    unsafe {
        copy_release(
            &encoded,
            destination.as_mut_ptr(),
            None,
            || panic!("disabled witness clock"),
            || {
                releases += 1;
                Ok::<_, i32>(())
            },
            |e| *e,
        )
    }
    .unwrap();
    assert_eq!(destination, encoded);
    assert_eq!(releases, 1);
}

#[test]
fn source_write_witness_prelease_selection_is_bounded_and_rejects_silence_or_bad_shape() {
    let witness = WriteWitness::default();
    let cursor = SourceProgress::default();
    for format in [SampleFormat::Pcm16, SampleFormat::Float32] {
        assert!(
            witness
                .prepare(format, &[0.; 4], &encode(&[0.; 4], format), 0, &cursor)
                .is_none()
        );
        let samples = [0., 0., 0., 0., 0.25, -0.25];
        let encoded = encode(&samples, format);
        assert!(
            witness
                .prepare(format, &samples, &encoded, 2, &cursor)
                .is_none()
        );
        assert!(
            witness
                .prepare(format, &samples, &encoded[..encoded.len() - 1], 0, &cursor)
                .is_none()
        );
        assert!(
            witness
                .prepare(format, &samples[..5], &encoded, 0, &cursor)
                .is_none()
        );
        let packet = witness
            .prepare(format, &samples, &encoded, 0, &cursor)
            .unwrap();
        assert_eq!(packet.prefix_bytes, 2 * format.bytes_per_sample());
        assert_eq!(packet.byte_offset + packet.prefix_bytes, encoded.len());
        let overflow = SourceProgress {
            acknowledged_source_frames: u64::MAX,
            ..Default::default()
        };
        assert!(
            witness
                .prepare(format, &samples, &encoded, 0, &overflow)
                .is_none()
        );
        let tiny = [f32::MIN_POSITIVE; 2];
        if format == SampleFormat::Pcm16 {
            assert!(
                witness
                    .prepare(format, &tiny, &encode(&tiny, format), 0, &cursor)
                    .is_none()
            );
        }
    }
}
