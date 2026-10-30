//! Finite live-prefix assessment; no writer closure, future guarantee or age.
use super::{
    AssessedProcessReplay, ObservedProcessTrace, ProcessBridgeOptions, ProcessIdentity,
    ProcessReplayError, ProcessTraceArchive, ProcessTraceCollectionRefusal, ProcessTraceOrigin,
    ReplayWindowPolicy, ReplayWindowStatus,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LiveProcessPrefix {
    pub cutoff_100ns: u64,
    pub observed_packet_events: u64,
    pub observed_read_events: u64,
    pub replayed_packet_events: u64,
    pub replayed_read_events: u64,
    pub packet_seal_before_100ns: u64,
    pub read_seal_before_100ns: u64,
    pub packet_writer_finished: bool,
    pub read_writer_finished: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveProcessWindowReport {
    pub schema_version: u32,
    pub qualified: bool,
    pub process: ProcessIdentity,
    pub owner_generation: u64,
    pub options: ProcessBridgeOptions,
    pub policy: ReplayWindowPolicy,
    pub status: ReplayWindowStatus,
    pub trace_origin: ProcessTraceOrigin,
    pub collection_refusal: Option<ProcessTraceCollectionRefusal>,
    pub prefix: Option<LiveProcessPrefix>,
    pub replay: Option<AssessedProcessReplay>,
    pub refusal: Option<String>,
}

impl ProcessTraceArchive {
    /// Assess only the immutable common prefix sealed by both retained writers.
    /// Editable archives never authenticate native provenance. Incomplete or
    /// ambiguous evidence returns inconclusive, not a successful empty replay.
    pub fn replay_live_window(
        &self,
        process: &ProcessIdentity,
        generation: u64,
        options: ProcessBridgeOptions,
        policy: ReplayWindowPolicy,
    ) -> Result<LiveProcessWindowReport, ProcessReplayError> {
        self.assess_live_prefix(
            process,
            generation,
            options,
            policy,
            ProcessTraceOrigin::CallerSupplied,
        )
    }

    // Only the Windows backend can authenticate a retained native owner.
    #[cfg(windows)]
    pub(crate) fn replay_retained_live_window(
        &self,
        process: &ProcessIdentity,
        generation: u64,
        options: ProcessBridgeOptions,
        policy: ReplayWindowPolicy,
    ) -> Result<LiveProcessWindowReport, ProcessReplayError> {
        self.assess_live_prefix(
            process,
            generation,
            options,
            policy,
            ProcessTraceOrigin::RetainedNativeOwner,
        )
    }

    fn assess_live_prefix(
        &self,
        process: &ProcessIdentity,
        generation: u64,
        options: ProcessBridgeOptions,
        policy: ReplayWindowPolicy,
        origin: ProcessTraceOrigin,
    ) -> Result<LiveProcessWindowReport, ProcessReplayError> {
        policy.validate()?;
        if !self.process.same_process(process) {
            return Err(ProcessReplayError::IdentityMismatch);
        }
        if self.owner_generation != generation {
            return Err(ProcessReplayError::GenerationMismatch);
        }
        let mut report = LiveProcessWindowReport {
            schema_version: 1,
            qualified: false,
            process: self.process.clone(),
            owner_generation: self.owner_generation,
            options,
            policy,
            status: ReplayWindowStatus::Inconclusive,
            trace_origin: origin,
            collection_refusal: self.collection_refusal,
            prefix: None,
            replay: None,
            refusal: None,
        };
        let result = (|| {
            if self.schema_version != 2
                || self.collection_refusal.is_some()
                || self.packet_history.events.len() > super::archive::ARCHIVE_EVENT_LIMIT
                || self.queue_history.events.len() > super::archive::ARCHIVE_EVENT_LIMIT
            {
                return Err(ProcessReplayError::IncompleteHistory);
            }
            // Validate the complete retained writer payloads, including the
            // seal events. Open tails may legitimately overlap or observe a
            // frontier beyond the other snapshot. Only those cross-writer
            // conditions can be deferred to the trimmed prefix below.
            match self.sequence_prefix(self.packet_history.clone(), self.queue_history.clone()) {
                Ok(_)
                | Err(
                    ProcessReplayError::AmbiguousOrder
                    | ProcessReplayError::InconsistentPublication,
                ) => {}
                Err(error) => return Err(error),
            }
            let mut packets = self.packet_history.clone();
            let mut reads = self.queue_history.clone();
            let packet_seal = packets
                .events
                .last()
                .ok_or(ProcessReplayError::IncompleteHistory)?
                .sample
                .qpc_before_100ns
                .ok_or(ProcessReplayError::ClockFault)?;
            let read_seal = reads
                .events
                .last()
                .ok_or(ProcessReplayError::IncompleteHistory)?
                .sample
                .qpc_before_100ns
                .ok_or(ProcessReplayError::ClockFault)?;
            let cutoff = packet_seal.min(read_seal);
            // The latest completed event on each sole writer is a sentinel.
            // Future events cannot begin before its validated clock horizon.
            // Strict end<cutoff also excludes ties and any in-flight tail.
            packets.events.retain(|event| {
                event
                    .sample
                    .qpc_after_100ns
                    .is_some_and(|after| after < cutoff)
            });
            reads.events.retain(|event| {
                event
                    .sample
                    .qpc_after_100ns
                    .is_some_and(|after| after < cutoff)
            });
            let prefix = LiveProcessPrefix {
                cutoff_100ns: cutoff,
                observed_packet_events: self.packet_history.total_events,
                observed_read_events: self.queue_history.total_events,
                replayed_packet_events: packets.events.len() as u64,
                replayed_read_events: reads.events.len() as u64,
                packet_seal_before_100ns: packet_seal,
                read_seal_before_100ns: read_seal,
                packet_writer_finished: self.packet_history.writer_finished,
                read_writer_finished: self.queue_history.writer_finished,
            };
            report.prefix = Some(prefix);
            packets.total_events = packets.events.len() as u64;
            packets.offered_frames = packets
                .events
                .iter()
                .map(|event| event.sample.offered_frames)
                .sum();
            packets.published_frames = packets
                .events
                .iter()
                .map(|event| event.sample.accepted_frames)
                .sum();
            reads.total_events = reads.events.len() as u64;
            // These are bounded prefix extents, not the original writers'
            // terminal snapshots. Never mark the extracted histories closed.
            packets.writer_finished = false;
            reads.writer_finished = false;
            let mut trace = self.sequence_prefix(packets, reads)?;
            trace.origin = origin;
            trace.replay_window(process, generation, options, policy)
        })();
        match result {
            Ok(replay) => {
                report.status = replay.assessment.status;
                report.replay = Some(replay);
            }
            Err(error) => report.refusal = Some(match error {
                ProcessReplayError::IncompleteHistory =>
                    "live prefix requires complete sequence-one histories within the retained event bound".into(),
                error => error.to_string(),
            }),
        }
        Ok(report)
    }

    fn sequence_prefix(
        &self,
        packets: crate::latency::packets::PacketHistorySnapshot,
        reads: crate::latency::queue::QueueHistorySnapshot,
    ) -> Result<ObservedProcessTrace, ProcessReplayError> {
        ObservedProcessTrace::from_sequence_prefix(
            self.process.clone(),
            self.owner_generation,
            self.input_sample_rate,
            self.consumer_sample_rate,
            self.prepared_consumer_demand_frames,
            packets,
            reads,
            super::archive::ARCHIVE_EVENT_LIMIT,
            false,
        )
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;
