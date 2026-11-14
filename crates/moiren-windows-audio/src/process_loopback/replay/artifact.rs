//! Versioned caller-supplied diagnostic artifact, never imported native proof.
use super::{ObservedProcessTrace, ProcessIdentity, ProcessReplayError};
use crate::latency::{packets::PacketHistorySnapshot, queue::QueueHistorySnapshot};
use serde::{Deserialize, Serialize};

/// Raw bounded diagnostics, including incomplete evidence. Successful decoding
/// is not validation, closure, authentic source identity or successful cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessTraceArtifact {
    pub schema_version: u32,
    pub process: ProcessIdentity,
    pub owner_generation: u64,
    pub input_sample_rate: u32,
    pub consumer_sample_rate: u32,
    pub prepared_consumer_demand_frames: u32,
    pub packet_history: PacketHistorySnapshot,
    pub queue_history: QueueHistorySnapshot,
}

impl ProcessTraceArtifact {
    pub fn trace(
        &self,
        expected_process: &ProcessIdentity,
        expected_generation: u64,
    ) -> Result<ObservedProcessTrace, ProcessReplayError> {
        if self.schema_version != 1 {
            return Err(ProcessReplayError::InvalidHistory);
        }
        if !self.process.same_process(expected_process) {
            return Err(ProcessReplayError::IdentityMismatch);
        }
        if self.owner_generation != expected_generation {
            return Err(ProcessReplayError::GenerationMismatch);
        }
        ObservedProcessTrace::from_observations(
            self.process.clone(),
            self.owner_generation,
            self.input_sample_rate,
            self.consumer_sample_rate,
            self.prepared_consumer_demand_frames,
            self.packet_history.clone(),
            self.queue_history.clone(),
        )
    }
}

#[cfg(test)]
#[path = "artifact/tests.rs"]
mod tests;
