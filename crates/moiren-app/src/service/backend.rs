//! Control-side catalog contract for native adapters.
use super::DeviceCatalog;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{stage}: {reason}")]
pub struct BackendError {
    pub stage: &'static str,
    pub reason: String,
}
impl BackendError {
    pub fn new(stage: &'static str, reason: impl ToString) -> Self {
        Self {
            stage,
            reason: reason.to_string(),
        }
    }
}

/// Device enumeration source injected into ManagedDriver. The retired legacy
/// driver carried its whole prepare/activate lifecycle through this trait;
/// only the catalog contract survives it.
pub trait CatalogBackend: Send + Sync + 'static {
    fn catalog(&self) -> Result<DeviceCatalog, BackendError>;
}
