//! Opt-in publication identity; the base v2 CREATE is compatibility-only.
use super::*;

#[derive(Debug, Clone, Copy)]
pub struct IdentityCapabilities;

impl IdentityCapabilities {
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != IDENTITY_CAPABILITIES_SIZE {
            return Err(ProtocolError::BufferSize {
                expected: IDENTITY_CAPABILITIES_SIZE,
                actual: bytes.len(),
            });
        }
        if read_u32(bytes, 0) != IDENTITY_CAPABILITIES_SIZE as u32
            || read_u32(bytes, 4) != IDENTITY_VERSION
            || read_u32(bytes, 8) != CAP_PUBLICATION_IDENTITY
            || read_u32(bytes, 12) != 0
        {
            return Err(ProtocolError::Inconsistent(
                "immutable publication identity is unavailable",
            ));
        }
        Ok(Self)
    }
}

pub fn identity_capabilities_request() -> [u8; 8] {
    let mut bytes = request_header();
    write_u32(&mut bytes, 4, IDENTITY_VERSION);
    bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PublicationMode {
    /// Only a newly allocated project identity may establish machine state.
    EstablishNew = 1,
    /// Project-loaded identities require existing state or unique adoption.
    ReopenOrAdopt = 2,
}

pub fn create_identity_request(
    id: StableEndpointId,
    intent: Intent,
    channels: Channels,
    name: &str,
    allow_default: bool,
    mode: PublicationMode,
) -> Result<[u8; CREATE_IDENTITY_SIZE], ProtocolError> {
    if id.low() == 0 && id.high() == 0 {
        return Err(invalid("stable identity", 0));
    }
    let legacy = create_request(id, intent, channels, name, allow_default)?;
    let mut bytes = [0; CREATE_IDENTITY_SIZE];
    bytes[..CREATE_SIZE].copy_from_slice(&legacy);
    write_u32(&mut bytes, 0, CREATE_IDENTITY_SIZE as u32);
    write_u32(&mut bytes, 4, IDENTITY_VERSION);
    write_u32(&mut bytes, 168, mode as u32);
    Ok(bytes)
}
