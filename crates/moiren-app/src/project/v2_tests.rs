use super::tests::TestDirectory;
use super::*;
use crate::service::*;
use moiren_core::graph::NodeKind;
use moiren_engine::node::VirtualEndpointId;

fn document() -> ProjectDocument {
    let graph = ServiceGraph::default().snapshot();
    let mut document = ProjectDocument::from_graph("Virtual session", &graph).unwrap();
    let source = document
        .graph
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.kind == GraphNodeKind::Source)
        .unwrap()
        .id;
    document.exports.push(ExportDocument {
        endpoint_id: VirtualEndpointId::from_bytes([0x81; 16]),
        direction: ExportDirection::Playback,
        name: "Application playback".into(),
        channels: 2,
        enabled: true,
        binding: Some(source),
    });
    document
}

#[test]
fn v1_migration_preserves_physical_selectors_and_never_infers_exports() {
    let directory = TestDirectory::new();
    let path = directory.file("v1.moiren");
    let spec = SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: "virtual-looking opaque capture".into(),
        },
        output_endpoint_id: "unchanged opaque output".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.25,
        pan: 0.0,
        max_block_frames: 256,
    };
    let mut legacy = ProjectDocument::new("Legacy", &spec);
    legacy.version = 1;
    let mut json = serde_json::to_value(&legacy).unwrap();
    json.as_object_mut().unwrap().remove("exports");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let restored = load_project(&path).unwrap();
    assert_eq!(restored.version, 2);
    assert_eq!(restored.session_spec().unwrap(), Some(spec));
    assert!(restored.exports.is_empty());
    let prefs = directory.file("prefs.json");
    std::fs::write(&prefs, br#"{"version":1,"density":"compact"}"#).unwrap();
    let prefs = load_preferences_at(prefs).unwrap();
    assert_eq!(prefs.version, 2);
    assert!(prefs.export_authorizations.is_empty());
    assert_eq!(prefs.density, UiDensity::Compact);
}

#[test]
fn v2_exports_round_trip_sparse_graph_bindings_and_explicit_engine_resources() {
    let directory = TestDirectory::new();
    let path = directory.file("v2.moiren");
    let mut document = document();
    let old = document.exports[0].binding.unwrap();
    document
        .graph
        .as_mut()
        .unwrap()
        .nodes
        .iter_mut()
        .find(|node| node.id == old)
        .unwrap()
        .id = 200;
    for edge in &mut document.graph.as_mut().unwrap().edges {
        if edge.source_node == old {
            edge.source_node = 200;
        }
        if edge.destination_node == old {
            edge.destination_node = 200;
        }
    }
    document.exports[0].binding = Some(200);
    save_project(&path, &document).unwrap();
    let restored = load_project(&path).unwrap();
    assert_eq!(restored, document);
    let mut prep = restored.restore_graph().unwrap();
    let source = prep.exports[0].binding.unwrap();
    assert_eq!(
        prep.graph.get_node(source).unwrap().kind(),
        NodeKind::Source
    );
    prep.revision = 1;
    assert!(prep.engine_project(256).is_err());
    prep.nodes
        .iter_mut()
        .find(|state| prep.graph.get_node(state.id).unwrap().kind() == NodeKind::Sink)
        .unwrap()
        .output_endpoint_id = Some("physical output".into());
    let engine = prep.engine_project(256).unwrap();
    assert!(
        matches!(&engine.nodes[&source], moiren_engine::host::NodeConfig::Input(config)
        if matches!(config.source, moiren_engine::node::InputSource::VirtualPlayback { endpoint_id } if endpoint_id == document.exports[0].endpoint_id))
    );
}

#[test]
fn v2_rejects_duplicate_identity_binding_channel_mismatch_and_project_authorization() {
    let mut invalid = document();
    invalid.exports.push(invalid.exports[0].clone());
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.exports[0].channels = 1;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.exports[0].binding = Some(9999);
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.exports[0].direction = ExportDirection::Microphone;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.version = 1;
    assert!(invalid.migrate().is_err());
    let mut json = serde_json::to_value(document()).unwrap();
    json["exports"][0]["allow_default"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ProjectDocument>(json).is_err());
}

#[test]
fn project_import_keeps_local_authorization_and_startup_is_desired_before_first_poll() {
    let directory = TestDirectory::new();
    let project_path = directory.file("restore.moiren");
    let mut document = document();
    document.processing = Some(moiren_engine::audio_config::ProcessingSettings {
        sample_rate: 96_000,
        ..Default::default()
    });
    save_project(&project_path, &document).unwrap();
    let preferences = AppPreferences {
        restore_last_project: true,
        last_project_path: Some(project_path),
        export_authorizations: vec![ExportAuthorization {
            endpoint_id: document.exports[0].endpoint_id,
            allow_default: true,
        }],
        ..Default::default()
    };
    let preferences_path = directory.file("prefs.json");
    save_preferences_at(&preferences_path, &preferences).unwrap();
    let startup = StartupDocuments::load_at(&preferences_path).unwrap();
    let mut context = startup.clone().into_context().unwrap();
    assert!(
        context
            .exports
            .allow_default(document.exports[0].endpoint_id)
    );
    context.load_project(document.clone()).unwrap();
    assert!(
        context
            .exports
            .allow_default(document.exports[0].endpoint_id)
    );
    let no_authorization = StartupDocuments {
        preferences: AppPreferences::default(),
        project: Some(document.clone()),
    }
    .into_context()
    .unwrap();
    assert!(
        !no_authorization
            .exports
            .allow_default(document.exports[0].endpoint_id)
    );
    struct InspectStartup(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl ServiceDriver for InspectStartup {
        fn request(&mut self, _: &mut ServiceContext, _: &mut JobPool, _: AppRequest) {}
        fn poll(&mut self, context: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
            assert_eq!(context.audio_settings.processing.sample_rate, 96_000);
            assert_eq!(context.graph.snapshot().exports.len(), 1);
            assert!(context.loaded_project.is_some());
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn is_drained(&self) -> bool {
            true
        }
    }
    let checked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let runtime =
        ServiceRuntime::spawn_with_startup(InspectStartup(checked.clone()), startup).unwrap();
    let handle = runtime.handle();
    let snapshot = handle.try_snapshot().unwrap();
    assert_eq!(snapshot.graph.exports.len(), 1);
    assert_eq!(snapshot.graph.active_revision, None);
    assert_eq!(snapshot.phase, SessionPhase::Idle);
    assert_eq!(
        snapshot.exports[0].resource,
        ExportResourceState::Unavailable
    );
    assert!(!snapshot.exports[0].gate_open);
    handle.request_exit();
    runtime.join().unwrap();
    assert!(checked.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn failed_project_preflight_preserves_candidate_active_revision_and_local_state() {
    let mut context = ServiceContext::default();
    let revision = context.graph.snapshot().candidate_revision;
    context.graph.preparing(SessionGeneration(7), revision);
    context.graph.running(SessionGeneration(7), revision);
    let before_graph = context.graph.snapshot();
    let before_audio = context.audio_settings.clone();

    let mut invalid = document();
    invalid.exports[0].channels = 3;
    assert!(context.load_project(invalid).is_err());

    assert_eq!(context.graph.snapshot(), before_graph);
    assert_eq!(context.audio_settings, before_audio);
    assert!(context.loaded_project.is_none());
}

#[test]
fn duplicate_or_v1_local_authorization_is_rejected_before_resource_creation() {
    let id = document().exports[0].endpoint_id;
    let grant = ExportAuthorization {
        endpoint_id: id,
        allow_default: true,
    };
    let duplicate = AppPreferences {
        export_authorizations: vec![grant, grant],
        ..Default::default()
    };
    assert!(duplicate.validate().is_err());
    let legacy = AppPreferences {
        version: 1,
        export_authorizations: vec![grant],
        ..Default::default()
    };
    assert!(legacy.migrate().is_err());
    let startup = StartupDocuments {
        preferences: duplicate,
        project: None,
    };
    assert!(ServiceRuntime::spawn_with_startup(TestDriver, startup).is_err());
    struct TestDriver;
    impl ServiceDriver for TestDriver {
        fn request(&mut self, _: &mut ServiceContext, _: &mut JobPool, _: AppRequest) {
            panic!("invalid startup must not start service");
        }
        fn is_drained(&self) -> bool {
            true
        }
    }
}
