//! Fresh-allocation authority stays in memory, outside graph/project data.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
pub(crate) struct FreshExportIdentity {
    #[cfg(windows)]
    native: moiren_windows_audio::vad::NewEndpointIdentity,
    #[cfg(not(windows))]
    id: VirtualEndpointId,
    revoked: AtomicBool,
}

impl FreshExportIdentity {
    fn allocate() -> Result<Self, String> {
        Ok(Self {
            #[cfg(windows)]
            native: moiren_windows_audio::vad::NewEndpointIdentity::allocate()
                .map_err(|error| error.to_string())?,
            #[cfg(not(windows))]
            id: VirtualEndpointId::from_bytes(*gpui_kit::accesskit::Uuid::new_v4().as_bytes()),
            revoked: AtomicBool::new(false),
        })
    }

    pub(crate) fn endpoint_id(&self) -> VirtualEndpointId {
        #[cfg(windows)]
        {
            self.native.endpoint_id()
        }
        #[cfg(not(windows))]
        {
            self.id
        }
    }

    pub(crate) fn valid_for(&self, id: VirtualEndpointId) -> bool {
        self.endpoint_id() == id && !self.revoked.load(Ordering::Acquire)
    }

    #[cfg(windows)]
    pub(crate) fn native(&self) -> &moiren_windows_audio::vad::NewEndpointIdentity {
        &self.native
    }

    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}

impl ExportService {
    pub(in crate::service) fn allocate_export(
        &mut self,
        graph: &mut crate::service::ServiceGraph,
        direction: ExportDirection,
        name: String,
        channels: usize,
    ) -> Result<(), String> {
        validate_export_name_channels(&name, channels)?;
        let snapshot = graph.snapshot();
        if snapshot.exports.len() >= MAX_EXPORTS || self.new_identities.len() >= 4096 {
            return Err("virtual export or retained allocation limit reached".into());
        }
        let identity = Arc::new(FreshExportIdentity::allocate()?);
        let id = identity.endpoint_id();
        let exports = append_export(&snapshot, id, direction, &name, channels)?;
        // Failed validation never publishes a token or partially changes intent.
        graph.edit(crate::service::GraphEdit::SetExports(exports))?;
        self.new_identities.insert(id, identity);
        Ok(())
    }

    pub(in crate::service) fn clear_new_identities(&mut self) {
        // Queued workers may retain an Arc after a document was replaced.
        // Revoke that authority too; dropping this map alone is insufficient.
        for identity in self.new_identities.values() {
            identity.revoke();
        }
        self.new_identities.clear();
    }
}
