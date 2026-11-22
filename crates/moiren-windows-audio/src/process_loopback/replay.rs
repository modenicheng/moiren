//! Finite ordered boundary-request replay, never production admission or age.
use super::{ProcessBridgeOptions, ProcessIdentity};
use crate::{
    capture::CaptureError,
    clock_bridge::{CapturePacket, capture_bridge},
    latency::{
        packets::{PacketHistorySnapshot, PacketPublicationOrigin, packet_history},
        queue::{QueueHistorySnapshot, queue_history},
    },
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};
use serde::Serialize;
use thiserror::Error;

#[path = "replay/artifact.rs"]
mod artifact;
pub use artifact::ProcessTraceArtifact;
#[path = "replay/archive.rs"]
mod archive;
pub use archive::{
    ProcessTraceArchive, ProcessTraceCollectionRefusal, ProcessTraceCollector, ProcessTraceDocument,
};
#[path = "replay/inventory.rs"]
mod inventory;
pub use inventory::ReplayInventoryReport;
#[path = "replay/window.rs"]
mod window;
pub use window::{
    AssessedProcessReplay, ReplayWindowAssessment, ReplayWindowPolicy, ReplayWindowStatus,
    ReplayWindowViolation,
};
#[path = "replay/live.rs"]
mod live;
pub use live::{LiveProcessPrefix, LiveProcessWindowReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProcessReplayError {
    #[error("Process replay requires a valid source identity and owner generation")]
    InvalidIdentity,
    #[error("Process replay identity does not match the requested source")]
    IdentityMismatch,
    #[error("Process replay owner generation does not match the requested generation")]
    GenerationMismatch,
    #[error("Process replay requires closed, complete, unsaturated histories from sequence one")]
    IncompleteHistory,
    #[error("Process replay history has malformed counts, conservation or provenance")]
    InvalidHistory,
    #[error("Process replay requires known, nonregressing host-QPC envelopes")]
    ClockFault,
    #[error(
        "Process replay cannot assess host-ingress elapsed enforcement without native freshness observations"
    )]
    IngressElapsedUnsupported,
    #[error("Process packet/read envelopes overlap or tie; order is ambiguous")]
    AmbiguousOrder,
    #[error("Process read publication frontiers disagree with admitted packet prefixes")]
    InconsistentPublication,
    #[error("Process replay format, prepared demand or bounded work limit is unsupported")]
    UnsupportedShape,
    #[error(
        "finite replay policy requires positive output frames/span and at least two output reads"
    )]
    InvalidWindowPolicy,
    #[error(transparent)]
    Candidate(#[from] CaptureError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessReplayStatus {
    Replayed,
}

/// How identity labels reached this trace, not a hardware qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTraceOrigin {
    CallerSupplied,
    RetainedNativeOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferedSupplyAssumption {
    CompletePacketAtObservedOrder,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct ReplayBaselineReport {
    pub offered_frames: u64,
    pub published_frames: u64,
    pub dropped_frames: u64,
    pub requested_frames: u64,
    pub output_frames: u64,
    pub underrun_frames: u64,
    pub prime_discarded_frames: u64,
    pub backlog_discarded_frames: u64,
    pub age_discarded_frames: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub ingress_elapsed_discarded_frames: u64,
    pub reset_discarded_frames: u64,
    pub resets: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessReplayReport {
    pub status: ProcessReplayStatus,
    pub qualified: bool,
    pub trace_origin: ProcessTraceOrigin,
    pub process: ProcessIdentity,
    pub owner_generation: u64,
    pub options: ProcessBridgeOptions,
    pub window_start_100ns: u64,
    pub window_end_100ns: u64,
    pub packet_events: u64,
    pub read_events: u64,
    pub offered_supply_assumption: OfferedSupplyAssumption,
    pub start_inventory_frames: u64,
    pub terminal_inventory_frames: u64,
    pub requested_frames: u64,
    pub startup_unserved_frames: u64,
    pub unserved_frames: u64,
    pub baseline: ReplayBaselineReport,
    pub candidate: ReplayQueueReport,
    pub inventory: ReplayInventoryReport,
}

/// Queue mechanics only. Synthetic replay samples have no measured content,
/// sample clocks, signal continuity or presentation provenance.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReplayQueueReport {
    pub captured_frames: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub output_frames: u64,
    pub priming_frames: u64,
    pub underrun_frames: u64,
    pub prime_discarded_frames: u64,
    pub backlog_discarded_frames: u64,
    pub backlog_recoveries: u64,
    pub age_discarded_frames: u64,
    /// Native input retired from both the ring and held interpolation state.
    pub retired_input_frames: u64,
    pub consumed_input_frames: u64,
    pub reset_discarded_frames: u64,
    pub resets: u64,
    pub min_fill_frames: usize,
    pub max_fill_frames: usize,
    pub correction_ppm: f64,
}

/// Structurally validated immutable trace. The public constructor checks caller
/// supplied labels, not authentic native provenance. Only a retained native
/// owner can provide that origin; no persisted JSON becomes trusted evidence.
/// Closure refers to both unique history writers, not transport cleanup.
#[derive(Debug, Clone)]
pub struct ObservedProcessTrace {
    process: ProcessIdentity,
    owner_generation: u64,
    origin: ProcessTraceOrigin,
    consumer_rate: u32,
    prepared_demand: u32,
    packets: PacketHistorySnapshot,
    reads: QueueHistorySnapshot,
    order: Vec<OrderedEvent>,
}

const MAX_EVENT_FRAMES: u64 = 65_536;
const MAX_WORK_FRAMES: u64 = 1_048_576;

#[derive(Debug, Clone, Copy)]
enum OrderedEvent {
    Packet(usize),
    Read(usize),
}

impl OrderedEvent {
    fn bounds(self, packets: &PacketHistorySnapshot, reads: &QueueHistorySnapshot) -> (u64, u64) {
        match self {
            Self::Packet(index) => {
                let sample = packets.events[index].sample;
                (
                    sample.qpc_before_100ns.unwrap(),
                    sample.qpc_after_100ns.unwrap(),
                )
            }
            Self::Read(index) => {
                let sample = reads.events[index].sample;
                (
                    sample.qpc_before_100ns.unwrap(),
                    sample.qpc_after_100ns.unwrap(),
                )
            }
        }
    }
}

impl ObservedProcessTrace {
    /// Validate caller-supplied counts, order and identity-label consistency.
    /// This cannot authenticate where public snapshots or labels originated.
    #[allow(clippy::too_many_arguments)]
    pub fn from_observations(
        process: ProcessIdentity,
        owner_generation: u64,
        input_rate: u32,
        consumer_rate: u32,
        prepared_demand: u32,
        packets: PacketHistorySnapshot,
        reads: QueueHistorySnapshot,
    ) -> Result<Self, ProcessReplayError> {
        Self::from_complete_prefix(
            process,
            owner_generation,
            input_rate,
            consumer_rate,
            prepared_demand,
            packets,
            reads,
            256,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_complete_prefix(
        process: ProcessIdentity,
        owner_generation: u64,
        input_rate: u32,
        consumer_rate: u32,
        prepared_demand: u32,
        packets: PacketHistorySnapshot,
        reads: QueueHistorySnapshot,
        event_limit: usize,
    ) -> Result<Self, ProcessReplayError> {
        Self::from_sequence_prefix(
            process,
            owner_generation,
            input_rate,
            consumer_rate,
            prepared_demand,
            packets,
            reads,
            event_limit,
            true,
        )
    }

    /// Internal live assessment also validates sequence one, conservation and
    /// canonical writer behavior. Only its separately sealed prefix may use an
    /// open history; public closed-trace constructors still require closure.
    #[allow(clippy::too_many_arguments)]
    fn from_sequence_prefix(
        process: ProcessIdentity,
        owner_generation: u64,
        input_rate: u32,
        consumer_rate: u32,
        prepared_demand: u32,
        packets: PacketHistorySnapshot,
        reads: QueueHistorySnapshot,
        event_limit: usize,
        require_finished: bool,
    ) -> Result<Self, ProcessReplayError> {
        if process.pid == 0 || process.creation_time_100ns == 0 || owner_generation == 0 {
            return Err(ProcessReplayError::InvalidIdentity);
        }
        if input_rate != crate::NATIVE_SAMPLE_RATE
            || prepared_demand == 0
            || u64::from(prepared_demand) > MAX_EVENT_FRAMES
            || !moiren_engine::audio_config::SAMPLE_RATES.contains(&consumer_rate)
        {
            return Err(ProcessReplayError::UnsupportedShape);
        }
        complete_history(
            packets.writer_finished,
            packets.total_events,
            packets.overwritten_events,
            packets.events.len(),
            packets.saturated,
            event_limit,
            require_finished,
        )?;
        complete_history(
            reads.writer_finished,
            reads.total_events,
            reads.overwritten_events,
            reads.events.len(),
            reads.saturated,
            event_limit,
            require_finished,
        )?;
        if packets.unavailable_clocks != 0
            || packets.regressed_clocks != 0
            || reads.unavailable_clocks != 0
            || reads.regressed_clocks != 0
        {
            return Err(ProcessReplayError::ClockFault);
        }
        let (mut writer, observer) = packet_history();
        let mut expected_generation = 1_u64;
        let mut work = 0_u64;
        let work_limit = if event_limit == 256 {
            MAX_WORK_FRAMES
        } else {
            16_777_216
        };
        for (index, event) in packets.events.iter().enumerate() {
            let sample = event.sample;
            validate_clock(sample.qpc_before_100ns, sample.qpc_after_100ns)?;
            if sample.origin != PacketPublicationOrigin::NativeCapture
                || sample.flags & !(DISCONTINUITY | SILENT | TIMESTAMP_ERROR) != 0
            {
                return Err(ProcessReplayError::InvalidHistory);
            }
            if sample.flags & DISCONTINUITY != 0 {
                expected_generation += 1;
            }
            if sample.buffer_generation != expected_generation {
                return Err(ProcessReplayError::InvalidHistory);
            }
            if sample.accepted_frames < sample.offered_frames {
                expected_generation += 1;
            }
            add_work(&mut work, sample.offered_frames, work_limit)?;
            writer.record(sample);
            // Validate every prefix before the fixed native ring overwrites
            // it. Comparing only the final suffix would trust earlier edits.
            if (index + 1).is_multiple_of(256) {
                let canonical = observer
                    .snapshot()
                    .ok_or(ProcessReplayError::InvalidHistory)?;
                if canonical.events != packets.events[index + 1 - 256..=index] {
                    return Err(ProcessReplayError::InvalidHistory);
                }
            }
        }
        drop(writer);
        let mut canonical = observer
            .snapshot()
            .ok_or(ProcessReplayError::InvalidHistory)?;
        // This temporary validation writer has been dropped, whereas the
        // observed native writer may still be open. Compare its payload and
        // counters without inventing native closure; preserve the input flag.
        canonical.writer_finished = packets.writer_finished;
        let mut expected = packets.clone();
        expected.events = expected.events[expected.events.len().saturating_sub(256)..].to_vec();
        expected.overwritten_events = expected.total_events - expected.events.len() as u64;
        if canonical != expected {
            return Err(ProcessReplayError::InvalidHistory);
        }
        let (mut writer, observer) = queue_history();
        let mut reset_total = 0_u64;
        for (index, event) in reads.events.iter().enumerate() {
            let sample = event.sample;
            validate_clock(sample.qpc_before_100ns, sample.qpc_after_100ns)?;
            add_work(&mut work, sample.requested_frames, work_limit)?;
            if sample.requested_frames > u64::from(prepared_demand) {
                return Err(ProcessReplayError::UnsupportedShape);
            }
            reset_total = reset_total
                .checked_add(sample.resets)
                .ok_or(ProcessReplayError::InvalidHistory)?;
            writer.record(sample);
            if (index + 1).is_multiple_of(256) {
                let canonical = observer
                    .snapshot()
                    .ok_or(ProcessReplayError::InvalidHistory)?;
                if canonical.events != reads.events[index + 1 - 256..=index] {
                    return Err(ProcessReplayError::InvalidHistory);
                }
            }
        }
        drop(writer);
        let mut canonical = observer
            .snapshot()
            .ok_or(ProcessReplayError::InvalidHistory)?;
        canonical.writer_finished = reads.writer_finished;
        let mut expected = reads.clone();
        expected.events = expected.events[expected.events.len().saturating_sub(256)..].to_vec();
        expected.overwritten_events = expected.total_events - expected.events.len() as u64;
        if canonical != expected {
            return Err(ProcessReplayError::InvalidHistory);
        }
        let first = reads.events[0].sample.before;
        if first.removed_frames != 0 || first.held_frames != 0 {
            return Err(ProcessReplayError::InconsistentPublication);
        }
        let mut order = (0..packets.events.len())
            .map(OrderedEvent::Packet)
            .chain((0..reads.events.len()).map(OrderedEvent::Read))
            .collect::<Vec<_>>();
        order.sort_by_key(|event| event.bounds(&packets, &reads).0);
        for pair in order.windows(2) {
            let (_, end) = pair[0].bounds(&packets, &reads);
            let (start, _) = pair[1].bounds(&packets, &reads);
            let same_writer = matches!(
                (pair[0], pair[1]),
                (OrderedEvent::Packet(_), OrderedEvent::Packet(_))
                    | (OrderedEvent::Read(_), OrderedEvent::Read(_))
            );
            if end > start || (!same_writer && end == start) {
                return Err(ProcessReplayError::AmbiguousOrder);
            }
        }
        let mut published = 0;
        for event in &order {
            match *event {
                OrderedEvent::Packet(index) => {
                    published += packets.events[index].sample.accepted_frames
                }
                OrderedEvent::Read(index) => {
                    let read = reads.events[index].sample;
                    if read.before.published_frames() != Some(published)
                        || read.after.published_frames() != Some(published)
                    {
                        return Err(ProcessReplayError::InconsistentPublication);
                    }
                }
            }
        }
        Ok(Self {
            process,
            owner_generation,
            origin: ProcessTraceOrigin::CallerSupplied,
            consumer_rate,
            prepared_demand,
            packets,
            reads,
            order,
        })
    }

    /// Internal origin promotion only after reading immutable retained Session
    /// metadata and its exact bridge histories, never an externally edited DTO.
    #[cfg(any(windows, test))]
    pub(crate) fn with_retained_owner_origin(mut self) -> Self {
        self.origin = ProcessTraceOrigin::RetainedNativeOwner;
        self
    }

    /// Execute queue mechanics while keeping the candidate producer open for
    /// the entire request window. Writer Drop/teardown timing is not replayed;
    /// synthetic PCM never establishes signal correctness or sample age.
    pub fn replay(
        &self,
        process: &ProcessIdentity,
        generation: u64,
        options: ProcessBridgeOptions,
    ) -> Result<ProcessReplayReport, ProcessReplayError> {
        if !self.process.same_process(process) {
            return Err(ProcessReplayError::IdentityMismatch);
        }
        if self.owner_generation != generation {
            return Err(ProcessReplayError::GenerationMismatch);
        }
        if options.max_ingress_elapsed_100ns.is_some() {
            return Err(ProcessReplayError::IngressElapsedUnsupported);
        }
        if options.engine_sample_rate != self.consumer_rate
            || options.max_demand_frames < self.prepared_demand
        {
            return Err(ProcessReplayError::UnsupportedShape);
        }
        let config = options.clock_config()?;
        if let Some(delivery) = options.delivery {
            let mut previous_before = None;
            for event in &self.packets.events {
                let sample = event.sample;
                if sample.offered_frames > u64::from(delivery.max_packet_frames) {
                    return Err(ProcessReplayError::UnsupportedShape);
                }
                if let Some(previous) = previous_before {
                    let gap = sample.qpc_after_100ns.unwrap() - previous;
                    let frames = (u128::from(gap) * u128::from(crate::NATIVE_SAMPLE_RATE))
                        .div_ceil(10_000_000);
                    if frames > u128::from(delivery.max_gap_frames) {
                        return Err(ProcessReplayError::UnsupportedShape);
                    }
                }
                previous_before = sample.qpc_before_100ns;
            }
        }
        let (mut ingress, mut source, observer) =
            capture_bridge(config).map_err(CaptureError::from)?;
        let max_read = self
            .reads
            .events
            .iter()
            .map(|e| e.sample.requested_frames as usize)
            .max()
            .unwrap();
        let mut output = vec![0.0; max_read * 2];
        let mut startup = true;
        let mut startup_unserved_frames = 0;
        let mut unserved_frames = 0;
        let baseline = self.baseline();
        let mut inventory = inventory::Inventory::default();
        let mut read_count = 0;
        for event in &self.order {
            match *event {
                OrderedEvent::Packet(index) => {
                    let packet = self.packets.events[index].sample;
                    // Sample values do not affect queue/servo mechanics. Keep
                    // native discontinuity, but never copy baseline overflow's
                    // derived generation into the candidate or invent age.
                    ingress
                        .push_packet(
                            &[],
                            CapturePacket {
                                frames: packet.offered_frames as usize,
                                flags: packet.flags | SILENT,
                                device_position_frames: 0,
                                qpc_100ns: 0,
                            },
                        )
                        .map_err(CaptureError::from)?;
                    inventory.packet(packet.offered_frames);
                }
                OrderedEvent::Read(index) => {
                    let requested = self.reads.events[index].sample.requested_frames;
                    let report = source
                        .read_interleaved(&mut output[..requested as usize * 2])
                        .map_err(CaptureError::from)?;
                    let missing = requested - report.transferred_frames as u64;
                    unserved_frames += missing;
                    if startup {
                        startup_unserved_frames += missing;
                    }
                    if report.transferred_frames > 0 {
                        startup = false;
                    }
                    let sample = self.reads.events[index].sample;
                    inventory.output(
                        report.transferred_frames as u64,
                        sample.qpc_before_100ns.unwrap(),
                        sample.qpc_after_100ns.unwrap(),
                        source.snapshot().fill_frames as u64,
                    );
                    read_count += 1;
                    if read_count % 256 == 0 {
                        inventory.reads(
                            &observer
                                .queue_history()
                                .ok_or(ProcessReplayError::InvalidHistory)?,
                        )?;
                    }
                }
            }
            inventory.boundary(source.snapshot().fill_frames as u64);
        }
        if read_count % 256 != 0 {
            inventory.reads(
                &observer
                    .queue_history()
                    .ok_or(ProcessReplayError::InvalidHistory)?,
            )?;
        }
        let s = source.snapshot();
        let inventory = inventory.finish(s.written_frames, s.fill_frames as u64)?;
        let candidate = ReplayQueueReport {
            captured_frames: s.captured_frames,
            written_frames: s.written_frames,
            dropped_frames: s.dropped_frames,
            output_frames: s.output_frames,
            priming_frames: s.priming_frames,
            underrun_frames: s.underrun_frames,
            prime_discarded_frames: s.prime_discarded_frames,
            backlog_discarded_frames: s.backlog_discarded_frames,
            backlog_recoveries: s.backlog_recoveries,
            age_discarded_frames: s.age_discarded_frames,
            retired_input_frames: inventory.retired_input_frames,
            consumed_input_frames: inventory.consumed_input_frames,
            reset_discarded_frames: inventory.reset_discarded_frames,
            resets: s.resets,
            min_fill_frames: s.min_fill_frames,
            max_fill_frames: s.max_fill_frames,
            correction_ppm: s.correction_ppm,
        };
        Ok(ProcessReplayReport {
            status: ProcessReplayStatus::Replayed,
            qualified: false,
            trace_origin: self.origin,
            process: self.process.clone(),
            owner_generation: self.owner_generation,
            options,
            window_start_100ns: self.order[0].bounds(&self.packets, &self.reads).0,
            window_end_100ns: self
                .order
                .last()
                .unwrap()
                .bounds(&self.packets, &self.reads)
                .1,
            packet_events: self.packets.total_events,
            read_events: self.reads.total_events,
            offered_supply_assumption: OfferedSupplyAssumption::CompletePacketAtObservedOrder,
            start_inventory_frames: 0,
            terminal_inventory_frames: s.fill_frames as u64,
            requested_frames: baseline.requested_frames,
            startup_unserved_frames,
            unserved_frames,
            baseline,
            candidate,
            inventory,
        })
    }

    fn baseline(&self) -> ReplayBaselineReport {
        let mut baseline = ReplayBaselineReport {
            offered_frames: self.packets.offered_frames,
            published_frames: self.packets.published_frames,
            dropped_frames: self.packets.offered_frames - self.packets.published_frames,
            ..Default::default()
        };
        for event in &self.reads.events {
            let s = event.sample;
            baseline.requested_frames += s.requested_frames;
            baseline.output_frames += s.output_frames;
            baseline.underrun_frames += s.underrun_frames;
            baseline.prime_discarded_frames += s.prime_discarded_frames;
            baseline.backlog_discarded_frames += s.backlog_discarded_frames;
            baseline.age_discarded_frames += s.age_discarded_frames;
            baseline.ingress_elapsed_discarded_frames += s.ingress_elapsed_discarded_frames;
            baseline.reset_discarded_frames += s.reset_discarded_frames;
            baseline.resets += s.resets;
        }
        baseline
    }
}

fn complete_history(
    finished: bool,
    total: u64,
    overwritten: u64,
    count: usize,
    saturated: bool,
    event_limit: usize,
    require_finished: bool,
) -> Result<(), ProcessReplayError> {
    if (require_finished && !finished)
        || saturated
        || total == 0
        || total > event_limit as u64
        || overwritten != 0
        || count as u64 != total
    {
        return Err(ProcessReplayError::IncompleteHistory);
    }
    Ok(())
}

fn validate_clock(before: Option<u64>, after: Option<u64>) -> Result<(), ProcessReplayError> {
    match (before, after) {
        (Some(before), Some(after)) if before > 0 && after >= before => Ok(()),
        _ => Err(ProcessReplayError::ClockFault),
    }
}

fn add_work(work: &mut u64, frames: u64, limit: u64) -> Result<(), ProcessReplayError> {
    *work = work
        .checked_add(frames)
        .ok_or(ProcessReplayError::UnsupportedShape)?;
    if frames == 0 || frames > MAX_EVENT_FRAMES || *work > limit {
        return Err(ProcessReplayError::UnsupportedShape);
    }
    Ok(())
}

#[cfg(test)]
#[path = "replay/tests.rs"]
mod tests;
