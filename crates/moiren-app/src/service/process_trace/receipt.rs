//! Opt-in joined-cleanup diagnostics, never native provenance or admission.
use super::ProcessTraceSessionEvidence;
use crate::service::SessionCleanupObservation;
use moiren_windows_audio::process_loopback::{
    MeasuredProcessDelivery, ProcessIdentity, ProcessTraceArchive,
};
use serde::Serialize;
use std::{fmt, io};

const TARGET: &str = "moiren::audio::process_trace";
const MARKER: &str = "process_trace_cleanup_receipt=";
const MAX_BYTES: usize = 128 * 1024;

#[derive(Debug, PartialEq, Eq)]
enum ReceiptError {
    Oversize,
    Serialization,
}

pub(crate) fn emit(cleanup: &SessionCleanupObservation) {
    if !log::log_enabled!(target: TARGET, log::Level::Debug) {
        return;
    }
    match encode(cleanup) {
        Ok(Some(line)) => log::debug!(target: TARGET, "{MARKER}{line}"),
        Ok(None) => {}
        Err(error) => log::debug!(target: TARGET, "{MARKER}{}", error_payload(cleanup, error)),
    }
}

fn encode(cleanup: &SessionCleanupObservation) -> Result<Option<String>, ReceiptError> {
    let Some(evidence) = cleanup.process_trace.as_deref() else {
        return Ok(None);
    };
    let origin = &evidence.origin;
    let identity_match = cleanup.generation == origin.generation
        && cleanup.state.owner_generation == origin.native_owner_generation
        && cleanup.state.desired_revision == origin.host_revision
        && evidence.artifact.owner_generation == origin.native_owner_generation
        && evidence.artifact.process.same_process(&origin.process)
        && evidence.measurement.as_ref().map_or(true, |measurement| {
            measurement.owner_generation() == origin.native_owner_generation
                && measurement.process().same_process(&origin.process)
        });
    bounded_json(&Receipt {
        schema_version: 1,
        qualified: false,
        scope: "joined_process_trace_cleanup",
        status: "recorded",
        identity_match,
        origin: Origin::new(evidence),
        cleanup: Cleanup {
            host_state: DebugRef(&cleanup.state.host),
            fault: cleanup.state.fault.as_ref().map(DebugRef),
            cleanup_error: cleanup.state.cleanup_error.as_ref().map(DebugRef),
        },
        archive: Archive::new(evidence),
        measurement: evidence.measurement.as_ref().ok(),
        measurement_error: evidence.measurement.as_ref().err().map(String::as_str),
        joined_observation: joined_observation(cleanup, evidence, identity_match),
    })
    .map(Some)
}

// Borrow histories and strings; never clone the archive or construct a JSON tree.
#[derive(Serialize)]
struct Receipt<'a> {
    schema_version: u32,
    qualified: bool,
    scope: &'static str,
    status: &'static str,
    identity_match: bool,
    origin: Origin<'a>,
    cleanup: Cleanup<'a>,
    archive: Archive<'a>,
    measurement: Option<&'a MeasuredProcessDelivery>,
    measurement_error: Option<&'a str>,
    joined_observation: JoinedObservation<'a>,
}

#[derive(Serialize)]
struct Origin<'a> {
    app_generation: u64,
    document: u64,
    node: DebugRef<'a, moiren_core::graph::NodeId>,
    process: &'a ProcessIdentity,
    native_owner_generation: u64,
    host_revision: u64,
    graph_revision: u64,
}

impl<'a> Origin<'a> {
    fn new(evidence: &'a ProcessTraceSessionEvidence) -> Self {
        let origin = &evidence.origin;
        Self {
            app_generation: origin.generation.0,
            document: origin.document,
            node: DebugRef(&origin.node),
            process: &origin.process,
            native_owner_generation: origin.native_owner_generation,
            host_revision: origin.host_revision,
            graph_revision: origin.graph_revision,
        }
    }
}

#[derive(Serialize)]
struct Cleanup<'a> {
    host_state: DebugRef<'a, moiren_engine::host::HostState>,
    fault: Option<DebugRef<'a, moiren_engine::host::Diagnostic>>,
    cleanup_error: Option<DebugRef<'a, moiren_engine::host::BackendError>>,
}

struct DebugRef<'a, T>(&'a T);

impl<T: fmt::Debug> fmt::Display for DebugRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

impl<T: fmt::Debug> Serialize for DebugRef<'_, T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Serialize)]
struct Archive<'a> {
    input_sample_rate: u32,
    consumer_sample_rate: u32,
    prepared_consumer_demand_frames: u32,
    packet_total_events: u64,
    packet_retained_events: usize,
    offered_frames: u64,
    published_frames: u64,
    packet_overwritten_events: u64,
    packet_unavailable_clocks: u64,
    packet_regressed_clocks: u64,
    packet_saturated: bool,
    packet_writer_finished: bool,
    read_total_events: u64,
    read_retained_events: usize,
    read_overwritten_events: u64,
    read_unavailable_clocks: u64,
    read_regressed_clocks: u64,
    read_saturated: bool,
    read_writer_finished: bool,
    total_read_requested_frames: Option<u64>,
    total_read_requested_frames_error: Option<&'static str>,
    max_publication_outer_gap_100ns: Option<u64>,
    max_publication_outer_gap_error: Option<&'static str>,
    collection_refusal:
        Option<moiren_windows_audio::process_loopback::ProcessTraceCollectionRefusal>,
    refusal: Option<&'a str>,
}

impl<'a> Archive<'a> {
    fn new(evidence: &'a ProcessTraceSessionEvidence) -> Self {
        let artifact = &evidence.artifact;
        let packets = &artifact.packet_history;
        let reads = &artifact.queue_history;
        let requested =
            if reads.total_events != reads.events.len() as u64 || reads.overwritten_events != 0 {
                Err("incomplete_history")
            } else {
                reads.events.iter().try_fold(0u64, |total, event| {
                    total
                        .checked_add(event.sample.requested_frames)
                        .ok_or("overflow")
                })
            };
        let gap = publication_gap(artifact);
        Self {
            input_sample_rate: artifact.input_sample_rate,
            consumer_sample_rate: artifact.consumer_sample_rate,
            prepared_consumer_demand_frames: artifact.prepared_consumer_demand_frames,
            packet_total_events: packets.total_events,
            packet_retained_events: packets.events.len(),
            offered_frames: packets.offered_frames,
            published_frames: packets.published_frames,
            packet_overwritten_events: packets.overwritten_events,
            packet_unavailable_clocks: packets.unavailable_clocks,
            packet_regressed_clocks: packets.regressed_clocks,
            packet_saturated: packets.saturated,
            packet_writer_finished: packets.writer_finished,
            read_total_events: reads.total_events,
            read_retained_events: reads.events.len(),
            read_overwritten_events: reads.overwritten_events,
            read_unavailable_clocks: reads.unavailable_clocks,
            read_regressed_clocks: reads.regressed_clocks,
            read_saturated: reads.saturated,
            read_writer_finished: reads.writer_finished,
            total_read_requested_frames: requested.ok(),
            total_read_requested_frames_error: requested.err(),
            max_publication_outer_gap_100ns: gap.ok(),
            max_publication_outer_gap_error: gap.err(),
            collection_refusal: artifact.collection_refusal,
            refusal: evidence.refusal.as_deref(),
        }
    }
}

fn publication_gap(artifact: &ProcessTraceArchive) -> Result<u64, &'static str> {
    let history = &artifact.packet_history;
    if history.events.len() < 2 {
        return Err("insufficient_history");
    }
    if history.total_events != history.events.len() as u64 || history.overwritten_events != 0 {
        return Err("incomplete_history");
    }
    let mut gap = 0;
    for event in &history.events {
        let before = event
            .sample
            .qpc_before_100ns
            .filter(|&value| value != 0)
            .ok_or("unavailable_clock")?;
        let after = event
            .sample
            .qpc_after_100ns
            .filter(|&value| value != 0)
            .ok_or("unavailable_clock")?;
        after.checked_sub(before).ok_or("regressed_clock")?;
    }
    for pair in history.events.windows(2) {
        let before = pair[0].sample.qpc_before_100ns.unwrap();
        let next = pair[1].sample;
        next.qpc_before_100ns
            .unwrap()
            .checked_sub(before)
            .ok_or("regressed_clock")?;
        gap = gap.max(
            next.qpc_after_100ns
                .unwrap()
                .checked_sub(before)
                .ok_or("regressed_clock")?,
        );
    }
    Ok(gap)
}

#[derive(Serialize)]
struct JoinedObservation<'a> {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: Option<&'a Option<moiren_windows_audio::latency::InputDeliverySnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    producer: Option<&'a Option<moiren_windows_audio::latency::events::EventHistorySnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    consumer: Option<&'a Option<moiren_windows_audio::latency::events::EventHistorySnapshot>>,
}

fn joined_observation<'a>(
    cleanup: &'a SessionCleanupObservation,
    evidence: &ProcessTraceSessionEvidence,
    identity_match: bool,
) -> JoinedObservation<'a> {
    let mut result = JoinedObservation {
        status: "not_available",
        delivery: None,
        producer: None,
        consumer: None,
    };
    if evidence.measurement.is_ok() {
        result.status = "not_needed";
        return result;
    }
    let Some(joined) = cleanup.final_observations.as_deref() else {
        return result;
    };
    let Some(native) = joined.native.as_ref() else {
        return result;
    };
    let origin = &evidence.origin;
    if !identity_match
        || joined.host_revision != origin.host_revision
        || joined.graph_revision != origin.graph_revision
        || native.owner_generation != origin.native_owner_generation
    {
        result.status = "mismatch";
        return result;
    }
    let mut matches = native.process_delivery.iter().filter(|observation| {
        observation.node == origin.node
            && observation.process.same_process(&origin.process)
            && observation.owner_generation == origin.native_owner_generation
    });
    let Some(observation) = matches.next() else {
        if !native.process_delivery.is_empty() {
            result.status = "mismatch";
        }
        return result;
    };
    if matches.next().is_some() {
        result.status = "multiple";
        return result;
    }
    // Matching ownership does not assert clock quality, history completeness or admission.
    result.status = "matched";
    result.delivery = Some(&observation.delivery);
    result.producer = Some(&observation.producer);
    result.consumer = Some(&observation.consumer);
    result
}

fn error_payload(cleanup: &SessionCleanupObservation, error: ReceiptError) -> String {
    #[derive(Serialize)]
    struct ErrorIdentity<'a> {
        app_generation: u64,
        document: u64,
        node: DebugRef<'a, moiren_core::graph::NodeId>,
        pid: u32,
        creation_time_100ns: u64,
        native_owner_generation: u64,
        host_revision: u64,
        graph_revision: u64,
    }
    #[derive(Serialize)]
    struct ErrorReceipt<'a> {
        schema_version: u32,
        qualified: bool,
        scope: &'static str,
        status: &'static str,
        max_bytes: usize,
        identity: Option<ErrorIdentity<'a>>,
    }
    let identity = cleanup.process_trace.as_ref().map(|evidence| {
        let origin = &evidence.origin;
        ErrorIdentity {
            app_generation: origin.generation.0,
            document: origin.document,
            node: DebugRef(&origin.node),
            pid: origin.process.pid,
            creation_time_100ns: origin.process.creation_time_100ns,
            native_owner_generation: origin.native_owner_generation,
            host_revision: origin.host_revision,
            graph_revision: origin.graph_revision,
        }
    });
    bounded_json(&ErrorReceipt {
        schema_version: 1, qualified: false, scope: "joined_process_trace_cleanup",
        status: match error { ReceiptError::Oversize => "oversize", ReceiptError::Serialization => "serialization_error" },
        max_bytes: MAX_BYTES, identity,
    }).unwrap_or_else(|_| "{\"schema_version\":1,\"qualified\":false,\"scope\":\"joined_process_trace_cleanup\",\"status\":\"serialization_error\",\"identity\":null}".into())
}

fn bounded_json(value: &impl Serialize) -> Result<String, ReceiptError> {
    struct Sink {
        bytes: Vec<u8>,
        oversize: bool,
    }
    impl io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_BYTES - self.bytes.len() {
                self.oversize = true;
                return Err(io::Error::other("receipt exceeds 128 KiB"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        bytes: Vec::with_capacity(MAX_BYTES),
        oversize: false,
    };
    if serde_json::to_writer(&mut sink, value).is_err() {
        return Err(if sink.oversize {
            ReceiptError::Oversize
        } else {
            ReceiptError::Serialization
        });
    }
    String::from_utf8(sink.bytes).map_err(|_| ReceiptError::Serialization)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{JoinedSessionObservations, ProcessTraceSessionEvidence};
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::host::{
        BackendError, Diagnostic, ExecutionState, HostSnapshot, HostState, UpdateState,
    };
    use moiren_windows_audio::{
        engine_backend::{NativeObservationSnapshot, NativeProcessDeliveryObservation},
        latency::{InputDeliveryRecorder, events::event_history},
        process_loopback::{ProcessBridgeOptions, ProcessTraceOrigin},
    };
    use serde_json::Value;
    use std::sync::Arc;

    fn cleanup(measured: bool) -> SessionCleanupObservation {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Source, 2).unwrap();
        let mut evidence = super::super::tests::evidence(node);
        let mut observation = crate::service::measured_process::tests::observation(node);
        // Record real complete-drain/whole-wake histories covering all eight
        // archive packets, not the four-event fixture used by other tests.
        let (mut producer, producer_observer) = event_history();
        let (mut consumer, consumer_observer) = event_history();
        let mut delivery = InputDeliveryRecorder::new(None).unwrap();
        for n in 0..8 {
            let stamp = 1_000_000 + n * 20_000;
            producer.record(96, Some(stamp));
            consumer.record(96, Some(stamp + 1000));
            delivery.packet(96, Some(stamp));
            delivery.callback_burst(96);
        }
        observation.process = evidence.origin.process.clone();
        observation.delivery = Some(delivery.snapshot());
        observation.producer = producer_observer.snapshot();
        observation.consumer = consumer_observer.snapshot();
        observation.counters.native_delivery = observation.delivery;
        if measured {
            evidence.measurement = observation
                .measured(&evidence.origin.process, 7)
                .map_err(|e| e.to_string());
        }
        SessionCleanupObservation {
            generation: evidence.origin.generation,
            state: HostSnapshot {
                host: HostState::Closed,
                execution: ExecutionState::Dormant,
                desired_revision: 5,
                active_revision: None,
                plan_revision: None,
                owner_generation: 7,
                resource_generation: 0,
                timeline: None,
                update: UpdateState::Idle,
                fault: None,
                cleanup_error: None,
            },
            process_trace: Some(Arc::new(evidence)),
            final_observations: Some(Arc::new(JoinedSessionObservations {
                graph_revision: 11,
                host_revision: 5,
                native: Some(NativeObservationSnapshot {
                    owner_generation: 7,
                    running: false,
                    streams: vec![],
                    bridges: vec![],
                    boundary_probes: vec![],
                    render_padding: vec![],
                    process_delivery: vec![observation],
                    virtual_clock: None,
                }),
                backend: None,
            })),
        }
    }

    fn payload(cleanup: &SessionCleanupObservation) -> Value {
        serde_json::from_str(&encode(cleanup).unwrap().unwrap()).unwrap()
    }

    fn evidence(cleanup: &mut SessionCleanupObservation) -> &mut ProcessTraceSessionEvidence {
        Arc::get_mut(cleanup.process_trace.as_mut().unwrap()).unwrap()
    }

    fn observation(
        cleanup: &mut SessionCleanupObservation,
    ) -> &mut NativeProcessDeliveryObservation {
        &mut Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
            .unwrap()
            .native
            .as_mut()
            .unwrap()
            .process_delivery[0]
    }

    #[test]
    fn joined_receipt_success_keeps_exact_measurement_without_duplicate_histories_or_origin_promotion()
     {
        let cleanup = cleanup(true);
        let retained = cleanup.process_trace.as_ref().unwrap();
        let before = Arc::as_ptr(retained);
        let line = encode(&cleanup).unwrap().unwrap();
        assert!(line.len() <= MAX_BYTES && !line.contains('\n'));
        let actual: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(actual["schema_version"], 1);
        assert_eq!(actual["qualified"], false);
        assert_eq!(actual["scope"], "joined_process_trace_cleanup");
        assert_eq!(actual["status"], "recorded");
        assert_eq!(actual["identity_match"], true);
        assert_eq!(actual["origin"]["app_generation"], 3);
        assert_eq!(actual["origin"]["document"], 9);
        assert_eq!(
            actual["origin"]["node"],
            format!("{:?}", retained.origin.node)
        );
        assert_eq!(actual["origin"]["native_owner_generation"], 7);
        assert_eq!(actual["origin"]["host_revision"], 5);
        assert_eq!(actual["origin"]["graph_revision"], 11);
        assert_eq!(
            actual["measurement"],
            serde_json::to_value(retained.measurement.as_ref().unwrap()).unwrap()
        );
        assert!(actual["measurement_error"].is_null());
        assert_eq!(actual["joined_observation"]["status"], "not_needed");
        assert!(actual["joined_observation"].get("producer").is_none());
        assert_eq!(actual["archive"]["packet_total_events"], 8);
        assert_eq!(actual["archive"]["offered_frames"], 768);
        assert_eq!(actual["archive"]["total_read_requested_frames"], 768);
        assert_eq!(actual["archive"]["max_publication_outer_gap_100ns"], 20_000);
        assert_eq!(actual["archive"]["packet_writer_finished"], true);
        assert_eq!(actual["archive"]["read_writer_finished"], true);
        assert!(actual.get("trace_origin").is_none());
        assert!(!line.contains("retained_native_owner"));
        let replay = retained
            .trace
            .as_ref()
            .unwrap()
            .replay(&retained.origin.process, 7, ProcessBridgeOptions::default())
            .unwrap();
        assert_eq!(replay.trace_origin, ProcessTraceOrigin::CallerSupplied);
        assert_eq!(Arc::as_ptr(cleanup.process_trace.as_ref().unwrap()), before);
    }

    #[test]
    fn joined_receipt_refusal_preserves_exact_matching_post_join_histories_and_cleanup_faults() {
        let mut cleanup = cleanup(false);
        cleanup.state.fault = Some(Diagnostic::Fault("retained\nfault".into()));
        cleanup.state.cleanup_error = Some(BackendError::Failed("cleanup error".into()));
        evidence(&mut cleanup).refusal = Some("cleanup refused".into());
        let before = cleanup.clone();
        let actual = payload(&cleanup);
        assert_eq!(
            actual["measurement_error"],
            "software fixture has no native final measurement"
        );
        assert!(actual["measurement"].is_null());
        assert_eq!(actual["archive"]["refusal"], "cleanup refused");
        assert_eq!(actual["joined_observation"]["status"], "matched");
        let raw = &cleanup
            .final_observations
            .as_ref()
            .unwrap()
            .native
            .as_ref()
            .unwrap()
            .process_delivery[0];
        assert_eq!(
            actual["joined_observation"]["delivery"],
            serde_json::to_value(raw.delivery).unwrap()
        );
        assert_eq!(
            actual["joined_observation"]["producer"],
            serde_json::to_value(&raw.producer).unwrap()
        );
        assert_eq!(
            actual["joined_observation"]["consumer"],
            serde_json::to_value(&raw.consumer).unwrap()
        );
        assert_eq!(actual["cleanup"]["host_state"], "Closed");
        assert!(
            actual["cleanup"]["fault"]
                .as_str()
                .unwrap()
                .contains("retained")
        );
        assert!(
            actual["cleanup"]["cleanup_error"]
                .as_str()
                .unwrap()
                .contains("cleanup error")
        );
        assert_eq!(cleanup, before);
    }

    #[test]
    fn joined_receipt_missing_stale_ambiguous_or_mismatched_histories_are_not_substituted() {
        for case in 0..9 {
            let mut cleanup = cleanup(false);
            let expected = match case {
                0 => {
                    cleanup.final_observations = None;
                    "not_available"
                }
                1 => {
                    Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
                        .unwrap()
                        .native = None;
                    "not_available"
                }
                2 => {
                    Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
                        .unwrap()
                        .host_revision += 1;
                    "mismatch"
                }
                3 => {
                    Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
                        .unwrap()
                        .graph_revision += 1;
                    "mismatch"
                }
                4 => {
                    observation(&mut cleanup).process.creation_time_100ns += 1;
                    "mismatch"
                }
                5 => {
                    observation(&mut cleanup).owner_generation += 1;
                    "mismatch"
                }
                6 => {
                    let native = Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
                        .unwrap()
                        .native
                        .as_mut()
                        .unwrap();
                    native
                        .process_delivery
                        .push(native.process_delivery[0].clone());
                    "multiple"
                }
                7 => {
                    Arc::get_mut(cleanup.final_observations.as_mut().unwrap())
                        .unwrap()
                        .native
                        .as_mut()
                        .unwrap()
                        .owner_generation += 1;
                    "mismatch"
                }
                _ => {
                    cleanup.generation.0 += 1;
                    "mismatch"
                }
            };
            let actual = payload(&cleanup);
            assert_eq!(
                actual["joined_observation"]["status"], expected,
                "case {case}"
            );
            assert!(
                actual["joined_observation"].get("producer").is_none(),
                "case {case}"
            );
        }
    }

    #[test]
    fn joined_receipt_archive_checked_summaries_refuse_unknown_regression_and_overflow() {
        let mut cleanup = cleanup(false);
        evidence(&mut cleanup).artifact.queue_history.events[0]
            .sample
            .requested_frames = u64::MAX;
        evidence(&mut cleanup).artifact.packet_history.events[1]
            .sample
            .qpc_after_100ns = None;
        let actual = payload(&cleanup);
        assert!(actual["archive"]["total_read_requested_frames"].is_null());
        assert_eq!(
            actual["archive"]["total_read_requested_frames_error"],
            "overflow"
        );
        assert!(actual["archive"]["max_publication_outer_gap_100ns"].is_null());
        assert!(actual["archive"]["max_publication_outer_gap_error"].is_string());
        let mut regressed = self::cleanup(false);
        evidence(&mut regressed).artifact.packet_history.events[1]
            .sample
            .qpc_after_100ns = Some(1);
        let actual = payload(&regressed);
        assert!(actual["archive"]["max_publication_outer_gap_100ns"].is_null());
        assert_eq!(
            actual["archive"]["max_publication_outer_gap_error"],
            "regressed_clock"
        );
    }

    #[test]
    fn joined_receipt_oversize_and_serialization_failure_are_explicit_without_partial_json() {
        let mut cleanup = cleanup(false);
        evidence(&mut cleanup).measurement = Err("x".repeat(MAX_BYTES));
        assert_eq!(encode(&cleanup), Err(ReceiptError::Oversize));
        struct Broken;
        impl Serialize for Broken {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("expected serialization failure"))
            }
        }
        assert_eq!(bounded_json(&Broken), Err(ReceiptError::Serialization));
        let error: Value =
            serde_json::from_str(&error_payload(&cleanup, ReceiptError::Oversize)).unwrap();
        assert_eq!(error["status"], "oversize");
        assert_eq!(error["identity"]["pid"], 42);
        assert_eq!(error["identity"]["creation_time_100ns"], 123);
        assert_eq!(error["qualified"], false);
    }

    #[test]
    fn joined_receipt_sink_stops_serializing_at_cap_instead_of_allocating_full_output() {
        use serde::ser::SerializeSeq;
        use std::cell::Cell;
        struct Many<'a>(&'a Cell<usize>);
        impl Serialize for Many<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut sequence = serializer.serialize_seq(None)?;
                for i in 0..MAX_BYTES * 4 {
                    self.0.set(i + 1);
                    sequence.serialize_element(&u64::MAX)?;
                }
                sequence.end()
            }
        }
        let visited = Cell::new(0);
        assert_eq!(bounded_json(&Many(&visited)), Err(ReceiptError::Oversize));
        assert!(visited.get() < MAX_BYTES);
    }

    #[test]
    fn joined_receipt_exports_actual_rust_fixtures_only_when_explicitly_requested() {
        let Some(directory) = std::env::var_os("MOIREN_PROCESS_TRACE_RECEIPT_FIXTURES") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        assert!(directory.is_absolute() && directory.is_dir());
        let write = |name: &str, text: &str| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(directory.join(name))
                .unwrap();
            file.write_all(text.as_bytes()).unwrap();
        };
        let success = cleanup(true);
        let retained = success.process_trace.as_ref().unwrap();
        assert_eq!(retained.measurement.as_ref().unwrap().owner_generation(), 7);
        write("matching-success.json", &encode(&success).unwrap().unwrap());
        write(
            "software-archive.json",
            &bounded_json(&retained.artifact).unwrap(),
        );
        let mut refused = cleanup(false);
        write("matching-refusal.json", &encode(&refused).unwrap().unwrap());
        refused.final_observations = None;
        write("missing-joined.json", &encode(&refused).unwrap().unwrap());
        let mut multiple = cleanup(false);
        let native = Arc::get_mut(multiple.final_observations.as_mut().unwrap())
            .unwrap()
            .native
            .as_mut()
            .unwrap();
        native
            .process_delivery
            .push(native.process_delivery[0].clone());
        write("multiple-joined.json", &encode(&multiple).unwrap().unwrap());
        let mut mismatched = cleanup(false);
        mismatched.generation.0 += 1;
        write(
            "mismatched-joined.json",
            &encode(&mismatched).unwrap().unwrap(),
        );
        // Serialization is truthful even when independently valid summaries
        // cover different frame totals: the checker must reject this negative.
        let mut uncovered = cleanup(true);
        let node = uncovered.process_trace.as_ref().unwrap().origin.node;
        let short = crate::service::measured_process::tests::observation(node);
        evidence(&mut uncovered).measurement =
            short.measured(&short.process, 7).map_err(|e| e.to_string());
        write(
            "uncovered-measurement.json",
            &encode(&uncovered).unwrap().unwrap(),
        );
        write(
            "oversize-error.json",
            &error_payload(&success, ReceiptError::Oversize),
        );
        write(
            "serialization-error.json",
            &error_payload(&success, ReceiptError::Serialization),
        );
    }

    #[test]
    fn joined_receipt_default_filter_is_disabled_and_absent_trace_has_no_payload() {
        let info = flexi_logger::LogSpecification::parse("info").unwrap();
        assert!(!info.enabled(log::Level::Debug, TARGET));
        let enabled =
            flexi_logger::LogSpecification::parse("info,moiren::audio::process_trace=debug")
                .unwrap();
        assert!(enabled.enabled(log::Level::Debug, TARGET));
        let mut cleanup = cleanup(false);
        cleanup.process_trace = None;
        assert_eq!(encode(&cleanup).unwrap(), None);
        assert_eq!(MARKER, "process_trace_cleanup_receipt=");
    }

    #[test]
    fn joined_receipt_lifecycle_hook_follows_trace_finish_and_precedes_completion() {
        let source = include_str!("../managed/lifecycle.rs");
        let joined = source.find("drop(host);").unwrap();
        let finish = source.find("trace.finish(&cleanup.state)").unwrap();
        let receipt = source
            .find("process_trace::receipt::emit(&cleanup)")
            .unwrap();
        let completion = source
            .find("completion.send_or_dispose(Completion::Reaped(cleanup)")
            .unwrap();
        assert!(joined < finish && finish < receipt && receipt < completion);
        assert_eq!(
            source
                .matches("process_trace::receipt::emit(&cleanup)")
                .count(),
            1
        );
    }
}
