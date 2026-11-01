//! Pinned native source reader. Public cursor DTOs cannot construct this handle.
use super::{NativeObservationReadHandle, NodeId, ProcessDeliverySource, serialize_node};
use crate::{
    latency::interpolation::{
        InterpolationTrace, InterpolationTraceChunk, InterpolationTraceCollector,
    },
    process_loopback::{ProcessBridgeOptions, ProcessIdentity},
};
use serde::Serialize;

#[cfg(test)]
#[path = "interpolation_trace_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize)]
pub struct NativeProcessInterpolationOrigin {
    pub schema_version: u32,
    pub owner_generation: u64,
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub process: ProcessIdentity,
    pub input_sample_rate: u32,
    pub consumer_sample_rate: u32,
    pub prepared_consumer_demand_frames: u32,
    pub admitted_bridge: ProcessBridgeOptions,
}

/// Native-source pinning is distinct from retained sequence coverage, durable
/// persistence, graph continuity, source PCM or physical latency qualification.
pub struct NativeProcessInterpolationCollector {
    source: ProcessDeliverySource,
    origin: NativeProcessInterpolationOrigin,
    collector: InterpolationTraceCollector,
    unavailable_polls: u64,
    final_history_available: bool,
}

impl NativeObservationReadHandle {
    /// Backend-local generations can be reused by a newly constructed backend.
    /// Retained immutable session identity, not an editable generation label,
    /// distinguishes replacement owners across those backend lifetimes.
    pub fn same_owner(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn process_interpolation_collector(
        &self,
        node: NodeId,
        process: &ProcessIdentity,
        generation: u64,
        limit: usize,
    ) -> Result<NativeProcessInterpolationCollector, &'static str> {
        if self.0.generation != generation {
            return Err("interpolation owner generation mismatch");
        }
        let source = self
            .0
            .process_delivery
            .iter()
            .find(|source| source.node == node && source.process.same_process(process))
            .ok_or("interpolation source identity mismatch")?;
        Ok(NativeProcessInterpolationCollector {
            source: source.clone(),
            origin: NativeProcessInterpolationOrigin {
                schema_version: 1,
                owner_generation: generation,
                node,
                process: source.process.clone(),
                input_sample_rate: source.input_sample_rate,
                consumer_sample_rate: source.consumer_sample_rate,
                prepared_consumer_demand_frames: source.prepared_consumer_demand_frames,
                admitted_bridge: source.admitted_bridge,
            },
            collector: InterpolationTraceCollector::new(limit)?,
            unavailable_polls: 0,
            final_history_available: false,
        })
    }
}

impl NativeProcessInterpolationCollector {
    pub fn origin(&self) -> &NativeProcessInterpolationOrigin {
        &self.origin
    }

    /// Control only, one snapshot attempt; registry replacement cannot redirect.
    pub fn poll(&mut self) -> bool {
        if let Some(history) = self.source.bridge.interpolation_history() {
            self.final_history_available = history.writer_finished;
            self.collector.observe(history);
            true
        } else {
            self.unavailable_polls = self.unavailable_polls.saturating_add(1);
            self.final_history_available = false;
            false
        }
    }

    pub fn unavailable_polls(&self) -> u64 {
        self.unavailable_polls
    }

    pub fn drain_chunk(&mut self) -> Option<InterpolationTraceChunk> {
        self.collector.drain_chunk()
    }

    pub fn artifact(&self) -> &InterpolationTrace {
        self.collector.artifact()
    }

    /// Join source owners first. A contended final poll cannot be certified by
    /// an earlier snapshot; leave it unfinished so callers can retry that pin.
    pub fn finish(&mut self) -> bool {
        self.poll();
        if !self.final_history_available {
            return false;
        }
        self.collector.finish();
        true
    }
}
