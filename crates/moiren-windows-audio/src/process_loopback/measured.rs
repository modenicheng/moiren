//! Control-side Process delivery evidence; arrival timing is not sample age.
//!
//! Admission keeps an all-phase reserve. Completed producer drains and consumer
//! demand requests do not establish the queue occupancy at the overlap boundary,
//! or the intra-drain publication order. Replaying them from an invented initial
//! fill would not prove real queue underflow/capacity feasibility. Phase is a
//! diagnostic only; this module does not claim that replay or hardware timing.
//! The per-wake demand maximum also does not bound wake density. Raw histories
//! retain that evidence, but admission does not qualify sustained queue balance.
use super::{ProcessBridgeOptions, ProcessIdentity};
use crate::latency::{
    InputDeliverySnapshot, LatencyBudgetError,
    events::{EVENT_HISTORY_CAPACITY, EventHistorySnapshot},
    transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy, prepare_transport_input,
    },
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MeasuredAdmissionError {
    #[error("measured Process admission is missing {observation} observations")]
    MissingObservation { observation: &'static str },
    #[error("Process identity requires a nonzero PID and creation time")]
    InvalidIdentity,
    #[error("measured Process admission requires a nonzero owner generation")]
    InvalidGeneration,
    #[error("Process input evidence must use the native 48000 Hz PCM format")]
    InvalidInputSampleRate,
    #[error("consumer rate must be supported and its prepared demand nonzero")]
    InvalidConsumer,
    #[error("{observation} history needs at least two retained events")]
    InsufficientHistory { observation: &'static str },
    #[error("{observation} history has inconsistent counts, sequence or frame shape")]
    InvalidHistory { observation: &'static str },
    #[error("{observation} observations contain unavailable or regressed clocks")]
    ClockFault { observation: &'static str },
    #[error("{observation} observations have exhausted a bounded counter")]
    SaturatedEvidence { observation: &'static str },
    #[error("packet delivery totals and retained callback drains disagree")]
    InconsistentDelivery,
    #[error("producer and consumer histories need two events each in a real overlapping window")]
    InsufficientOverlap,
    #[error("measured Process identity does not match the requested source")]
    IdentityMismatch,
    #[error("measured Process owner generation does not match the requested generation")]
    GenerationMismatch,
    #[error("measured admission requires an explicit Target policy")]
    TargetRequired,
    #[error("measured Process admission cannot preserve or replay ingress elapsed enforcement")]
    IngressElapsedUnsupported,
    #[error("measured Process frame conversion or arithmetic overflowed")]
    ArithmeticOverflow,
    #[error(transparent)]
    Budget(#[from] LatencyBudgetError),
}

/// Constructed from observation snapshots, never deserialized as trusted proof.
/// The native observation owner must supply the source/generation association.
#[derive(Debug, Clone, Serialize)]
pub struct MeasuredProcessDelivery {
    process: ProcessIdentity,
    owner_generation: u64,
    input_sample_rate: u32,
    consumer_sample_rate: u32,
    prepared_consumer_demand_frames: u32,
    observed_consumer_demand_frames: u32,
    max_callback_burst_frames: u32,
    max_inter_drain_gap_100ns: u64,
    max_gap_frames: u32,
    delivery: InputDeliverySnapshot,
    producer: EventHistorySnapshot,
    consumer: EventHistorySnapshot,
    phase: PhaseReport,
}

#[derive(Debug, Clone, Serialize)]
struct PhaseReport {
    overlap_start_100ns: u64,
    overlap_end_100ns: u64,
    producer_events: u64,
    consumer_events: u64,
    phase_samples: u64,
    min_producer_to_consumer_100ns: Option<u64>,
    max_producer_to_consumer_100ns: Option<u64>,
    timestamp_ties: u64,
}

impl MeasuredProcessDelivery {
    #[allow(clippy::too_many_arguments)]
    pub fn from_observations(
        process: ProcessIdentity,
        owner_generation: u64,
        input_sample_rate: u32,
        consumer_sample_rate: u32,
        prepared_consumer_demand_frames: u32,
        delivery: InputDeliverySnapshot,
        producer: EventHistorySnapshot,
        consumer: EventHistorySnapshot,
    ) -> Result<Self, MeasuredAdmissionError> {
        Self::from_optional_observations(
            process,
            owner_generation,
            input_sample_rate,
            consumer_sample_rate,
            prepared_consumer_demand_frames,
            Some(delivery),
            Some(producer),
            Some(consumer),
        )
    }

    /// Check every present component before reporting readiness. Independent
    /// snapshots need not arrive together, but a missing peer cannot hide a
    /// known clock, counter or shape fault. No missing evidence is synthesized.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_optional_observations(
        process: ProcessIdentity,
        owner_generation: u64,
        input_sample_rate: u32,
        consumer_sample_rate: u32,
        prepared_consumer_demand_frames: u32,
        delivery: Option<InputDeliverySnapshot>,
        producer: Option<EventHistorySnapshot>,
        consumer: Option<EventHistorySnapshot>,
    ) -> Result<Self, MeasuredAdmissionError> {
        if process.pid == 0 || process.creation_time_100ns == 0 {
            return Err(MeasuredAdmissionError::InvalidIdentity);
        }
        if owner_generation == 0 {
            return Err(MeasuredAdmissionError::InvalidGeneration);
        }
        if input_sample_rate != crate::NATIVE_SAMPLE_RATE {
            return Err(MeasuredAdmissionError::InvalidInputSampleRate);
        }
        validate_consumer(consumer_sample_rate, prepared_consumer_demand_frames)?;
        let delivery = delivery
            .map(|value| validate_delivery_counters(value).map(|burst| (value, burst)))
            .transpose()?;
        let producer = producer
            .map(|history| validate_history(&history, "producer").map(|summary| (history, summary)))
            .transpose()?;
        let consumer = consumer
            .map(|history| validate_history(&history, "consumer").map(|summary| (history, summary)))
            .transpose()?;
        if let Some((_, supply)) = &producer
            && supply.max_gap_100ns != 0
        {
            frames_for_gap(supply.max_gap_100ns, input_sample_rate)?;
        }
        if let (Some((delivery, burst)), Some((producer, supply))) = (&delivery, &producer) {
            validate_delivery(*delivery, producer, supply, *burst)?;
        }
        let (delivery, max_callback_burst_frames) =
            delivery.ok_or(MeasuredAdmissionError::MissingObservation {
                observation: "packet delivery",
            })?;
        let (producer, supply) = producer.ok_or(MeasuredAdmissionError::MissingObservation {
            observation: "producer",
        })?;
        let (consumer, demand) = consumer.ok_or(MeasuredAdmissionError::MissingObservation {
            observation: "consumer",
        })?;
        for (history, observation) in [(&producer, "producer"), (&consumer, "consumer")] {
            if history.events.len() < 2 {
                return Err(MeasuredAdmissionError::InsufficientHistory { observation });
            }
        }
        let max_gap_frames = frames_for_gap(
            delivery.max_gap_100ns.max(supply.max_gap_100ns),
            input_sample_rate,
        )?;
        let phase = phase_report(&producer, &consumer, &supply, &demand)?;
        Ok(Self {
            process,
            owner_generation,
            input_sample_rate,
            consumer_sample_rate,
            prepared_consumer_demand_frames,
            observed_consumer_demand_frames: demand.max_frames,
            max_callback_burst_frames,
            max_inter_drain_gap_100ns: supply.max_gap_100ns,
            max_gap_frames,
            delivery,
            producer,
            consumer,
            phase,
        })
    }

    pub fn admit_for_consumer(
        &self,
        expected_process: &ProcessIdentity,
        expected_generation: u64,
        input: TransportInputPolicy,
        engine_sample_rate: u32,
        max_demand_frames: u32,
    ) -> Result<ProcessBridgeOptions, MeasuredAdmissionError> {
        if !self.process.same_process(expected_process) {
            return Err(MeasuredAdmissionError::IdentityMismatch);
        }
        if self.owner_generation != expected_generation {
            return Err(MeasuredAdmissionError::GenerationMismatch);
        }
        if !matches!(input, TransportInputPolicy::Target { .. }) {
            return Err(MeasuredAdmissionError::TargetRequired);
        }
        validate_consumer(engine_sample_rate, max_demand_frames)?;
        // Preserve both observed demand and the old owner's full prepared
        // bound when moving to another engine rate. Neither graph segmentation
        // nor a smaller new caller estimate can erase measured demand.
        let historical_demand = self
            .prepared_consumer_demand_frames
            .max(self.observed_consumer_demand_frames);
        let rebound_demand = u64::from(historical_demand)
            .checked_mul(u64::from(engine_sample_rate))
            .ok_or(MeasuredAdmissionError::ArithmeticOverflow)?
            .div_ceil(u64::from(self.consumer_sample_rate));
        let rebound_demand = u32::try_from(rebound_demand)
            .map_err(|_| MeasuredAdmissionError::ArithmeticOverflow)?;
        let max_demand_frames = max_demand_frames.max(rebound_demand);
        let delivery = InputDeliveryTiming {
            sample_rate: self.input_sample_rate,
            // The existing budget's publication headroom must fit one complete
            // drain, even when its individual packets were much smaller.
            max_packet_frames: self.max_callback_burst_frames,
            max_gap_frames: self.max_gap_frames,
            source: DeliveryTimingSource::Empirical,
        };
        prepare_transport_input(
            input,
            Some(delivery),
            engine_sample_rate,
            max_demand_frames,
            2,
        )?;
        Ok(ProcessBridgeOptions {
            input,
            delivery: Some(delivery),
            engine_sample_rate,
            max_demand_frames,
            age_limit_100ns: None,
            max_ingress_elapsed_100ns: None,
        })
    }

    pub fn process(&self) -> &ProcessIdentity {
        &self.process
    }

    /// Closed retained traces also bound publication with two clock reads.
    /// Widen the measured reserve to their outer envelope, never replace the
    /// independently measured whole-drain or whole-wake maxima with segments.
    #[cfg(any(windows, test))]
    pub(crate) fn with_publication_gap_100ns(
        mut self,
        gap: u64,
    ) -> Result<Self, MeasuredAdmissionError> {
        self.max_gap_frames = self
            .max_gap_frames
            .max(frames_for_gap(gap, self.input_sample_rate)?);
        Ok(self)
    }

    pub fn owner_generation(&self) -> u64 {
        self.owner_generation
    }

    /// Time since a strictly earlier completed producer drain, in the retained
    /// overlap only. Same-stamp supply/demand has unknown order and is excluded.
    pub fn phase_range_100ns(&self) -> Option<(u64, u64)> {
        self.phase
            .min_producer_to_consumer_100ns
            .zip(self.phase.max_producer_to_consumer_100ns)
    }

    pub fn timestamp_ties(&self) -> u64 {
        self.phase.timestamp_ties
    }
}

fn validate_consumer(rate: u32, demand: u32) -> Result<(), MeasuredAdmissionError> {
    if demand == 0 || !moiren_engine::audio_config::SAMPLE_RATES.contains(&rate) {
        return Err(MeasuredAdmissionError::InvalidConsumer);
    }
    Ok(())
}

struct HistorySummary {
    first_100ns: u64,
    last_100ns: u64,
    max_frames: u32,
    total_frames: u64,
    max_gap_100ns: u64,
}

fn validate_history(
    history: &EventHistorySnapshot,
    observation: &'static str,
) -> Result<HistorySummary, MeasuredAdmissionError> {
    if history.saturated || history.total_events == u64::MAX {
        return Err(MeasuredAdmissionError::SaturatedEvidence { observation });
    }
    if history.unavailable_clocks != 0 || history.regressed_clocks != 0 {
        return Err(MeasuredAdmissionError::ClockFault { observation });
    }
    // A real fixed ring only omits a prefix after all retained slots fill.
    // Otherwise an internally consistent shortened suffix could erase a
    // larger consumer burst and manufacture a smaller measured reserve.
    let expected_count = history.total_events.min(EVENT_HISTORY_CAPACITY as u64);
    if history.events.len() as u64 != expected_count
        || history.overwritten_events != history.total_events - expected_count
        || history
            .overwritten_events
            .checked_add(history.events.len() as u64)
            != Some(history.total_events)
    {
        return Err(MeasuredAdmissionError::InvalidHistory { observation });
    }
    let mut summary = HistorySummary {
        first_100ns: 0,
        last_100ns: 0,
        max_frames: 0,
        total_frames: 0,
        max_gap_100ns: 0,
    };
    for (index, event) in history.events.iter().enumerate() {
        // Counts were checked above, so every expected sequence fits u64.
        if event.sequence != history.overwritten_events + index as u64 + 1 || event.frames == 0 {
            return Err(MeasuredAdmissionError::InvalidHistory { observation });
        }
        let frames =
            u32::try_from(event.frames).map_err(|_| MeasuredAdmissionError::ArithmeticOverflow)?;
        let stamp = event
            .qpc_100ns
            .filter(|&stamp| stamp != 0)
            .ok_or(MeasuredAdmissionError::ClockFault { observation })?;
        if index == 0 {
            summary.first_100ns = stamp;
        } else {
            let gap = stamp
                .checked_sub(summary.last_100ns)
                .ok_or(MeasuredAdmissionError::ClockFault { observation })?;
            summary.max_gap_100ns = summary.max_gap_100ns.max(gap);
        }
        summary.last_100ns = stamp;
        summary.max_frames = summary.max_frames.max(frames);
        summary.total_frames = summary
            .total_frames
            .checked_add(event.frames)
            .ok_or(MeasuredAdmissionError::ArithmeticOverflow)?;
    }
    Ok(summary)
}

fn validate_delivery_counters(
    delivery: InputDeliverySnapshot,
) -> Result<u32, MeasuredAdmissionError> {
    if delivery.unavailable_clocks != 0 || delivery.regressed_clocks != 0 {
        return Err(MeasuredAdmissionError::ClockFault {
            observation: "delivery",
        });
    }
    if delivery.packets == u64::MAX {
        return Err(MeasuredAdmissionError::SaturatedEvidence {
            observation: "delivery",
        });
    }
    let burst = u32::try_from(delivery.max_callback_burst_frames)
        .map_err(|_| MeasuredAdmissionError::ArithmeticOverflow)?;
    if delivery.gap_samples != delivery.packets.saturating_sub(1)
        || (delivery.packets == 0
            && (delivery.max_packet_frames != 0 || burst != 0 || delivery.max_gap_100ns != 0))
        || (delivery.packets != 0 && delivery.max_packet_frames == 0)
        || delivery.max_packet_frames > burst
        || delivery.packet_bound_violations > delivery.packets
        || delivery.gap_bound_violations > delivery.gap_samples
    {
        return Err(MeasuredAdmissionError::InconsistentDelivery);
    }
    if delivery.max_gap_100ns != 0 {
        frames_for_gap(delivery.max_gap_100ns, crate::NATIVE_SAMPLE_RATE)?;
    }
    Ok(burst)
}

fn validate_delivery(
    delivery: InputDeliverySnapshot,
    producer: &EventHistorySnapshot,
    supply: &HistorySummary,
    burst: u32,
) -> Result<(), MeasuredAdmissionError> {
    if delivery.packets < producer.total_events
        || burst < supply.max_frames
        || (producer.overwritten_events == 0 && burst != supply.max_frames)
    {
        return Err(MeasuredAdmissionError::InconsistentDelivery);
    }
    if delivery.packets == 0 {
        return Ok(());
    }
    // Every retained drain must be explainable as complete native packets.
    // Prefix overwrites are permitted, but each omitted drain still needs at
    // least one packet in the cumulative delivery totals.
    let packet_frames = u64::from(delivery.max_packet_frames);
    let mut minimum_packets = producer.overwritten_events;
    for event in &producer.events {
        minimum_packets = minimum_packets
            .checked_add(event.frames.div_ceil(packet_frames))
            .ok_or(MeasuredAdmissionError::ArithmeticOverflow)?;
    }
    if burst > supply.max_frames {
        // A larger cumulative maximum must have occurred in the overwritten
        // prefix. One omitted callback then needs its full packet supply,
        // replacing the one-packet lower bound already counted above.
        minimum_packets = minimum_packets
            .checked_add(u64::from(burst).div_ceil(packet_frames) - 1)
            .ok_or(MeasuredAdmissionError::ArithmeticOverflow)?;
    }
    if minimum_packets > delivery.packets {
        return Err(MeasuredAdmissionError::InconsistentDelivery);
    }
    // An omitted drain cannot exceed the cumulative burst maximum. There must
    // be enough frames for every packet and one packet attaining its maximum.
    let maximum_frames = u128::from(supply.total_frames)
        + u128::from(producer.overwritten_events) * u128::from(burst);
    let minimum_frames = u128::from(delivery.packets - 1) + u128::from(packet_frames);
    if maximum_frames < minimum_frames {
        return Err(MeasuredAdmissionError::InconsistentDelivery);
    }
    Ok(())
}

fn frames_for_gap(gap_100ns: u64, rate: u32) -> Result<u32, MeasuredAdmissionError> {
    let frames = (u128::from(gap_100ns) * u128::from(rate)).div_ceil(10_000_000);
    let frames = u32::try_from(frames).map_err(|_| MeasuredAdmissionError::ArithmeticOverflow)?;
    if frames == 0 {
        return Err(MeasuredAdmissionError::InsufficientOverlap);
    }
    Ok(frames)
}

fn phase_report(
    producer: &EventHistorySnapshot,
    consumer: &EventHistorySnapshot,
    supply: &HistorySummary,
    demand: &HistorySummary,
) -> Result<PhaseReport, MeasuredAdmissionError> {
    let start = supply.first_100ns.max(demand.first_100ns);
    let end = supply.last_100ns.min(demand.last_100ns);
    let in_overlap = |stamp: Option<u64>| stamp.is_some_and(|stamp| stamp >= start && stamp <= end);
    let producer_events = producer
        .events
        .iter()
        .filter(|event| in_overlap(event.qpc_100ns))
        .count() as u64;
    let consumer_events = consumer
        .events
        .iter()
        .filter(|event| in_overlap(event.qpc_100ns))
        .count() as u64;
    if start >= end || producer_events < 2 || consumer_events < 2 {
        return Err(MeasuredAdmissionError::InsufficientOverlap);
    }
    let mut phase = PhaseReport {
        overlap_start_100ns: start,
        overlap_end_100ns: end,
        producer_events,
        consumer_events,
        phase_samples: 0,
        min_producer_to_consumer_100ns: None,
        max_producer_to_consumer_100ns: None,
        timestamp_ties: 0,
    };
    for event in &consumer.events {
        if !in_overlap(event.qpc_100ns) {
            continue;
        }
        // Histories have already been checked for nonzero monotonic clocks.
        // A retained producer just before the overlap is a valid phase anchor.
        let index = producer
            .events
            .partition_point(|producer| producer.qpc_100ns <= event.qpc_100ns);
        let previous = &producer.events[index - 1];
        if previous.qpc_100ns == event.qpc_100ns {
            // Consumer-first is the conservative interpretation for supply
            // availability; never report a zero-wait phase from an unknown tie.
            phase.timestamp_ties += 1;
            continue;
        }
        let gap = event.qpc_100ns.unwrap() - previous.qpc_100ns.unwrap();
        phase.phase_samples += 1;
        phase.min_producer_to_consumer_100ns = Some(
            phase
                .min_producer_to_consumer_100ns
                .map_or(gap, |minimum| minimum.min(gap)),
        );
        phase.max_producer_to_consumer_100ns = Some(
            phase
                .max_producer_to_consumer_100ns
                .map_or(gap, |maximum| maximum.max(gap)),
        );
    }
    Ok(phase)
}

#[cfg(test)]
#[path = "measured/tests.rs"]
mod tests;
