//! Immutable control-side project views. No prepared or realtime resources are
//! exposed here; retaining a view cannot keep an audio owner alive.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::{EngineProject, HostError, HostSnapshot};

static NEXT_HOST_ID: AtomicU64 = AtomicU64::new(1);

/// Process-local identity separating equal graph revisions in different hosts.
/// This is not a persistent project-file identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostId(u64);

/// Topology/static-configuration basis, distinct from realtime parameter edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditBasis {
    pub host_id: HostId,
    pub graph_revision: u64,
}

impl HostId {
    pub(crate) fn allocate() -> Result<Self, HostError> {
        NEXT_HOST_ID
            .try_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map(Self)
            .map_err(|_| HostError::IdentityExhausted)
    }
}

/// One coherent publication of project intent and confirmed execution state.
/// Projects are immutable, control-side copies and reused until their content
/// changes. `active` describes the confirmed plan's preparation configuration,
/// not the instantaneous DSP parameter values; consult parameter receipts for
/// application evidence.
#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    pub host_id: HostId,
    pub state: HostSnapshot,
    pub desired: Arc<EngineProject>,
    pub active: Option<Arc<EngineProject>>,
}

impl ProjectSnapshot {
    pub fn edit_basis(&self) -> EditBasis {
        EditBasis {
            host_id: self.host_id,
            graph_revision: self.state.desired_revision,
        }
    }
}
