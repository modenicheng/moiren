//! Session-local archive evidence, never persisted trusted admission.
use super::{AudioLatencyProfile, AudioSettings, SessionGeneration, TransportLatencySettings};
use moiren_core::graph::NodeId;
use moiren_windows_audio::process_loopback::{
    ObservedProcessTrace, ProcessIdentity, ProcessTraceArchive,
};

pub(crate) mod candidate;
mod evaluation;
pub use candidate::EvaluatedProcessCandidate;
#[cfg(test)]
pub(crate) mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessTraceSessionOrigin {
    pub generation: SessionGeneration,
    pub document: u64,
    pub node: NodeId,
    pub process: ProcessIdentity,
    pub native_owner_generation: u64,
    pub host_revision: u64,
    pub graph_revision: u64,
    pub audio_settings: AudioSettings,
    pub transport_latency: TransportLatencySettings,
    pub latency_profile: AudioLatencyProfile,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessTraceCollectionStatus {
    pub origin: Option<ProcessTraceSessionOrigin>,
    pub refusal: Option<String>,
}

/// One retained predecessor only. Arc reuse keeps ordinary UI publication
/// from cloning up to 8192 events per writer. An editable artifact cannot
/// replace the privately retained validated trace.
#[derive(Debug)]
pub struct ProcessTraceSessionEvidence {
    pub(crate) origin: ProcessTraceSessionOrigin,
    pub(crate) artifact: ProcessTraceArchive,
    pub(crate) refusal: Option<String>,
    pub(crate) trace: Option<ObservedProcessTrace>,
    pub(crate) measurement:
        Result<moiren_windows_audio::process_loopback::MeasuredProcessDelivery, String>,
}

impl ProcessTraceSessionEvidence {
    pub fn origin(&self) -> &ProcessTraceSessionOrigin {
        &self.origin
    }

    pub fn artifact(&self) -> &ProcessTraceArchive {
        &self.artifact
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    pub fn ordered_trace(&self) -> Result<&ObservedProcessTrace, String> {
        self.trace
            .as_ref()
            .filter(|_| self.refusal.is_none())
            .ok_or_else(|| {
                self.refusal
                    .clone()
                    .unwrap_or_else(|| "Process trace is unavailable".into())
            })
    }
}
