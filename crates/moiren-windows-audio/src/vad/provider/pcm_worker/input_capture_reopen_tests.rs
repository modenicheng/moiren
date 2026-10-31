use super::*;

#[test]
fn input_capture_same_identity_owner_reopen_does_not_merge_raw_segments() {
    let mut f = Fixture::new(8, 8);
    let identity = capture_identity(&f.state, SampleFormat::Float32);
    f.capture.as_mut().unwrap().bind(identity);
    f.packet(1, [0.25, -0.5]);
    assert_eq!(f.pump().unwrap(), PumpState::Current);
    // run_playback binds at each actual PcmOwner creation, even when the
    // native stamp and admission fields happen to remain identical.
    f.capture.as_mut().unwrap().bind(identity);
    f.order = PacketOrder::default();
    f.packet(1, [-0.75, 0.125]);
    assert_eq!(f.pump().unwrap(), PumpState::Current);
    let mut samples = [0.; 4];
    assert_eq!(
        f.reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        2
    );
    assert_eq!(samples, [0.25, -0.5, -0.75, 0.125]);
    let artifact = f.finish();
    assert_eq!(artifact.seal_reason, "owner_reopened");
    assert_eq!(artifact.identity, Some(identity));
    assert_eq!(artifact.stored_frames, 1);
    assert_eq!(artifact.packets.len(), 1);
    assert_eq!(artifact.packets[0].publication, "published");
    assert_eq!(
        artifact.raw,
        [0.25_f32, -0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>()
    );
}

#[test]
fn input_capture_reopened_owner_keeps_original_identity_and_prior_seals() {
    for frames in [1, 8] {
        for changed_identity in [false, true] {
            let mut f = Fixture::new(frames, 8);
            let identity = capture_identity(&f.state, SampleFormat::Float32);
            f.capture.as_mut().unwrap().bind(identity);
            f.packet(1, [0.25, -0.5]);
            f.pump().unwrap();
            let mut next = identity;
            if changed_identity {
                next.native_stamp[3] += 1;
            }
            crate::test_allocation::assert_allocates_nothing(|| {
                f.capture.as_mut().unwrap().bind(next);
            });
            f.order = PacketOrder::default();
            f.packet(1, [-0.75, 0.125]);
            assert_eq!(f.pump().unwrap(), PumpState::Current);
            let mut samples = [0.; 4];
            assert_eq!(
                f.reader
                    .read_interleaved(&mut samples)
                    .unwrap()
                    .transferred_frames,
                2
            );
            assert_eq!(samples, [0.25, -0.5, -0.75, 0.125]);
            let artifact = f.finish();
            assert_eq!(artifact.identity, Some(identity));
            assert_eq!(artifact.stored_frames, 1);
            assert_eq!(artifact.packets.len(), 1);
            assert_eq!(artifact.excluded_epoch_packets, 0);
            assert_eq!(artifact.excluded_stale_packets, 0);
            assert_eq!(
                artifact.seal_reason,
                if frames == 1 {
                    "frame_capacity"
                } else {
                    "owner_reopened"
                }
            );
        }
    }
}

#[test]
fn input_capture_rebind_before_first_data_uses_the_first_observed_owner() {
    let mut f = Fixture::new(8, 8);
    let mut identity = capture_identity(&f.state, SampleFormat::Float32);
    identity.native_stamp[3] += 1;
    f.capture.as_mut().unwrap().bind(identity);
    f.packet(1, [0.125, -0.25]);
    assert_eq!(f.pump().unwrap(), PumpState::Current);
    let artifact = f.finish();
    assert_eq!(artifact.identity, Some(identity));
    assert_eq!(artifact.stored_frames, 1);
    assert_eq!(artifact.packets[0].publication, "published");
    assert_eq!(artifact.seal_reason, "worker_exit");
}
