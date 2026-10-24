use super::*;
use moiren_engine::node::VirtualEndpointId;

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn id() -> StableEndpointId {
    StableEndpointId::new(0x0807_0605_0403_0201, 0x100f_0e0d_0c0b_0a09)
}
fn caps_bytes(flags: u32) -> [u8; 64] {
    let mut bytes = [0; 64];
    for (offset, value) in [
        (0, 64),
        (4, 2),
        (8, flags),
        (12, 64),
        (16, 1),
        (20, 48_000),
        (24, if flags & CAP_AUDIO != 0 { 3 } else { 0 }),
        (28, 6),
        (32, 6),
        (36, 4),
        (48, 4_800),
    ] {
        put32(&mut bytes, offset, value);
    }
    put64(&mut bytes, 40, 0x1020_3040_5060_7080);
    bytes
}
fn caps() -> Capabilities {
    Capabilities::decode(&caps_bytes(31)).unwrap()
}
fn foundation() -> Capabilities {
    Capabilities::decode(&caps_bytes(3)).unwrap()
}

fn snapshot_bytes() -> [u8; 128] {
    let mut bytes = [0; 128];
    for (offset, value) in [
        (0, 128),
        (4, 2),
        (16, 1),
        (20, 1),
        (24, 2),
        (28, 1),
        (32, 1),
        (36, 1),
        (88, 2),
        (92, 480),
        (96, 0),
        (100, 2),
    ] {
        put32(&mut bytes, offset, value);
    }
    for (offset, value) in [
        (8, 0x1234_5678_9abc_def0),
        (40, caps().session_epoch()),
        (48, 0x1234_5678_9abc_def0),
        (56, 5),
        (64, 7),
        (104, 27),
        (112, 31),
    ] {
        put64(&mut bytes, offset, value);
    }
    bytes[72..88].copy_from_slice(&id().to_le_bytes());
    bytes
}
fn snapshot() -> EndpointSnapshot {
    EndpointSnapshot::decode(&snapshot_bytes(), caps()).unwrap()
}
fn raw_bytes(state: u32) -> [u8; 128] {
    let mut bytes = snapshot_bytes();
    for (offset, value) in [
        (16, state),
        (28, 0),
        (32, u32::from(state == 1)),
        (36, 0),
        (88, 0),
        (92, 0),
        (100, 0),
    ] {
        put32(&mut bytes, offset, value);
    }
    put64(&mut bytes, 56, 0);
    bytes
}

#[test]
fn capabilities_are_exact_v2_and_foundation_stays_closed() {
    let value = caps();
    assert_eq!(value.flags(), 31);
    assert_eq!(value.max_endpoints(), 64);
    assert_eq!(value.max_streams_per_pin(), 1);
    assert_eq!(value.target_sample_rate(), 48_000);
    assert_eq!(value.sample_formats(), 3);
    assert_eq!(value.channel_mask(), 6);
    assert_eq!(value.intent_mask(), 6);
    assert_eq!(value.request_slots(), 4);
    assert_eq!(value.max_packet_frames(), 4_800);
    value.require_pcm().unwrap();
    assert_eq!(
        foundation().require_audio(),
        Err(ProtocolError::AudioUnavailable)
    );
    let audio_only = Capabilities::decode(&caps_bytes(15)).unwrap();
    audio_only.require_audio().unwrap();
    assert_eq!(audio_only.require_pcm(), Err(ProtocolError::PcmUnavailable));
}

#[test]
fn fail_closed_capability_negotiation_checks_all_fields() {
    for (offset, values) in [
        (0, vec![0, 48, 128]),
        (4, vec![0, 1, 3]),
        (8, vec![0, 2, 7, 11, 19, 32, u32::MAX]),
        (12, vec![0, 65]),
        (16, vec![0, 2]),
        (20, vec![0, 44_100]),
        (24, vec![4, u32::MAX]),
        (28, vec![0, 1, 7]),
        (32, vec![0, 1, 7]),
        (36, vec![0, 3, 5]),
        (48, vec![0, 4_801]),
        (52, vec![1]),
        (56, vec![1]),
        (60, vec![1]),
    ] {
        for value in values {
            let mut bytes = caps_bytes(3);
            put32(&mut bytes, offset, value);
            assert!(
                Capabilities::decode(&bytes).is_err(),
                "offset={offset}, value={value}"
            );
        }
    }
    let mut bytes = caps_bytes(3);
    put64(&mut bytes, 40, 0);
    assert!(Capabilities::decode(&bytes).is_err());
    for offset in [24, 28, 32] {
        let mut bytes = caps_bytes(31);
        put32(&mut bytes, offset, 2);
        assert!(Capabilities::decode(&bytes).is_err());
    }
}

#[test]
fn stable_identity_is_opaque_bytes_not_guid_or_runtime_token() {
    assert_eq!(
        id().to_le_bytes(),
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_eq!(id().low(), 0x0807_0605_0403_0201);
    assert_eq!(id().high(), 0x100f_0e0d_0c0b_0a09);
    assert_eq!(id().to_string(), "100f0e0d0c0b0a090807060504030201");
    for bytes in [[0; 16], [255; 16], id().to_le_bytes()] {
        assert_eq!(StableEndpointId::from_le_bytes(bytes).to_le_bytes(), bytes);
        assert!(
            create_request(
                StableEndpointId::from_le_bytes(bytes),
                Intent::Playback,
                Channels::Mono,
                "zero permitted",
                false
            )
            .is_ok()
        );
    }
    assert!(RuntimePdoToken::new(0).is_err());
    assert_eq!(RuntimePdoToken::new(u64::MAX).unwrap().get(), u64::MAX);
    let token = RuntimePdoToken::new(3).unwrap();
    assert!(EndpointKey::new(token, 0, 1).is_err());
    assert!(EndpointKey::new(token, 1, 0).is_err());
}

#[test]
fn persistent_virtual_identity_maps_to_abi_words_without_endian_changes() {
    let virtual_id = VirtualEndpointId::from_bytes([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ]);

    let stable_id = StableEndpointId::from_virtual_endpoint_id(virtual_id);

    assert_eq!(stable_id.low(), 0x0807_0605_0403_0201);
    assert_eq!(stable_id.high(), 0x100f_0e0d_0c0b_0a09);
    assert_eq!(stable_id.to_virtual_endpoint_id(), virtual_id);
}

#[test]
fn create_wire_contains_all_fields_and_zero_terminated_name_without_truncation() {
    let bytes = create_request(id(), Intent::Microphone, Channels::Stereo, "VAD", false).unwrap();
    assert_eq!(bytes.len(), 168);
    for (offset, expected) in [(0, 168), (4, 2), (8, 2), (12, 2), (32, 0), (36, 0)] {
        assert_eq!(u32_at(&bytes, offset), expected);
    }
    assert_eq!(&bytes[16..32], &id().to_le_bytes());
    assert_eq!(&bytes[40..48], &[86, 0, 65, 0, 68, 0, 0, 0]);
    assert!(bytes[48..].iter().all(|b| *b == 0));
    assert_eq!(
        u32_at(
            &create_request(id(), Intent::Playback, Channels::Mono, "x", true).unwrap(),
            32
        ),
        1
    );
    assert!(
        create_request(
            id(),
            Intent::Playback,
            Channels::Mono,
            &"x".repeat(63),
            false
        )
        .is_ok()
    );
    for name in [
        "".to_owned(),
        "x\0y".to_owned(),
        "x\ny".to_owned(),
        "x".repeat(64),
        "\u{1f3b5}".repeat(32),
    ] {
        assert_eq!(
            create_request(id(), Intent::Playback, Channels::Mono, &name, false),
            Err(ProtocolError::InvalidName)
        );
    }
    let bytes = create_request(id(), Intent::Playback, Channels::Mono, "\u{1f3b5}", false).unwrap();
    assert_eq!(&bytes[40..46], &[0x3c, 0xd8, 0xb5, 0xdf, 0, 0]);
}

#[test]
fn endpoint_and_gate_requests_have_exact_epoch_offsets() {
    assert_eq!(capabilities_request(), [8, 0, 0, 0, 2, 0, 0, 0]);
    let value = snapshot();
    let bytes = endpoint_request(value.key());
    assert_eq!(bytes.len(), 32);
    assert_eq!(u32_at(&bytes, 0), 32);
    assert_eq!(u32_at(&bytes, 4), 2);
    assert_eq!(u64_at(&bytes, 8), value.pdo_token().get());
    assert_eq!(u64_at(&bytes, 16), value.session_epoch());
    assert_eq!(u64_at(&bytes, 24), value.endpoint_generation());
    for open in [false, true] {
        let bytes = gate_request(value, open);
        assert_eq!(bytes.len(), 48);
        assert_eq!(u32_at(&bytes, 0), 48);
        assert_eq!(&bytes[8..32], &endpoint_request(value.key())[8..]);
        assert_eq!(u32_at(&bytes, 32), u32::from(open));
        assert_eq!(u32_at(&bytes, 36), 0);
        assert_eq!(u64_at(&bytes, 40), value.gate_epoch());
    }
}

#[test]
fn snapshots_decode_every_field_at_its_c_offset() {
    let value = snapshot();
    assert_eq!(value.pdo_token().get(), 0x1234_5678_9abc_def0);
    assert_eq!(value.state(), PdoState::Present);
    assert_eq!(value.intent(), Intent::Playback);
    assert_eq!(value.channels(), Channels::Stereo);
    assert!(value.gate_open() && value.audio_ready() && value.policy_prepared());
    assert_eq!(value.session_epoch(), 0x1020_3040_5060_7080);
    assert_eq!(value.endpoint_generation(), 0x1234_5678_9abc_def0);
    assert_eq!(value.stream_generation(), 5);
    assert_eq!(value.gate_epoch(), 7);
    assert_eq!(value.stable_id(), id());
    assert_eq!(value.sample_format(), Some(SampleFormat::Float32));
    assert_eq!(value.packet_frames(), 480);
    assert_eq!(value.last_status(), 0);
    assert_eq!(value.stream_state(), StreamState::Running);
    assert_eq!(value.dropped_packets(), 27);
    assert_eq!(value.silence_packets(), 31);
}

#[test]
fn snapshot_rejects_invalid_headers_bools_enums_epochs_and_shape() {
    for (offset, values) in [
        (0, vec![48, 127, 129]),
        (4, vec![0, 1, 3]),
        (16, vec![0, 5]),
        (20, vec![0, 3]),
        (24, vec![0, 3]),
        (28, vec![2]),
        (32, vec![2]),
        (36, vec![2]),
        (88, vec![0, 3]),
        (92, vec![4_801]),
        (100, vec![3]),
        (120, vec![1]),
        (124, vec![1]),
    ] {
        for value in values {
            let mut bytes = snapshot_bytes();
            put32(&mut bytes, offset, value);
            assert!(
                EndpointSnapshot::decode(&bytes, caps()).is_err(),
                "offset {offset}, value {value}"
            );
        }
    }
    for offset in [8, 40, 48, 56, 64] {
        let mut bytes = snapshot_bytes();
        put64(&mut bytes, offset, 0);
        assert!(
            EndpointSnapshot::decode(&bytes, caps()).is_err(),
            "offset {offset}"
        );
    }
    let mut bytes = snapshot_bytes();
    put64(&mut bytes, 40, caps().session_epoch() + 1);
    assert!(EndpointSnapshot::decode(&bytes, caps()).is_err());
    assert!(EndpointSnapshot::decode(&snapshot_bytes(), foundation()).is_err());
}

#[test]
fn creating_and_async_failure_do_not_claim_ready_audio() {
    let creating = EndpointSnapshot::decode(&raw_bytes(4), foundation()).unwrap();
    assert_eq!(creating.state(), PdoState::Creating);
    creating
        .require_created(id(), Intent::Playback, Channels::Stereo)
        .unwrap();
    assert!(!creating.policy_prepared() && !creating.audio_ready() && !creating.gate_open());
    let present = EndpointSnapshot::decode(&raw_bytes(1), foundation()).unwrap();
    present.require_progress_after(creating).unwrap();
    assert!(creating.require_progress_after(present).is_err());
    let mut bytes = raw_bytes(2);
    put32(&mut bytes, 96, 0xc000_009a);
    let failed = EndpointSnapshot::decode(&bytes, foundation()).unwrap();
    assert_eq!(failed.last_status(), 0xc000_009a);
    failed.require_progress_after(creating).unwrap();
}

#[test]
fn lifecycle_identity_and_all_generations_cannot_regress() {
    let previous = snapshot();
    for offset in [8, 40, 48, 72, 80] {
        let mut bytes = snapshot_bytes();
        put64(&mut bytes, offset, u64_at(&snapshot_bytes(), offset) + 1);
        let result = EndpointSnapshot::decode(&bytes, caps())
            .and_then(|value| value.require_progress_after(previous));
        assert!(result.is_err(), "identity offset {offset}");
    }
    for offset in [56, 64] {
        let mut bytes = snapshot_bytes();
        put64(&mut bytes, offset, 1);
        assert!(
            EndpointSnapshot::decode(&bytes, caps())
                .unwrap()
                .require_progress_after(previous)
                .is_err()
        );
    }
    let removing = EndpointSnapshot::decode(&raw_bytes(2), foundation()).unwrap();
    let removed = EndpointSnapshot::decode(&raw_bytes(3), foundation()).unwrap();
    assert!(!removing.removal_complete_after(removing).unwrap());
    assert!(removed.removal_complete_after(removing).unwrap());
    assert!(removing.removal_complete_after(removed).is_err());
}

#[test]
fn pcm_shape_requires_open_running_stream_but_native_closed_gate_is_valid_snapshot() {
    for (offset, value) in [
        (16, 4),
        (28, 0),
        (32, 0),
        (36, 0),
        (92, 0),
        (100, 0),
        (100, 1),
    ] {
        let mut bytes = snapshot_bytes();
        put32(&mut bytes, offset, value);
        let result = EndpointSnapshot::decode(&bytes, caps()).and_then(|s| s.pcm_shape(caps()));
        assert!(result.is_err(), "offset={offset}");
    }
    let mut bytes = snapshot_bytes();
    put32(&mut bytes, 28, 0);
    let native = EndpointSnapshot::decode(&bytes, caps()).unwrap();
    assert_eq!(native.stream_state(), StreamState::Running);
    assert_eq!(native.pcm_shape(caps()), Err(ProtocolError::NotReady));
    // Native admission publishes its generation before format/packets exist.
    // QUERY during that interval is normal not-ready, not a malformed ABI.
    put32(&mut bytes, 88, 0);
    put32(&mut bytes, 92, 0);
    put32(&mut bytes, 100, 1);
    let opening = EndpointSnapshot::decode(&bytes, caps()).unwrap();
    assert_eq!(opening.pcm_shape(caps()), Err(ProtocolError::NotReady));
}

#[test]
fn exact_native_frames_determine_every_transfer_size() {
    for intent in [1, 2] {
        for channels in [1, 2] {
            for format in [1, 2] {
                for frames in [1, 48, 480, 4_800] {
                    let mut bytes = snapshot_bytes();
                    for (offset, value) in
                        [(20, intent), (24, channels), (88, format), (92, frames)]
                    {
                        put32(&mut bytes, offset, value);
                    }
                    let shape = EndpointSnapshot::decode(&bytes, caps())
                        .unwrap()
                        .pcm_shape(caps())
                        .unwrap();
                    assert_eq!(
                        shape.pcm_bytes(),
                        (frames * channels * if format == 1 { 2 } else { 4 }) as usize
                    );
                    assert_eq!(shape.read_bytes(), 80 + shape.pcm_bytes());
                    assert!(shape.pcm_bytes() <= MAX_PCM_BYTES);
                }
            }
        }
    }
}

#[test]
fn pcm_header_offsets_and_driver_stamp_validation() {
    let shape = snapshot().pcm_shape(caps()).unwrap();
    let header = shape.header(0x1122_3344_5566_7788, 0x2233_4455_6677_8899);
    assert_eq!(header.len(), 80);
    for (offset, value) in [(0, 80), (4, 2), (64, 2), (68, 480), (72, 2), (76, 0)] {
        assert_eq!(u32_at(&header, offset), value);
    }
    for (offset, value) in [
        (8, snapshot().pdo_token().get()),
        (16, snapshot().session_epoch()),
        (24, 0x1234_5678_9abc_def0),
        (32, 5),
        (40, 7),
        (48, 0x1122_3344_5566_7788),
        (56, 0x2233_4455_6677_8899),
    ] {
        assert_eq!(u64_at(&header, offset), value);
    }
    let stamp = PcmPacketStamp::decode_read(&header, shape).unwrap();
    assert_eq!(stamp.sequence, 0x1122_3344_5566_7788);
    assert_eq!(stamp.qpc, 0x2233_4455_6677_8899);
    for offset in [0, 4, 8, 16, 24, 32, 40, 64, 68, 72, 76] {
        let mut changed = header;
        changed[offset] ^= 1;
        assert!(
            PcmPacketStamp::decode_read(&changed, shape).is_err(),
            "offset {offset}"
        );
    }
    assert!(PcmPacketStamp::decode_read(&shape.header(1, 0), shape).is_err());
}

#[test]
fn arbitrary_lengths_and_bytes_never_panic_or_accept_wrong_size() {
    let mut seed = 0x5eed_u64;
    for length in 0..=256 {
        let mut bytes = vec![0; length];
        for byte in &mut bytes {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            *byte = (seed >> 32) as u8;
        }
        let _ = Capabilities::decode(&bytes);
        let _ = EndpointSnapshot::decode(&bytes, caps());
        let _ = PcmPacketStamp::decode_read(&bytes, snapshot().pcm_shape(caps()).unwrap());
        if length != 64 {
            let mut resized = caps_bytes(3).to_vec();
            resized.resize(length, 0);
            assert!(Capabilities::decode(&resized).is_err());
        }
        if length != 128 {
            let mut resized = snapshot_bytes().to_vec();
            resized.resize(length, 0);
            assert!(EndpointSnapshot::decode(&resized, caps()).is_err());
        }
    }
}
