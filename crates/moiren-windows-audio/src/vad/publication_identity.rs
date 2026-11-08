//! Control-side fresh-ID provenance, never reconstructed by parsing a project.
use super::{ClientError, StableEndpointId};
use moiren_engine::node::VirtualEndpointId;

/// Retain this allocation through establishment retries. A saved project's ID
/// is plain `VirtualEndpointId` and can only use the reopen/adopt entry point.
#[derive(Debug)]
pub struct NewEndpointIdentity {
    id: StableEndpointId,
}

impl NewEndpointIdentity {
    pub fn allocate() -> Result<Self, ClientError> {
        // SAFETY: no borrowed pointers or COM objects; Windows returns a new
        // UUID by value. This is control work, never called from Engine render.
        let guid = unsafe { windows::Win32::System::Com::CoCreateGuid() }
            .map_err(|error| ClientError::windows("CoCreateGuid(VAD identity)", error))?;
        Ok(Self {
            id: StableEndpointId::from_le_bytes(guid.to_u128().to_le_bytes()),
        })
    }

    pub const fn endpoint_id(&self) -> VirtualEndpointId {
        self.id.to_virtual_endpoint_id()
    }

    pub(super) const fn stable_id(&self) -> StableEndpointId {
        self.id
    }
}
