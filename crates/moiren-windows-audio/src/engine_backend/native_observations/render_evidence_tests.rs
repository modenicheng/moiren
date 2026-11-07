use super::*;
use crate::render_evidence::*;
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::audio_config::SampleFormat;

#[test]
fn render_evidence_readers_are_exactly_once_and_pinned_across_same_generation_replacement() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let format = RenderEvidenceFormat {
        sample_rate: 48_000,
        sample_format: SampleFormat::Float32,
        channels: 2,
        container_bits: 32,
        valid_bits: 32,
    };
    let config = RenderEvidenceConfig {
        capacity_frames: 4,
        max_packet_frames: 2,
    };
    let observations = NativeObservations::default();
    let (mut writer, reader) = render_evidence_stream(config, format).unwrap();
    let source = RenderEvidenceSource::new(
        NativeRenderEvidenceOrigin {
            schema_version: 1,
            owner_generation: 7,
            node,
            kind: NativeStreamKind::MasterOutput,
            format,
            config,
        },
        reader,
    );
    observations.publish_with_render_evidence(
        7,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        None,
        Arc::default(),
        vec![source],
    );
    observations.running(7);
    let old = observations.retain_session().unwrap();
    assert_eq!(old.render_evidence_origins().len(), 1);
    assert!(old.render_evidence_reader(node, 8).is_err());
    let mut claimed = old.render_evidence_reader(node, 7).unwrap();
    assert!(old.render_evidence_reader(node, 7).is_err());
    observations.clear(7);
    observations.publish(7, vec![], vec![], vec![]);
    assert!(
        observations
            .retain_session()
            .unwrap()
            .render_evidence_origins()
            .is_empty()
    );
    writer.stage_packet(&[42; 8], 1).unwrap();
    writer.finish_packet(-1).unwrap();
    writer.close();
    assert!(claimed.final_summary().is_none());
    writer.publish_joined();
    assert_eq!(claimed.origin().owner_generation, 7);
    assert_eq!(claimed.pop().unwrap().bytes[0], 42);
    assert!(claimed.is_finished());
    assert_eq!(claimed.final_summary().unwrap().rejected_frames, 1);
}
