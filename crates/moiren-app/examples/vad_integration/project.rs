use moiren_app::{
    project::{
        AppPreferences, ExportDocument, GraphEdgeDocument, GraphNodeKind, ProjectDocument,
        SendTapDocument, StartupDocuments, save_preferences_at, save_project,
    },
    service::{ExportDirection, ServiceGraph},
};
use moiren_engine::node::VirtualEndpointId;
use std::path::Path;

pub(super) fn document() -> anyhow::Result<ProjectDocument> {
    let mut document = ProjectDocument::from_graph(
        "Moiren VAD integration stage1",
        &ServiceGraph::default().snapshot(),
    )?;
    let graph = document
        .graph
        .as_mut()
        .expect("from_graph retains topology");
    graph.nodes.retain(|node| node.kind != GraphNodeKind::Pan);
    for node in &mut graph.nodes {
        node.source = None;
        node.output_endpoint_id = None;
        if node.kind == GraphNodeKind::Gain {
            node.gain = 0.5;
        }
    }
    let node = |kind| {
        graph
            .nodes
            .iter()
            .find(|node| node.kind == kind)
            .expect("fixed session node")
            .id
    };
    let [source, gain, sink] = [
        node(GraphNodeKind::Source),
        node(GraphNodeKind::Gain),
        node(GraphNodeKind::Sink),
    ];
    graph.edges = [(source, gain), (gain, sink)]
        .into_iter()
        .map(|(source_node, destination_node)| GraphEdgeDocument {
            source_node,
            source_output: 0,
            destination_node,
            destination_input: 0,
            gain: 1.0,
            pan: 0.0,
            mute: false,
            tap: SendTapDocument::PostFader,
        })
        .collect();
    document.exports = [
        (
            VirtualEndpointId::from_bytes(*b"MoirenAppVadPlay"),
            ExportDirection::Playback,
            source,
            "Moiren integration playback",
        ),
        (
            VirtualEndpointId::from_bytes(*b"MoirenAppVadMic0"),
            ExportDirection::Microphone,
            sink,
            "Moiren integration microphone",
        ),
    ]
    .into_iter()
    .map(|(endpoint_id, direction, binding, name)| ExportDocument {
        endpoint_id,
        direction,
        name: name.into(),
        channels: 2,
        enabled: true,
        binding: Some(binding),
    })
    .collect();
    document.processing = Some(Default::default());
    document.validate()?;
    Ok(document)
}

pub(super) fn persist(directory: &Path) -> anyhow::Result<StartupDocuments> {
    let project_path = directory.join("project.moiren");
    let preferences_path = directory.join("preferences.json");
    anyhow::ensure!(
        !project_path.exists() && !preferences_path.exists(),
        "probe will not overwrite existing documents"
    );
    save_project(&project_path, &document()?)?;
    save_preferences_at(
        &preferences_path,
        &AppPreferences {
            restore_last_project: true,
            last_project_path: Some(project_path),
            ..Default::default()
        },
    )?;
    // Exercise the actual disk decoder and startup restore, not a prebuilt context.
    Ok(StartupDocuments::load_at(preferences_path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_app::{project::GraphNodeKind, service::AudioSettings};
    use moiren_engine::{
        host::{NodeConfig, ParameterTarget},
        node::{InputSource, OutputTarget},
        processor::Gain,
    };

    #[test]
    fn v2_fixture_is_stereo_virtual_playback_gain_half_virtual_microphone() {
        let document = document().unwrap();
        assert_eq!(document.version, 2);
        document.validate().unwrap();
        let graph = document.graph.as_ref().unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(document.exports.len(), 2);
        assert!(graph.nodes.iter().all(|node| node.channels == 2));
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.source.is_none() && node.output_endpoint_id.is_none())
        );
        let gain_document = graph
            .nodes
            .iter()
            .find(|node| node.kind == GraphNodeKind::Gain)
            .unwrap();
        assert_eq!(gain_document.gain, 0.5);
        let restored = document.restore_graph().unwrap();
        let engine = restored
            .configured_engine_project(256, &AudioSettings::default())
            .unwrap();
        let mut source = None;
        let mut gain = None;
        let mut sink = None;
        for (&id, node) in &engine.nodes {
            match node {
                NodeConfig::Input(input) => {
                    assert!(matches!(input.source, InputSource::VirtualPlayback { .. }));
                    source = Some(id);
                }
                NodeConfig::Gain { .. } => gain = Some(id),
                NodeConfig::Output(output) => {
                    assert!(matches!(
                        output.target,
                        OutputTarget::VirtualMicrophone { .. }
                    ));
                    sink = Some(id);
                }
                other => panic!("unexpected resource or processor: {other:?}"),
            }
        }
        let [source, gain, sink] = [source.unwrap(), gain.unwrap(), sink.unwrap()];
        assert!(
            engine
                .graph
                .edges()
                .iter()
                .any(|edge| edge.src() == source && edge.dst() == gain)
        );
        assert!(
            engine
                .graph
                .edges()
                .iter()
                .any(|edge| edge.src() == gain && edge.dst() == sink)
        );
        assert!(engine.parameters.iter().any(|value| {
            value.target
                == ParameterTarget::Node {
                    node: gain,
                    parameter: Gain::LEVEL,
                }
                && value.value == moiren_core::protocol::ParamValue::Float(0.5)
        }));
    }

    #[test]
    fn restored_virtual_fixture_retains_typed_bindings_without_physical_selectors() {
        let mut context = moiren_app::service::ServiceContext::default();
        context.load_project(document().unwrap()).unwrap();
        let snapshot = context.graph.snapshot();
        assert!(snapshot.active_revision.is_none());
        assert!(snapshot.pending_revision.is_none());
        assert_eq!(snapshot.exports.len(), 2);
        assert!(
            snapshot
                .nodes
                .iter()
                .all(|node| node.source.is_none() && node.output_endpoint_id.is_none())
        );
        for export in &snapshot.exports {
            assert!(export.enabled);
            assert_eq!(export.channels, 2);
            let bound = snapshot
                .graph
                .get_node(export.binding.expect("fixture export must be bound"))
                .unwrap();
            let expected = match export.direction {
                ExportDirection::Playback => moiren_core::graph::NodeKind::Source,
                ExportDirection::Microphone => moiren_core::graph::NodeKind::Sink,
            };
            assert_eq!(bound.kind(), expected);
        }
    }

    #[test]
    fn persistence_reloads_startup_without_touching_user_preferences() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-vad-integration-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let startup = persist(&directory).unwrap();
        assert_eq!(startup.project, Some(document().unwrap()));
        assert!(startup.preferences.restore_last_project);
        assert!(startup.preferences.export_authorizations.is_empty());
        assert_eq!(startup.preferences.audio, AudioSettings::default());
        let project = startup.project.as_ref().unwrap();
        let ids: Vec<_> = project
            .exports
            .iter()
            .map(|export| export.endpoint_id)
            .collect();
        let context = startup.into_context().unwrap();
        assert!(ids.into_iter().all(|id| !context.exports.allow_default(id)));
        // Remove only the exact files created by this test, never a shared tree.
        std::fs::remove_file(directory.join("project.moiren")).unwrap();
        std::fs::remove_file(directory.join("preferences.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
