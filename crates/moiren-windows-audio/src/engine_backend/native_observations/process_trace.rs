//! Opaque provenance boundary around the public, untrusted archive format.
use super::{NativeObservationReadHandle, NodeId, ProcessDeliverySource};
use crate::process_loopback::{
    MeasuredProcessDelivery, ObservedProcessTrace, ProcessIdentity, ProcessReplayError,
    ProcessTraceArchive, ProcessTraceCollectionRefusal, ProcessTraceCollector,
};

/// Control-side sequence-one archive pinned to one native source. It retains
/// observation storage only, never native audio resources or history writers.
/// Missing/capped prefixes remain refusals; collection never stops audio.
pub struct NativeProcessTraceCollector {
    source: ProcessDeliverySource,
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
            source: source.clone(),
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
        self.collector.poll(&self.source.bridge);
    }

    /// Read the original source's final complete-drain/whole-wake evidence.
    /// Call after the owning host joins; registry retirement alone is not
    /// closure. Queue replay remains independently validated and unqualified.
    pub fn final_measurement(&self) -> Result<MeasuredProcessDelivery, String> {
        self.ordered_trace().map_err(|error| error.to_string())?;
        let source = &self.source;
        let counters = source.bridge.snapshot();
        let delivery = counters
            .native_delivery
            .ok_or("final Process delivery observations are missing")?;
        let measured = MeasuredProcessDelivery::from_observations(
            source.process.clone(),
            self.owner_generation(),
            source.input_sample_rate,
            source.consumer_sample_rate,
            source.prepared_consumer_demand_frames,
            delivery,
            source
                .bridge
                .delivery_events()
                .ok_or("final Process drain observations are missing")?,
            source
                .consumer
                .as_ref()
                .and_then(|observer| observer.snapshot())
                .ok_or("final Process whole-wake observations are missing")?,
        )
        .map_err(|error| error.to_string())?;
        // Packet publication is bracketed, while delivery/drain observations
        // use point timestamps. Exact replay must fit the conservative outer
        // envelope too, including rounding at native frame boundaries.
        let artifact = self.collector.artifact();
        let mut gap = 0;
        for pair in artifact.packet_history.events.windows(2) {
            let outer = pair[1]
                .sample
                .qpc_after_100ns
                .zip(pair[0].sample.qpc_before_100ns)
                .and_then(|(after, before)| after.checked_sub(before))
                .ok_or("final Process publication envelope is unavailable or regressed")?;
            gap = gap.max(outer);
        }
        measured
            .with_publication_gap_100ns(gap)
            .map_err(|error| error.to_string())
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
