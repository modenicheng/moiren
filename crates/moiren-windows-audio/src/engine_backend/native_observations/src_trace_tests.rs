use super::*;
use crate::virtual_owner::{BoundaryProbe, VirtualMicrophoneSink};
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::{
    boundary::{RtAudioSink, audio_bridge},
    buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    processor::ProcessContext,
};

pub(super) struct Fixture {
    pub(super) observations: NativeObservations,
    node: NodeId,
    pub(super) probe: Arc<BoundaryProbe>,
    sink: Option<VirtualMicrophoneSink>,
    arena: BufferArena<f32>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Sink, 2).unwrap();
        let (writer, _reader) = audio_bridge(2, 1024, 1 << 20).unwrap();
        let probe = Arc::new(BoundaryProbe::default());
        let sink = VirtualMicrophoneSink::new(writer, 2, 32, 48_000, 1 << 20)
            .unwrap()
            .with_clock_drift_target(384)
            .unwrap()
            .with_probe(probe.clone());
        let observations = NativeObservations::default();
        observations.publish(
            7,
            Vec::new(),
            Vec::new(),
            vec![(node, NativeStreamKind::VirtualMicrophone, probe.clone())],
        );
        let arena = BufferArena::new(
            &[BufferSlotLayout {
                channels: 2,
                capacity_frames: 32,
            }],
            1 << 20,
        )
        .unwrap();
        Self {
            observations,
            node,
            probe,
            sink: Some(sink),
            arena,
        }
    }

    pub(super) fn callbacks(&mut self, first: u64, count: u64) {
        let read = self
            .arena
            .prepare_io(&[PortAccess::Read {
                port: 0,
                slot: self.arena.slot(0).unwrap(),
            }])
            .unwrap();
        for index in first..first + count {
            self.arena
                .with_io(&read, 0..32, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        unreachable!()
                    };
                    self.sink.as_mut().unwrap().write(
                        &ProcessContext {
                            execution: None,
                            timeline_epoch: 1,
                            timeline_start: index * 32,
                            frames: 32,
                            processing_sr: 48_000.0,
                        },
                        inputs.get(0).unwrap(),
                    );
                })
                .unwrap();
        }
    }
}

#[test]
fn native_src_trace_retains_all_interpoll_callbacks_and_final_faults_after_registry_clear() {
    let mut fixture = Fixture::new();
    let retained = fixture.observations.retain_session().unwrap();
    let mut collector = retained.src_trace_collector(1024).unwrap();
    fixture.callbacks(0, 200);
    collector.poll();
    collector.poll();
    fixture.callbacks(200, 200);
    collector.poll();
    fixture.observations.clear(7);
    fixture.callbacks(400, 200);
    drop(fixture.sink.take());
    collector.finish();
    let artifact = serde_json::to_value(collector.artifact()).unwrap();
    assert_eq!(artifact["owner_generation"], 7);
    assert_eq!(artifact["history_complete"], true);
    assert_eq!(artifact["retained_checkpoints"], 600);
    let boundary = &artifact["boundaries"][0];
    assert_eq!(boundary["node"], format!("{:?}", fixture.node));
    assert_eq!(boundary["total_callbacks"], 600);
    assert_eq!(boundary["writer_finished"], true);
    let points = boundary["checkpoints"].as_array().unwrap();
    for (index, point) in points.iter().enumerate() {
        assert_eq!(point["sequence"], index as u64 + 1);
        assert_eq!(point["timeline_start"], index as u64 * 32);
    }
    assert!(points.last().unwrap()["discontinuity"].as_bool().unwrap());
    assert_eq!(artifact["qualified"], false);
    assert!(fixture.probe.src_history().unwrap().writer_finished);
}

#[test]
fn native_src_trace_refuses_missing_prefix_and_cap_without_erasing_evidence() {
    for limit in [8, 1024] {
        let mut fixture = Fixture::new();
        let retained = fixture.observations.retain_session().unwrap();
        let mut collector = retained.src_trace_collector(limit).unwrap();
        fixture.callbacks(0, 300);
        collector.poll();
        drop(fixture.sink.take());
        collector.finish();
        let artifact = serde_json::to_value(collector.artifact()).unwrap();
        assert_eq!(artifact["history_complete"], false);
        assert!(!artifact["refusal"].is_null());
        let boundary = &artifact["boundaries"][0];
        assert_eq!(boundary["missing_callbacks"], 44);
        assert_eq!(boundary["total_callbacks"], 300);
        assert_eq!(boundary["checkpoints"][0]["sequence"], 45);
        assert!(boundary["checkpoints"].as_array().unwrap().len() <= limit);
        assert_eq!(boundary["writer_finished"], true);
    }
}

#[test]
fn native_src_trace_does_not_claim_complete_while_writer_is_open() {
    let mut fixture = Fixture::new();
    let retained = fixture.observations.retain_session().unwrap();
    assert!(retained.src_trace_collector(0).is_err());
    assert!(retained.src_trace_collector(131_073).is_err());
    let mut collector = retained.src_trace_collector(8).unwrap();
    fixture.callbacks(0, 1);
    collector.finish();
    let artifact = serde_json::to_value(collector.artifact()).unwrap();
    assert_eq!(artifact["history_complete"], false);
    assert_eq!(artifact["boundaries"][0]["writer_finished"], false);
    assert!(!artifact["refusal"].is_null());
}

#[test]
fn native_src_trace_pins_owner_and_refuses_interior_gaps_and_reused_probe() {
    let mut fixture = Fixture::new();
    let retained = fixture.observations.retain_session().unwrap();
    let mut collector = retained.src_trace_collector(1024).unwrap();
    fixture.callbacks(0, 5);
    collector.poll();
    fixture
        .observations
        .publish(9, Vec::new(), Vec::new(), Vec::new());
    fixture.callbacks(5, 300);
    drop(fixture.sink.take());
    collector.finish();
    let artifact = collector.artifact();
    assert_eq!(artifact.owner_generation, 7);
    assert_eq!(artifact.boundaries[0].missing_callbacks, 44);
    assert_eq!(artifact.boundaries[0].checkpoints[4].sequence, 5);
    assert_eq!(artifact.boundaries[0].checkpoints[5].sequence, 50);
    assert!(!artifact.history_complete);
    assert_eq!(fixture.observations.snapshot().unwrap().owner_generation, 9);

    let mut fixture = Fixture::new();
    let mut collector = fixture
        .observations
        .retain_session()
        .unwrap()
        .src_trace_collector(8)
        .unwrap();
    fixture.callbacks(0, 1);
    collector.poll();
    let (writer, _reader) = audio_bridge(2, 1024, 1 << 20).unwrap();
    let reused = VirtualMicrophoneSink::new(writer, 2, 32, 48_000, 1 << 20)
        .unwrap()
        .with_probe(fixture.probe.clone());
    drop(reused);
    drop(fixture.sink.take());
    collector.finish();
    let artifact = collector.artifact();
    assert!(!artifact.history_complete);
    assert_eq!(artifact.boundaries[0].checkpoints.len(), 1);
    assert_eq!(artifact.refusal, Some(SrcTraceRefusal::HistoryUnavailable));
}

#[test]
fn native_src_trace_shared_cap_and_final_tail_never_turn_suffix_into_complete_prefix() {
    let mut first = Fixture::new();
    let mut second = Fixture::new();
    first.observations.publish(
        7,
        Vec::new(),
        Vec::new(),
        vec![
            (
                first.node,
                NativeStreamKind::VirtualPlayback,
                first.probe.clone(),
            ),
            (
                second.node,
                NativeStreamKind::VirtualMicrophone,
                second.probe.clone(),
            ),
        ],
    );
    let mut collector = first
        .observations
        .retain_session()
        .unwrap()
        .src_trace_collector(8)
        .unwrap();
    first.callbacks(0, 5);
    second.callbacks(0, 5);
    collector.poll();
    assert_eq!(collector.artifact().retained_checkpoints, 8);
    assert_eq!(collector.artifact().boundaries[1].unretained_callbacks, 2);
    first.callbacks(5, 300);
    drop(first.sink.take());
    drop(second.sink.take());
    collector.finish();
    assert!(!collector.artifact().history_complete);
    assert_eq!(collector.artifact().boundaries[0].missing_callbacks, 44);
    assert_eq!(collector.artifact().boundaries[0].total_callbacks, 305);
    assert_eq!(collector.artifact().boundaries[0].unretained_callbacks, 256);
    assert_eq!(collector.artifact().retained_checkpoints, 8);
    let before = serde_json::to_value(collector.artifact()).unwrap();
    collector.poll();
    collector.finish();
    assert_eq!(serde_json::to_value(collector.artifact()).unwrap(), before);
}

#[test]
fn native_src_trace_drains_ordered_chunks_beyond_the_lifetime_limit() {
    let mut fixture = Fixture::new();
    let mut collector = fixture
        .observations
        .retain_session()
        .unwrap()
        .src_trace_collector(256)
        .unwrap();
    let mut observed = 0_u64;
    for chunk_index in 1..=660 {
        fixture.callbacks(observed, 200);
        collector.poll();
        let chunk = collector.drain_chunk().unwrap();
        assert_eq!(chunk.owner_generation, 7);
        assert_eq!(chunk.sequence, chunk_index);
        assert_eq!(chunk.boundaries.len(), 1);
        assert_eq!(chunk.boundaries[0].node, fixture.node);
        for point in &chunk.boundaries[0].checkpoints {
            observed += 1;
            assert_eq!(point.sequence, observed);
            assert_eq!(point.timeline_start, (observed - 1) * 32);
        }
        assert!(collector.artifact().retained_checkpoints <= 256);
        assert!(collector.drain_chunk().is_none());
    }
    assert_eq!(observed, 132_000);
    drop(fixture.sink.take());
    collector.finish();
    let artifact = collector.artifact();
    assert!(artifact.history_complete, "{:?}", artifact.refusal);
    assert_eq!(artifact.drained_checkpoints, observed);
    assert_eq!(artifact.boundaries[0].drained_checkpoints, observed);
    assert_eq!(artifact.retained_checkpoints, 0);
    assert_eq!(artifact.boundaries[0].missing_callbacks, 0);
    assert!(!artifact.qualified);
}

#[test]
fn native_src_trace_drain_keeps_final_faults_and_cannot_clear_missing_evidence() {
    let mut fixture = Fixture::new();
    let mut collector = fixture
        .observations
        .retain_session()
        .unwrap()
        .src_trace_collector(256)
        .unwrap();
    fixture.callbacks(0, 200);
    collector.poll();
    collector.drain_chunk().unwrap();
    fixture.callbacks(200, 300);
    drop(fixture.sink.take());
    collector.finish();
    let chunk = collector.drain_chunk().unwrap();
    assert_eq!(chunk.boundaries[0].checkpoints[0].sequence, 245);
    assert!(
        chunk.boundaries[0]
            .checkpoints
            .last()
            .unwrap()
            .discontinuity
    );
    assert_eq!(collector.artifact().boundaries[0].missing_callbacks, 44);
    assert_eq!(collector.artifact().drained_checkpoints, 456);
    assert!(!collector.artifact().history_complete);
    assert_eq!(
        collector.artifact().refusal,
        Some(SrcTraceRefusal::MissingCallbacks)
    );
}
