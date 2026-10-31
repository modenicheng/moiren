//! Opaque provenance boundary around the public, untrusted archive format.
use super::{NativeObservationReadHandle, NodeId, ProcessDeliverySource};
use crate::latency::{
    InputDeliverySnapshot, events::EventHistorySnapshot, transport::TransportInputPolicy,
};
use crate::process_loopback::{
    LiveProcessWindowReport, MeasuredProcessDelivery, ObservedProcessTrace, ProcessBridgeOptions,
    ProcessIdentity, ProcessReplayError, ProcessTraceArchive, ProcessTraceCollectionRefusal,
    ProcessTraceCollector, ProcessTraceOrigin, ReplayWindowPolicy, ReplayWindowStatus,
};

/// Frozen native evidence for evaluation on a control worker. Only the pinned
/// collector can construct this value; no DTO import can issue native proof.
pub struct PreparedNativeProcessWindow {
    artifact: ProcessTraceArchive,
    measurement: MeasuredProcessDelivery,
    options: ProcessBridgeOptions,
    policy: ReplayWindowPolicy,
}

/// A privately issued finite-window result, never a hardware qualification.
#[derive(Clone, Debug)]
pub struct NativeProcessWindowCandidate {
    report: LiveProcessWindowReport,
    measurement: MeasuredProcessDelivery,
    options: ProcessBridgeOptions,
}

impl NativeProcessWindowCandidate {
    pub fn report(&self) -> &LiveProcessWindowReport {
        &self.report
    }

    pub fn measurement(&self) -> &MeasuredProcessDelivery {
        &self.measurement
    }

    pub fn options(&self) -> ProcessBridgeOptions {
        self.options
    }
}

impl PreparedNativeProcessWindow {
    /// Run the bounded actual bridge replay off the service/RT worker. Native
    /// provenance authenticates the frozen source, never physical performance.
    pub fn evaluate(self) -> Result<NativeProcessWindowCandidate, String> {
        let report = self
            .artifact
            .replay_retained_live_window(
                self.measurement.process(),
                self.measurement.owner_generation(),
                self.options,
                self.policy,
            )
            .map_err(|error| error.to_string())?;
        let accepted = report.status == ReplayWindowStatus::NoViolationInWindow
            && report.trace_origin == ProcessTraceOrigin::RetainedNativeOwner
            && report.collection_refusal.is_none()
            && report.refusal.is_none()
            && report.prefix.is_some()
            && report.replay.as_ref().is_some_and(|replay| {
                replay.assessment.status == ReplayWindowStatus::NoViolationInWindow
                    && replay.report.trace_origin == ProcessTraceOrigin::RetainedNativeOwner
                    && replay.report.options == self.options
            });
        if !accepted {
            return Err(format!(
                "native Process window {:?}: {}",
                report.status,
                report
                    .refusal
                    .as_deref()
                    .unwrap_or("finite replay did not pass")
            ));
        }
        Ok(NativeProcessWindowCandidate {
            report,
            measurement: self.measurement,
            options: self.options,
        })
    }
}

#[derive(PartialEq, Eq)]
struct WindowEvidence {
    delivery: InputDeliverySnapshot,
    producer: EventHistorySnapshot,
    consumer: EventHistorySnapshot,
}

impl WindowEvidence {
    fn snapshot(source: &ProcessDeliverySource) -> Result<Self, String> {
        Ok(Self {
            delivery: source
                .bridge
                .snapshot()
                .native_delivery
                .ok_or("Process delivery observations are missing or contended")?,
            producer: source
                .bridge
                .delivery_events()
                .ok_or("Process complete-drain observations are contended")?,
            consumer: source
                .consumer
                .as_ref()
                .and_then(|observer| observer.snapshot())
                .ok_or("Process whole-wake observations are missing or contended")?,
        })
    }

    fn require_coverage(&self, artifact: &ProcessTraceArchive) -> Result<(), String> {
        // These rings do not retain cumulative frame totals or whole-wake
        // maxima. A suffix cannot prove coverage of the full archived prefix.
        if self.producer.overwritten_events != 0 || self.consumer.overwritten_events != 0 {
            return Err(
                "Process measured histories cannot cover the full archive after overwrite (limit: 256 complete-drain and 256 whole-wake events)"
                    .into(),
            );
        }
        let supply = checked_frames(self.producer.events.iter().map(|event| event.frames))?;
        let demand = checked_frames(self.consumer.events.iter().map(|event| event.frames))?;
        let reads = checked_frames(
            artifact
                .queue_history
                .events
                .iter()
                .map(|event| event.sample.requested_frames),
        )?;
        // Packet publication precedes drain completion, and individual graph
        // reads precede whole-wake completion. Even stable metadata can lag a
        // partially completed callback. Equal cumulative frame/packet totals
        // fence those gaps without inventing a cross-owner atomic snapshot.
        if self.delivery.packets != artifact.packet_history.total_events
            || supply != artifact.packet_history.offered_frames
            || demand != reads
        {
            return Err(
                "Process complete-drain/whole-wake observations do not cover the full archive"
                    .into(),
            );
        }
        Ok(())
    }
}

fn checked_frames(mut frames: impl Iterator<Item = u64>) -> Result<u64, String> {
    frames
        .try_fold(0_u64, |total, frames| total.checked_add(frames))
        .ok_or_else(|| "Process measured archive frame coverage overflowed".into())
}

fn publication_gap_100ns(artifact: &ProcessTraceArchive) -> Result<u64, String> {
    let mut gap = 0;
    for pair in artifact.packet_history.events.windows(2) {
        let outer = pair[1]
            .sample
            .qpc_after_100ns
            .zip(pair[0].sample.qpc_before_100ns)
            .and_then(|(after, before)| after.checked_sub(before))
            .ok_or("Process publication envelope is unavailable or regressed")?;
        gap = gap.max(outer);
    }
    Ok(gap)
}

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
    /// Freeze original-owner observations on control before sending bounded
    /// replay work to another worker. Refuses incomplete, racing or overwritten
    /// measurement histories; never waits, retries or stops the native owner.
    /// Both measured histories must retain sequence one (at most 256 completed
    /// drains and 256 nonempty whole wakes), independently of the archive cap.
    pub fn prepare_live_window(
        &mut self,
        input: TransportInputPolicy,
        policy: ReplayWindowPolicy,
    ) -> Result<PreparedNativeProcessWindow, String> {
        if self
            .source
            .admitted_bridge
            .max_ingress_elapsed_100ns
            .is_some()
        {
            return Err(ProcessReplayError::IngressElapsedUnsupported.to_string());
        }
        policy.validate().map_err(|error| error.to_string())?;
        let before = WindowEvidence::snapshot(&self.source)?;
        self.poll();
        let artifact = self.collector.artifact();
        let after = WindowEvidence::snapshot(&self.source)?;
        // Each history is independently coherent, not a shared transaction.
        // Monotonic counters plus exact coverage below refuse any publication
        // that could otherwise leave the replay with undersized measured bounds.
        if before != after {
            return Err("Process measurement changed while polling the live archive".into());
        }
        after.require_coverage(&artifact)?;
        let measurement = MeasuredProcessDelivery::from_observations(
            self.source.process.clone(),
            self.owner_generation(),
            self.source.input_sample_rate,
            self.source.consumer_sample_rate,
            self.source.prepared_consumer_demand_frames,
            after.delivery,
            after.producer,
            after.consumer,
        )
        .map_err(|error| error.to_string())?
        .with_publication_gap_100ns(publication_gap_100ns(&artifact)?)
        .map_err(|error| error.to_string())?;
        let options = measurement
            .admit_for_consumer(
                self.process(),
                self.owner_generation(),
                input,
                self.source.consumer_sample_rate,
                self.source.prepared_consumer_demand_frames,
            )
            .map_err(|error| error.to_string())?;
        Ok(PreparedNativeProcessWindow {
            artifact,
            measurement,
            options,
            policy,
        })
    }

    /// Assess a sealed sequence-one prefix while its original owner runs.
    /// Does not stop audio or authenticate any exported/edited archive.
    pub fn replay_live_window(
        &mut self,
        options: crate::process_loopback::ProcessBridgeOptions,
        policy: crate::process_loopback::ReplayWindowPolicy,
    ) -> Result<crate::process_loopback::LiveProcessWindowReport, ProcessReplayError> {
        if self
            .source
            .admitted_bridge
            .max_ingress_elapsed_100ns
            .is_some()
        {
            return Err(ProcessReplayError::IngressElapsedUnsupported);
        }
        self.poll();
        self.collector.artifact().replay_retained_live_window(
            self.process(),
            self.owner_generation(),
            options,
            policy,
        )
    }

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
        // Refuse only replacement admission. The pinned raw archive and its
        // closure/provenance remain available independently of this policy.
        if source.admitted_bridge.max_ingress_elapsed_100ns.is_some() {
            return Err(
                crate::process_loopback::MeasuredAdmissionError::IngressElapsedUnsupported
                    .to_string(),
            );
        }
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
        measured
            .with_publication_gap_100ns(publication_gap_100ns(&artifact)?)
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
