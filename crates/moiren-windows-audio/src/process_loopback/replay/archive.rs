//! Control-side sequence-one archival; never an enlarged realtime audio ring.
use super::{ObservedProcessTrace, ProcessIdentity, ProcessReplayError, ProcessTraceArtifact};
use crate::{
    clock_bridge::BridgeObserver,
    latency::{
        bounded_history::deserialize_bounded_events,
        packets::{PacketHistorySnapshot, PacketPublicationEvent},
        queue::{QueueHistorySnapshot, QueueReadEvent},
    },
};
use serde::{Deserialize, Deserializer, Serialize, de};

pub(super) const ARCHIVE_EVENT_LIMIT: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTraceCollectionRefusal {
    MissedEvents,
    ConflictingHistory,
    EventLimit,
}

/// Retained control-side diagnostic prefix, not authenticated native proof.
/// A refusal preserves partial observations but permanently disallows replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessTraceArchive {
    #[serde(deserialize_with = "schema_two")]
    pub schema_version: u32,
    pub process: ProcessIdentity,
    pub owner_generation: u64,
    pub input_sample_rate: u32,
    pub consumer_sample_rate: u32,
    pub prepared_consumer_demand_frames: u32,
    #[serde(deserialize_with = "archive_packets")]
    pub packet_history: PacketHistorySnapshot,
    #[serde(deserialize_with = "archive_reads")]
    pub queue_history: QueueHistorySnapshot,
    // Missing is not an implicit successful collector. Explicit null is required.
    #[serde(deserialize_with = "Option::deserialize")]
    pub collection_refusal: Option<ProcessTraceCollectionRefusal>,
}

fn schema_two<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(d)?;
    if version != 2 {
        return Err(de::Error::custom("archive schema_version must be 2"));
    }
    Ok(version)
}

fn events<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserialize_bounded_events::<D, T, ARCHIVE_EVENT_LIMIT>(d)
}

fn archive_packets<'de, D: Deserializer<'de>>(d: D) -> Result<PacketHistorySnapshot, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Raw {
        #[serde(deserialize_with = "events")]
        events: Vec<PacketPublicationEvent>,
        total_events: u64,
        overwritten_events: u64,
        offered_frames: u64,
        published_frames: u64,
        unavailable_clocks: u64,
        regressed_clocks: u64,
        saturated: bool,
        writer_finished: bool,
    }
    let r = Raw::deserialize(d)?;
    Ok(PacketHistorySnapshot {
        events: r.events,
        total_events: r.total_events,
        overwritten_events: r.overwritten_events,
        offered_frames: r.offered_frames,
        published_frames: r.published_frames,
        unavailable_clocks: r.unavailable_clocks,
        regressed_clocks: r.regressed_clocks,
        saturated: r.saturated,
        writer_finished: r.writer_finished,
    })
}

fn archive_reads<'de, D: Deserializer<'de>>(d: D) -> Result<QueueHistorySnapshot, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Raw {
        #[serde(deserialize_with = "events")]
        events: Vec<QueueReadEvent>,
        total_events: u64,
        overwritten_events: u64,
        unavailable_clocks: u64,
        regressed_clocks: u64,
        saturated: bool,
        writer_finished: bool,
    }
    let r = Raw::deserialize(d)?;
    Ok(QueueHistorySnapshot {
        events: r.events,
        total_events: r.total_events,
        overwritten_events: r.overwritten_events,
        unavailable_clocks: r.unavailable_clocks,
        regressed_clocks: r.regressed_clocks,
        saturated: r.saturated,
        writer_finished: r.writer_finished,
    })
}

impl ProcessTraceArchive {
    pub fn trace(
        &self,
        process: &ProcessIdentity,
        generation: u64,
    ) -> Result<ObservedProcessTrace, ProcessReplayError> {
        if !self.process.same_process(process) {
            return Err(ProcessReplayError::IdentityMismatch);
        }
        if self.owner_generation != generation {
            return Err(ProcessReplayError::GenerationMismatch);
        }
        if self.schema_version != 2 || self.collection_refusal.is_some() {
            return Err(ProcessReplayError::IncompleteHistory);
        }
        ObservedProcessTrace::from_complete_prefix(
            self.process.clone(),
            self.owner_generation,
            self.input_sample_rate,
            self.consumer_sample_rate,
            self.prepared_consumer_demand_frames,
            self.packet_history.clone(),
            self.queue_history.clone(),
            ARCHIVE_EVENT_LIMIT,
        )
    }
}

/// Strict versioned import. Neither variant can promote caller data to native origin.
#[derive(Debug, Clone)]
pub enum ProcessTraceDocument {
    Archive(ProcessTraceArchive),
    Legacy(ProcessTraceArtifact),
}

impl<'de> Deserialize<'de> for ProcessTraceDocument {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Untagged serde enums buffer the entire document before invoking
        // bounded vector visitors. Decode once directly, with a fixed bound.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            process: ProcessIdentity,
            owner_generation: u64,
            input_sample_rate: u32,
            consumer_sample_rate: u32,
            prepared_consumer_demand_frames: u32,
            #[serde(deserialize_with = "archive_packets")]
            packet_history: PacketHistorySnapshot,
            #[serde(deserialize_with = "archive_reads")]
            queue_history: QueueHistorySnapshot,
            #[serde(default, deserialize_with = "present_refusal")]
            collection_refusal: Option<Option<ProcessTraceCollectionRefusal>>,
        }
        let a = Wire::deserialize(d)?;
        match (a.schema_version, a.collection_refusal) {
            (1, None)
                if a.packet_history.events.len() <= 256 && a.queue_history.events.len() <= 256 =>
            {
                Ok(Self::Legacy(ProcessTraceArtifact {
                    schema_version: 1,
                    process: a.process,
                    owner_generation: a.owner_generation,
                    input_sample_rate: a.input_sample_rate,
                    consumer_sample_rate: a.consumer_sample_rate,
                    prepared_consumer_demand_frames: a.prepared_consumer_demand_frames,
                    packet_history: a.packet_history,
                    queue_history: a.queue_history,
                }))
            }
            (2, Some(collection_refusal)) => Ok(Self::Archive(ProcessTraceArchive {
                schema_version: 2,
                process: a.process,
                owner_generation: a.owner_generation,
                input_sample_rate: a.input_sample_rate,
                consumer_sample_rate: a.consumer_sample_rate,
                prepared_consumer_demand_frames: a.prepared_consumer_demand_frames,
                packet_history: a.packet_history,
                queue_history: a.queue_history,
                collection_refusal,
            })),
            _ => Err(de::Error::custom(
                "invalid Process trace schema, collection marker or event count",
            )),
        }
    }
}

fn present_refusal<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<ProcessTraceCollectionRefusal>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

impl ProcessTraceDocument {
    pub fn process(&self) -> &ProcessIdentity {
        match self {
            Self::Archive(a) => &a.process,
            Self::Legacy(a) => &a.process,
        }
    }
    pub fn owner_generation(&self) -> u64 {
        match self {
            Self::Archive(a) => a.owner_generation,
            Self::Legacy(a) => a.owner_generation,
        }
    }
    pub fn trace(
        &self,
        process: &ProcessIdentity,
        generation: u64,
    ) -> Result<ObservedProcessTrace, ProcessReplayError> {
        match self {
            Self::Archive(a) => a.trace(process, generation),
            Self::Legacy(a) => a.trace(process, generation),
        }
    }
}

/// Control-only accumulator. It records archive faults and keeps collecting
/// final closure/counters; it never requests audio cancellation or retries RT.
pub struct ProcessTraceCollector {
    artifact: ProcessTraceArchive,
}

impl ProcessTraceCollector {
    pub fn new(
        process: ProcessIdentity,
        generation: u64,
        input_rate: u32,
        consumer_rate: u32,
        prepared_demand: u32,
    ) -> Self {
        Self {
            artifact: ProcessTraceArchive {
                schema_version: 2,
                process,
                owner_generation: generation,
                input_sample_rate: input_rate,
                consumer_sample_rate: consumer_rate,
                prepared_consumer_demand_frames: prepared_demand,
                packet_history: PacketHistorySnapshot {
                    events: Vec::new(),
                    total_events: 0,
                    overwritten_events: 0,
                    offered_frames: 0,
                    published_frames: 0,
                    unavailable_clocks: 0,
                    regressed_clocks: 0,
                    saturated: false,
                    writer_finished: false,
                },
                queue_history: QueueHistorySnapshot {
                    events: Vec::new(),
                    total_events: 0,
                    overwritten_events: 0,
                    unavailable_clocks: 0,
                    regressed_clocks: 0,
                    saturated: false,
                    writer_finished: false,
                },
                collection_refusal: None,
            },
        }
    }

    pub fn poll(&mut self, observer: &BridgeObserver) {
        if let Some(snapshot) = observer.packet_history() {
            self.packets(snapshot);
        }
        if let Some(snapshot) = observer.queue_history() {
            self.reads(snapshot);
        }
    }

    pub fn artifact(&self) -> ProcessTraceArchive {
        self.artifact.clone()
    }

    /// Constant-size control-side status; never clone a growing archive merely
    /// to discover that a missed/capped prefix has permanently refused replay.
    pub fn collection_refusal(&self) -> Option<ProcessTraceCollectionRefusal> {
        self.artifact.collection_refusal
    }

    pub fn process(&self) -> &ProcessIdentity {
        &self.artifact.process
    }

    pub fn owner_generation(&self) -> u64 {
        self.artifact.owner_generation
    }

    fn packets(&mut self, mut next: PacketHistorySnapshot) {
        let previous = &mut self.artifact.packet_history;
        let conflict = next.total_events < previous.total_events
            || next.offered_frames < previous.offered_frames
            || next.published_frames < previous.published_frames
            || next.unavailable_clocks < previous.unavailable_clocks
            || next.regressed_clocks < previous.regressed_clocks
            || (previous.saturated && !next.saturated)
            || (previous.writer_finished && next != final_suffix(previous));
        merge(
            &mut previous.events,
            &next.events,
            next.total_events,
            conflict,
            &mut self.artifact.collection_refusal,
            |e| e.sequence,
        );
        std::mem::swap(&mut previous.events, &mut next.events);
        next.overwritten_events = next.total_events.saturating_sub(next.events.len() as u64);
        *previous = next;
    }

    fn reads(&mut self, mut next: QueueHistorySnapshot) {
        let previous = &mut self.artifact.queue_history;
        let conflict = next.total_events < previous.total_events
            || next.unavailable_clocks < previous.unavailable_clocks
            || next.regressed_clocks < previous.regressed_clocks
            || (previous.saturated && !next.saturated)
            || (previous.writer_finished && next != final_read_suffix(previous));
        merge(
            &mut previous.events,
            &next.events,
            next.total_events,
            conflict,
            &mut self.artifact.collection_refusal,
            |e| e.sequence,
        );
        std::mem::swap(&mut previous.events, &mut next.events);
        next.overwritten_events = next.total_events.saturating_sub(next.events.len() as u64);
        *previous = next;
    }
}

// A closed native ring can only be observed again as its identical final suffix.
fn final_suffix(a: &PacketHistorySnapshot) -> PacketHistorySnapshot {
    let mut a = a.clone();
    a.events = a.events.into_iter().rev().take(256).collect::<Vec<_>>();
    a.events.reverse();
    a.overwritten_events = a.total_events.saturating_sub(a.events.len() as u64);
    a
}
fn final_read_suffix(a: &QueueHistorySnapshot) -> QueueHistorySnapshot {
    let mut a = a.clone();
    a.events = a.events.into_iter().rev().take(256).collect::<Vec<_>>();
    a.events.reverse();
    a.overwritten_events = a.total_events.saturating_sub(a.events.len() as u64);
    a
}

fn merge<T: Copy + PartialEq>(
    retained: &mut Vec<T>,
    suffix: &[T],
    total: u64,
    conflict: bool,
    refusal: &mut Option<ProcessTraceCollectionRefusal>,
    sequence: impl Fn(&T) -> u64,
) {
    if conflict {
        refusal.get_or_insert(ProcessTraceCollectionRefusal::ConflictingHistory);
    }
    if refusal.is_some() {
        return;
    }
    for event in suffix {
        let seq = sequence(event);
        if seq == 0 || seq > total {
            *refusal = Some(ProcessTraceCollectionRefusal::ConflictingHistory);
            return;
        }
        if seq <= retained.len() as u64 {
            if retained[seq as usize - 1] != *event {
                *refusal = Some(ProcessTraceCollectionRefusal::ConflictingHistory);
                return;
            }
        } else if seq != retained.len() as u64 + 1 {
            *refusal = Some(ProcessTraceCollectionRefusal::MissedEvents);
            return;
        } else if retained.len() == ARCHIVE_EVENT_LIMIT {
            *refusal = Some(ProcessTraceCollectionRefusal::EventLimit);
            return;
        } else {
            retained.push(*event);
        }
    }
    if retained.len() as u64 != total {
        *refusal = Some(ProcessTraceCollectionRefusal::MissedEvents);
    }
}

#[cfg(test)]
#[path = "archive/tests.rs"]
mod tests;
