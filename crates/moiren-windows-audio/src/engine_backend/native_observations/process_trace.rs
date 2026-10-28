//! Opaque provenance boundary around the public, untrusted archive format.
use super::{NativeObservationReadHandle, NodeId};
use crate::{
    clock_bridge::BridgeObserver,
    process_loopback::{
        ObservedProcessTrace, ProcessIdentity, ProcessReplayError, ProcessTraceArchive,
        ProcessTraceCollectionRefusal, ProcessTraceCollector,
    },
};

/// Control-side sequence-one archive pinned to one native source. It retains
/// observation storage only, never native audio resources or history writers.
/// Missing/capped prefixes remain refusals; collection never stops audio.
pub struct NativeProcessTraceCollector {
    bridge: BridgeObserver,
    collector: ProcessTraceCollector,
}

impl NativeObservationReadHandle {
    /// Authenticate the source once and poll its current suffix immediately.
    /// Later registry replacement cannot redirect this collector. Call early
    /// enough to retain sequence one, then poll within the native ring window.
    pub fn process_trace_collector(
        &self,
        node: NodeId,
        expected_process: &ProcessIdentity,
        expected_generation: u64,
    ) -> Result<NativeProcessTraceCollector, ProcessReplayError> {
        if self.0.generation != expected_generation {
            return Err(ProcessReplayError::GenerationMismatch);
        }
        let source = self
            .0
            .process_delivery
            .iter()
            .find(|source| source.node == node)
            .ok_or(ProcessReplayError::InvalidIdentity)?;
        if !source.process.same_process(expected_process) {
            return Err(ProcessReplayError::IdentityMismatch);
        }
        let mut collector = NativeProcessTraceCollector {
            bridge: source.bridge.clone(),
            collector: ProcessTraceCollector::new(
                source.process.clone(),
                self.0.generation,
                source.input_sample_rate,
                source.consumer_sample_rate,
                source.prepared_consumer_demand_frames,
            ),
        };
        collector.poll();
        Ok(collector)
    }
}

impl NativeProcessTraceCollector {
    pub fn process(&self) -> &ProcessIdentity {
        self.collector.process()
    }

    pub fn owner_generation(&self) -> u64 {
        self.collector.owner_generation()
    }

    pub fn collection_refusal(&self) -> Option<ProcessTraceCollectionRefusal> {
        self.collector.collection_refusal()
    }

    /// Control/cleanup workers only; does not wait for a contended RT writer.
    pub fn poll(&mut self) {
        self.collector.poll(&self.bridge);
    }

    /// Exported/edited artifacts have caller-supplied provenance, regardless
    /// of where they were originally collected.
    pub fn artifact(&self) -> ProcessTraceArchive {
        self.collector.artifact()
    }

    /// Requires complete, closed writers. This authenticates origin, not clean
    /// teardown, waveform continuity, future-safe admission or physical delay.
    pub fn ordered_trace(&self) -> Result<ObservedProcessTrace, ProcessReplayError> {
        let artifact = self.collector.artifact();
        Ok(artifact
            .trace(&artifact.process, artifact.owner_generation)?
            .with_retained_owner_origin())
    }
}
