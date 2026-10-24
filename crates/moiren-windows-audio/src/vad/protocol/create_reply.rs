//! CREATE alone may acknowledge a deferred request without creating a resource.
use super::*;

const DEFERRED_REPLY_KIND: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateReply {
    Created(EndpointSnapshot),
    Deferred,
}

pub(crate) fn decode_create_reply(
    bytes: &[u8],
    capabilities: Capabilities,
    id: StableEndpointId,
    intent: Intent,
    channels: Channels,
) -> Result<CreateReply, ProtocolError> {
    validate_header(bytes, SNAPSHOT_SIZE)?;
    if read_u32(bytes, 16) == DEFERRED_REPLY_KIND {
        if capabilities.flags() & CAP_CREATE_DEFERRED == 0 {
            return Err(ProtocolError::Inconsistent(
                "CREATE deferral was not negotiated",
            ));
        }
        capabilities.validate_create(intent, channels)?;
        let mut expected = request_header::<SNAPSHOT_SIZE>();
        write_u32(&mut expected, 16, DEFERRED_REPLY_KIND);
        write_u32(&mut expected, 20, intent as u32);
        write_u32(&mut expected, 24, channels as u32);
        write_u64(&mut expected, 40, capabilities.session_epoch());
        expected[72..88].copy_from_slice(&id.to_le_bytes());
        // This acknowledgment carries no owned key. Any unexpected bit keeps
        // CREATE uncertain rather than authorizing a retry after partial work.
        if bytes != expected {
            return Err(ProtocolError::Inconsistent(
                "CREATE deferral identity or reserved fields changed",
            ));
        }
        return Ok(CreateReply::Deferred);
    }
    Ok(CreateReply::Created(
        EndpointSnapshot::decode(bytes, capabilities)?.require_created(id, intent, channels)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(flags: u32) -> Capabilities {
        let mut bytes = request_header::<CAPABILITIES_SIZE>();
        for (offset, value) in [
            (8, flags),
            (12, MAX_ENDPOINTS),
            (16, MAX_STREAMS_PER_PIN),
            (20, SAMPLE_RATE),
            (24, FORMATS),
            (28, CHANNEL_MASK),
            (32, INTENT_MASK),
            (36, REQUEST_SLOTS as u32),
            (48, MAX_PACKET_FRAMES),
        ] {
            write_u32(&mut bytes, offset, value);
        }
        write_u64(&mut bytes, 40, 7);
        Capabilities::decode(&bytes).unwrap()
    }

    fn id() -> StableEndpointId {
        StableEndpointId::new(0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210)
    }

    fn deferred() -> [u8; SNAPSHOT_SIZE] {
        let mut bytes = request_header();
        write_u32(&mut bytes, 16, 5);
        write_u32(&mut bytes, 20, Intent::Playback as u32);
        write_u32(&mut bytes, 24, Channels::Stereo as u32);
        write_u64(&mut bytes, 40, 7);
        bytes[72..88].copy_from_slice(&id().to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8], flags: u32) -> Result<CreateReply, ProtocolError> {
        decode_create_reply(bytes, caps(flags), id(), Intent::Playback, Channels::Stereo)
    }

    #[test]
    fn deferred_create_acknowledges_no_resource_without_an_owned_key() {
        assert_eq!(decode(&deferred(), 63), Ok(CreateReply::Deferred));
        assert!(EndpointSnapshot::decode(&deferred(), caps(63)).is_err());
    }

    #[test]
    fn deferred_create_requires_negotiated_capability_and_exact_request_echo() {
        assert!(decode(&deferred(), 31).is_err());
        // Every bit is either part of the echoed request/session or must be zero.
        for offset in 0..SNAPSHOT_SIZE {
            for bit in 0..8 {
                let mut bytes = deferred();
                bytes[offset] ^= 1 << bit;
                assert!(decode(&bytes, 63).is_err(), "offset {offset}, bit {bit}");
            }
        }
        for length in [0, 8, 127, 129, 256] {
            assert!(decode(&vec![0; length], 63).is_err());
        }
    }

    #[test]
    fn normal_created_reply_still_requires_current_ownership_and_shape() {
        let mut bytes = deferred();
        write_u64(&mut bytes, 8, 11);
        write_u32(&mut bytes, 16, PdoState::Creating as u32);
        write_u64(&mut bytes, 48, 11);
        let CreateReply::Created(snapshot) = decode(&bytes, 63).unwrap() else {
            panic!("normal CREATE must produce an owned key");
        };
        assert_eq!(snapshot.pdo_token().get(), 11);
        assert_eq!(snapshot.session_epoch(), 7);
        assert!(
            decode(&bytes, 31).is_ok(),
            "old driver replies remain valid"
        );
        for offset in [20, 24, 40, 72] {
            let mut malformed = bytes;
            malformed[offset] ^= 1;
            assert!(decode(&malformed, 63).is_err());
        }
    }
}
