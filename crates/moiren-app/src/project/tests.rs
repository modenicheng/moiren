use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use moiren_core::graph::{NodeKind, SendParams, SendTap};
use moiren_engine::{
    host::NodeConfig,
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
    rack::{RackProcessor, RackSettings, RackSlot},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};

use super::*;
use crate::service::{
    BuiltinState, GraphEdit, InputSelection, RunLimit, ServiceGraph, SessionSpec,
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn audio_defaults_and_directional_overrides_round_trip_without_active_state() {
    use moiren_engine::{
        audio_config::{AudioBackend, ProcessingPrecision, SampleFormat},
        node::DeviceMode,
    };
    let directory = TestDirectory::new();
    let path = directory.file("audio-preferences.json");
    let mut preferences = AppPreferences::default();
    preferences.audio.backend = AudioBackend::Asio;
    preferences.audio.processing.sample_rate = 96_000;
    preferences.audio.processing.precision = ProcessingPrecision::F64;
    preferences.audio.input.buffer_frames = Some(128);
    let mut output = preferences.audio.output;
    output.sample_rate = Some(88_200);
    output.format = SampleFormat::Pcm24;
    output.mode = DeviceMode::Exclusive;
    output.buffer_frames = Some(512);
    preferences
        .audio
        .output_overrides
        .insert("{stable-endpoint}".into(), output);
    save_preferences_at(&path, &preferences).unwrap();
    let restored = load_preferences_at(&path).unwrap();
    assert_eq!(restored, preferences);
    assert_eq!(
        restored.audio.effective_input("{stable-endpoint}"),
        restored.audio.input
    );
    assert_eq!(restored.audio.effective_output("{stable-endpoint}"), output);
    let document: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(document.get("active_audio_settings").is_none());
}

#[test]
fn invalid_audio_preferences_do_not_replace_a_valid_file() {
    let directory = TestDirectory::new();
    let path = directory.file("preferences.json");
    save_preferences_at(&path, &AppPreferences::default()).unwrap();
    let original = fs::read(&path).unwrap();
    let mut preferences = AppPreferences::default();
    preferences.audio.output.buffer_frames = Some(480);
    assert!(save_preferences_at(&path, &preferences).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn old_preferences_inherit_audio_defaults_and_projects_persist_processing_only() {
    use moiren_engine::audio_config::{AudioSettings, ProcessingPrecision};
    let directory = TestDirectory::new();
    let path = directory.file("preferences.json");
    fs::write(&path, br#"{"version":1,"density":"compact"}"#).unwrap();
    assert_eq!(
        load_preferences_at(&path).unwrap().audio,
        AudioSettings::default()
    );
    let mut document =
        ProjectDocument::from_graph("96k project", &ServiceGraph::default().snapshot()).unwrap();
    let mut processing = AudioSettings::default().processing;
    processing.sample_rate = 96_000;
    processing.precision = ProcessingPrecision::F64;
    document.processing = Some(processing);
    let project_path = directory.file("project.moiren");
    save_project(&project_path, &document).unwrap();
    assert_eq!(
        load_project(&project_path).unwrap().processing,
        Some(processing)
    );
    let mut old = serde_json::to_value(document).unwrap();
    old.as_object_mut().unwrap().remove("processing");
    let restored: ProjectDocument = serde_json::from_value(old).unwrap();
    assert_eq!(restored.processing, None);
    restored.validate().unwrap();
}

#[test]
fn project_processing_block_takes_precedence_over_legacy_session_block() {
    let spec = session(InputSelection::Tone {
        frequency_hz: 440.0,
    });
    let mut document = ProjectDocument::new("Processing settings", &spec);
    assert_eq!(
        document.session_spec().unwrap().unwrap().max_block_frames,
        512
    );
    document.processing = Some(moiren_engine::audio_config::ProcessingSettings {
        block_frames: 1024,
        ..Default::default()
    });
    assert_eq!(
        document.session_spec().unwrap().unwrap().max_block_frames,
        1024
    );
    assert_eq!(document.session.as_ref().unwrap().max_block_frames, 512);
}

fn builtin_settings_cases() -> Vec<(NodeKind, BuiltinState)> {
    vec![
        (
            NodeKind::Eq,
            BuiltinState::Eq(EqSettings {
                frequency_hz: 2345.0,
                gain_db: -3.0,
                q: 2.5,
            }),
        ),
        (
            NodeKind::Gate,
            BuiltinState::Gate(GateSettings {
                threshold_db: -35.0,
                attack_ms: 3.0,
                release_ms: 250.0,
                hold_ms: 30.0,
                range_db: -60.0,
            }),
        ),
        (
            NodeKind::Limiter,
            BuiltinState::Limiter(LimiterSettings {
                threshold_db: -4.0,
                release_ms: 200.0,
                input_gain_db: 3.0,
            }),
        ),
        (
            NodeKind::Delay,
            BuiltinState::Delay(DelaySettings {
                delay_ms: 375.0,
                feedback: 0.4,
                mix: 0.3,
                max_delay_ms: 1500.0,
            }),
        ),
        (
            NodeKind::ChannelMapper,
            BuiltinState::ChannelMapper(ChannelMapperSettings {
                mapping: vec![1, 0],
                ..Default::default()
            }),
        ),
        (
            NodeKind::Recorder,
            BuiltinState::Recorder(RecorderSettings {
                capacity_frames: 1234,
            }),
        ),
        (
            NodeKind::Meter,
            BuiltinState::Meter(MeterSettings { window_frames: 777 }),
        ),
        (
            NodeKind::SpectrumAnalyzer,
            BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                fft_size: 256,
                hop_frames: 64,
            }),
        ),
        (
            NodeKind::Rack,
            BuiltinState::Rack(RackSettings {
                slots: vec![
                    RackSlot {
                        id: 47,
                        bypass: true,
                        processor: RackProcessor::Gain { initial: 0.75 },
                    },
                    RackSlot {
                        id: 9,
                        bypass: false,
                        processor: RackProcessor::Pan {
                            initial: -0.25,
                            mono: true,
                            swap: false,
                        },
                    },
                ],
                ..Default::default()
            }),
        ),
    ]
}

#[test]
fn builtin_settings_persist_and_reach_engine_project_without_loss() {
    let mut service = ServiceGraph::default();
    for (kind, state) in builtin_settings_cases() {
        service
            .edit(GraphEdit::AddNode { kind, channels: 2 })
            .unwrap();
        let node = service.snapshot().graph.nodes().last().unwrap().id();
        service.edit(GraphEdit::SetBuiltin { node, state }).unwrap();
    }
    let sink = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Sink)
        .unwrap()
        .id();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "output".into(),
        })
        .unwrap();

    let dir = TestDirectory::new();
    let path = dir.file("builtins.moiren");
    let document = ProjectDocument::from_graph("Builtins", &service.snapshot()).unwrap();
    save_project(&path, &document).unwrap();
    let decoded = load_project(&path).unwrap();
    assert_eq!(decoded, document);
    let (graph, nodes) = decoded.graph.unwrap().restore().unwrap();
    let mut restored = ServiceGraph::default();
    restored.replace(graph, nodes).unwrap();

    let snapshot = restored.snapshot();
    let project = restored.preparation().engine_project(256).unwrap();
    for (kind, expected_state) in builtin_settings_cases() {
        let node = snapshot
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == kind)
            .unwrap();
        assert_eq!(snapshot.node(node.id()).unwrap().builtin, expected_state);
        let expected_config = match expected_state {
            BuiltinState::Eq(settings) => NodeConfig::Eq(settings),
            BuiltinState::Gate(settings) => NodeConfig::Gate(settings),
            BuiltinState::Limiter(settings) => NodeConfig::Limiter(settings),
            BuiltinState::Delay(settings) => NodeConfig::Delay(settings),
            BuiltinState::ChannelMapper(settings) => NodeConfig::ChannelMapper(settings),
            BuiltinState::Recorder(settings) => NodeConfig::Recorder(settings),
            BuiltinState::Meter(settings) => NodeConfig::Meter(settings),
            BuiltinState::SpectrumAnalyzer(settings) => NodeConfig::SpectrumAnalyzer(settings),
            BuiltinState::Rack(settings) => NodeConfig::Rack(settings),
            BuiltinState::None => unreachable!("fixture has explicit settings"),
        };
        assert_eq!(project.nodes[&node.id()], expected_config);
    }
}

#[test]
fn legacy_graph_documents_default_builtin_state_on_restore() {
    let mut service = ServiceGraph::default();
    for (kind, _) in builtin_settings_cases() {
        service
            .edit(GraphEdit::AddNode { kind, channels: 2 })
            .unwrap();
    }
    let document = GraphDocument::from_snapshot(&service.snapshot()).unwrap();
    let mut legacy = serde_json::to_value(document).unwrap();
    for node in legacy["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("builtin");
        node.as_object_mut().unwrap().remove("input_channels");
    }
    let legacy: GraphDocument = serde_json::from_value(legacy).unwrap();
    assert!(
        legacy
            .nodes
            .iter()
            .all(|node| node.builtin == BuiltinState::None)
    );
    let (graph, nodes) = legacy.restore().unwrap();
    for node in graph.nodes() {
        assert_eq!(
            nodes
                .iter()
                .find(|state| state.id == node.id())
                .unwrap()
                .builtin,
            BuiltinState::for_kind(node.kind())
        );
    }
    let mut restored = ServiceGraph::default();
    restored.replace(graph, nodes).unwrap();
    assert_eq!(restored.snapshot().active_revision, None);
}

#[test]
fn channel_mapper_persistence_preserves_matrix_and_distinct_widths() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::ChannelMapper,
            channels: 2,
        })
        .unwrap();
    let mut document = GraphDocument::from_snapshot(&service.snapshot()).unwrap();
    let mapper = document
        .nodes
        .iter_mut()
        .find(|node| node.kind == GraphNodeKind::ChannelMapper)
        .unwrap();
    mapper.input_channels = Some(3);
    mapper.builtin = BuiltinState::ChannelMapper(ChannelMapperSettings {
        matrix: vec![vec![0.5, 0.0, 0.25], vec![0.0, 0.75, -0.5]],
        ..Default::default()
    });
    let decoded: GraphDocument =
        serde_json::from_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
    let (graph, nodes) = decoded.restore().unwrap();
    let mapper = graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::ChannelMapper)
        .unwrap();
    assert_eq!(mapper.inputs()[0].channels(), 3);
    assert_eq!(mapper.channels(), 2);
    service.replace(graph, nodes).unwrap();
    assert_eq!(
        GraphDocument::from_snapshot(&service.snapshot()).unwrap(),
        document
    );

    let mut invalid = document.clone();
    invalid
        .nodes
        .iter_mut()
        .find(|node| node.kind == GraphNodeKind::ChannelMapper)
        .unwrap()
        .input_channels = Some(0);
    assert!(invalid.restore().is_err());
    let mut invalid = document.clone();
    invalid
        .nodes
        .iter_mut()
        .find(|node| node.kind == GraphNodeKind::ChannelMapper)
        .unwrap()
        .input_channels = Some(257);
    assert!(invalid.restore().is_err());
    let mut invalid = document;
    invalid.nodes[0].input_channels = Some(2);
    assert!(invalid.restore().is_err());
}

#[test]
fn invalid_builtin_documents_and_replacements_leave_candidate_unchanged() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    let document = GraphDocument::from_snapshot(&service.snapshot()).unwrap();
    for builtin in [
        BuiltinState::Eq(EqSettings {
            q: 0.0,
            ..Default::default()
        }),
        BuiltinState::ChannelMapper(ChannelMapperSettings::default()),
    ] {
        let mut invalid = document.clone();
        invalid.nodes.last_mut().unwrap().builtin = builtin.clone();
        assert!(invalid.restore().is_err());

        let preparation = service.preparation();
        let mut nodes = preparation.nodes;
        nodes.last_mut().unwrap().builtin = builtin;
        let before = service.snapshot();
        assert!(service.replace(preparation.graph, nodes).is_err());
        assert_eq!(service.snapshot(), before);
    }

    let mut invalid = serde_json::to_value(document).unwrap();
    invalid["nodes"][4]["builtin"] = serde_json::json!({ "kind": "unknown_processor" });
    assert!(serde_json::from_value::<GraphDocument>(invalid).is_err());
}

#[test]
fn compressor_properties_restore_and_prepare_the_saved_values() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Compressor,
            channels: 2,
        })
        .unwrap();
    let document = ProjectDocument::from_graph("Dynamics", &service.snapshot()).unwrap();
    let mut json = serde_json::to_value(&document).unwrap();
    let compressor = json["graph"]["nodes"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    compressor["compressor"] = serde_json::json!({
        "input_gain_db": 2.0, "threshold_db": -24.0, "ratio": 3.0,
        "attack_ms": 5.0, "release_ms": 200.0, "hold_ms": 20.0,
        "knee_db": 3.0, "makeup_gain_db": 4.0, "output_gain_db": -2.0, "mix": 0.75
    });
    let restored: ProjectDocument =
        serde_json::from_value(json).expect("editable compressor properties must load");
    restored.validate().unwrap();
    let (graph, nodes) = restored.graph.as_ref().unwrap().restore().unwrap();
    let mut project_graph = crate::service::GraphPreparation {
        exports: Vec::new(),
        graph,
        nodes,
        revision: 1,
    };
    let sink_id = project_graph
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Sink)
        .unwrap()
        .id();
    project_graph
        .nodes
        .iter_mut()
        .find(|node| node.id == sink_id)
        .unwrap()
        .output_endpoint_id = Some("output".into());
    let project = project_graph.engine_project(256).unwrap();
    let compressor_id = project_graph
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Compressor)
        .unwrap()
        .id();
    for (parameter, value) in [
        (crate::service::CompressorParameter::Threshold, -24.),
        (crate::service::CompressorParameter::Hold, 20.),
        (crate::service::CompressorParameter::Mix, 0.75),
    ] {
        assert!(project.parameters.iter().any(|desired| desired.target
            == moiren_engine::host::ParameterTarget::Node {
                node: compressor_id,
                parameter: parameter.id()
            }
            && desired.value == moiren_core::protocol::ParamValue::Float(value)));
    }
    assert_eq!(
        serde_json::to_value(&restored).unwrap()["graph"]["nodes"][4]["compressor"]["ratio"],
        3.0
    );
}

#[test]
fn mixer_layout_and_node_settings_round_trip_with_document_local_ids() {
    let mut service = ServiceGraph::default();
    let id = service.snapshot().nodes[0].id;
    let settings = crate::service::MixerState {
        gain: 0.5,
        muted: true,
        solo: true,
        channel_mode: 3,
        ..Default::default()
    };
    service
        .edit(GraphEdit::SetMixer {
            node: id,
            state: settings,
        })
        .unwrap();
    let mut document = ProjectDocument::from_graph("Mixer", &service.snapshot()).unwrap();
    document.layout.mixer_order = vec![3, 0, 2, 1];
    document.layout.mixer_width = 220.;
    document.layout.mixer_dock_width = 440.;
    let json = serde_json::to_string(&document).unwrap();
    let restored: ProjectDocument = serde_json::from_str(&json).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.layout, document.layout);
    assert_eq!(
        restored.graph.as_ref().unwrap().restore().unwrap().1[0].mixer,
        settings
    );
    let mut invalid = restored.clone();
    invalid.layout.mixer_order = vec![0, 0];
    assert!(invalid.validate().is_err());
    invalid.layout.mixer_order = vec![99];
    assert!(invalid.validate().is_err());
    let mut old = serde_json::to_value(&restored).unwrap();
    for field in ["mixer_order", "mixer_width", "mixer_dock_width"] {
        old["layout"].as_object_mut().unwrap().remove(field);
    }
    for node in old["graph"]["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("mixer");
        node.as_object_mut().unwrap().remove("compressor");
    }
    let compatible: ProjectDocument = serde_json::from_value(old).unwrap();
    compatible.validate().unwrap();
    assert_eq!(compatible.layout.mixer_width, 176.);
    assert!(
        compatible
            .graph
            .unwrap()
            .nodes
            .iter()
            .all(|node| node.compressor == Default::default())
    );
}

pub(super) struct TestDirectory(PathBuf);
impl TestDirectory {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moiren-project-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn file(&self, filename: &str) -> PathBuf {
        self.0.join(filename)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn session(source: InputSelection) -> SessionSpec {
    SessionSpec {
        source,
        output_endpoint_id: "{stable-output-identity}".into(),
        limit: RunLimit::Seconds(90),
        gain: 0.35,
        pan: -0.25,
        max_block_frames: 512,
    }
}

#[test]
fn every_session_source_round_trips_with_presentation_state() {
    let directory = TestDirectory::new();
    let path = directory.file("session.moiren");
    for source in [
        InputSelection::Physical {
            endpoint_id: "{input-device}".into(),
        },
        InputSelection::Process {
            pid: 12,
            creation_time_100ns: 987654321,
        },
        InputSelection::Tone {
            frequency_hz: 997.0,
        },
    ] {
        let spec = session(source);
        let mut document = ProjectDocument::new("监听工程", &spec);
        document.layout.workspace = WorkspaceDocument::Matrix;
        document.layout.camera_x = 150.5;
        document.layout.camera_y = -70.0;
        document.layout.zoom = 0.75;
        document.layout.nodes.push(NodeLayout {
            node_id: 0,
            x: -30.0,
            y: 55.0,
        });
        save_project(&path, &document).unwrap();
        let restored = load_project(&path).unwrap();
        assert_eq!(restored, document);
        assert_eq!(restored.session_spec().unwrap(), Some(spec));
    }
}

#[test]
fn graph_round_trip_preserves_bus_inputs_bindings_and_send_parameters() {
    let mut service = ServiceGraph::default();
    let spec = session(InputSelection::Tone {
        frequency_hz: 440.0,
    });
    service.configure_session(&spec).unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .unwrap();
    let bus = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Bus)
        .unwrap()
        .id();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    let snapshot = service.snapshot();
    let source = snapshot
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Source)
        .unwrap();
    let input = snapshot.graph.get_node(bus).unwrap().inputs()[1].id();
    service
        .edit(GraphEdit::Connect {
            source: source.outputs()[0].id(),
            destination: input,
        })
        .unwrap();
    let snapshot = service.snapshot();
    let edge = snapshot.graph.edges().last().unwrap().id();
    service
        .edit(GraphEdit::SetSend {
            edge,
            params: SendParams {
                gain: 1.75,
                pan: -0.4,
                mute: true,
                tap: SendTap::PreFader,
            },
        })
        .unwrap();
    let gain = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Gain)
        .unwrap()
        .id();
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 4.0,
        })
        .unwrap();
    let document = GraphDocument::from_snapshot(&service.snapshot()).unwrap();
    let (restored, states) = document.restore().unwrap();
    let mut loaded_service = ServiceGraph::default();
    loaded_service.replace(restored, states).unwrap();
    assert_eq!(
        GraphDocument::from_snapshot(&loaded_service.snapshot()).unwrap(),
        document
    );
}

#[test]
fn graph_load_rejects_missing_ports_cycles_and_duplicate_node_ids() {
    let snapshot = ServiceGraph::default().snapshot();
    let document = GraphDocument::from_snapshot(&snapshot).unwrap();
    let mut invalid = document.clone();
    invalid.edges[0].source_output = usize::MAX;
    assert!(
        invalid
            .restore()
            .unwrap_err()
            .to_string()
            .contains("source output index")
    );
    let mut invalid = document.clone();
    invalid.nodes[1].id = invalid.nodes[0].id;
    assert!(
        invalid
            .restore()
            .unwrap_err()
            .to_string()
            .contains("duplicate document node ID")
    );
    let mut invalid = document;
    invalid.edges.remove(0);
    invalid.edges.push(GraphEdgeDocument {
        source_node: 2,
        source_output: 0,
        destination_node: 1,
        destination_input: 0,
        gain: 1.0,
        pan: 0.0,
        mute: false,
        tap: SendTapDocument::PostFader,
    });
    assert!(invalid.restore().unwrap_err().to_string().contains("cycle"));
}

#[test]
fn invalid_save_does_not_change_previously_saved_file() {
    let directory = TestDirectory::new();
    let path = directory.file("session.moiren");
    let mut document = ProjectDocument::new(
        "Monitor",
        &session(InputSelection::Tone {
            frequency_hz: 440.0,
        }),
    );
    save_project(&path, &document).unwrap();
    let original = fs::read(&path).unwrap();
    document.session.as_mut().unwrap().pan = f64::NAN;
    assert!(
        save_project(&path, &document)
            .unwrap_err()
            .to_string()
            .contains("session")
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn failed_replacement_keeps_destination_and_cleans_temporary_file() {
    let directory = TestDirectory::new();
    let path = directory.file("not-a-file.moiren");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("sentinel"), b"preserved").unwrap();
    let document = ProjectDocument::new(
        "Monitor",
        &session(InputSelection::Tone {
            frequency_hz: 440.0,
        }),
    );
    assert!(matches!(
        save_project(&path, &document),
        Err(PersistenceError::Io {
            operation: "replace saved file",
            ..
        })
    ));
    assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"preserved");
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn load_rejects_corrupt_future_invalid_and_oversized_documents() {
    let directory = TestDirectory::new();
    let path = directory.file("bad.moiren");
    fs::write(&path, "{broken").unwrap();
    assert!(matches!(
        load_project(&path),
        Err(PersistenceError::Json { .. })
    ));
    let mut document = ProjectDocument::new(
        "Monitor",
        &session(InputSelection::Tone {
            frequency_hz: 440.0,
        }),
    );
    document.version = 99;
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(matches!(
        load_project(&path),
        Err(PersistenceError::UnsupportedVersion { version: 99, .. })
    ));
    document.version = PROJECT_VERSION;
    document.session.as_mut().unwrap().max_block_frames = 0;
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(
        load_project(&path)
            .unwrap_err()
            .to_string()
            .contains("max_block_frames")
    );
    let file = fs::File::create(&path).unwrap();
    file.set_len(8 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(
        load_project(&path),
        Err(PersistenceError::TooLarge { .. })
    ));
}

#[test]
fn preferences_persist_density_and_missing_file_uses_defaults() {
    let directory = TestDirectory::new();
    let path = directory.file("settings/preferences.json");
    assert_eq!(
        load_preferences_at(&path).unwrap(),
        AppPreferences::default()
    );
    let preferences = AppPreferences {
        density: UiDensity::Compact,
        restore_last_project: true,
        last_project_path: Some(directory.file("session.moiren")),
        ..AppPreferences::default()
    };
    save_preferences_at(&path, &preferences).unwrap();
    assert_eq!(load_preferences_at(&path).unwrap(), preferences);
    fs::write(&path, b"invalid preferences").unwrap();
    assert!(matches!(
        load_preferences_at(&path),
        Err(PersistenceError::Json { .. })
    ));
}

#[test]
fn preferences_restore_every_latency_profile_and_default_older_documents() {
    use crate::service::AudioLatencyProfile;
    let directory = TestDirectory::new();
    let path = directory.file("preferences.json");
    for profile in AudioLatencyProfile::ALL {
        let preferences = AppPreferences {
            latency_profile: profile,
            ..AppPreferences::default()
        };
        save_preferences_at(&path, &preferences).unwrap();
        assert_eq!(load_preferences_at(&path).unwrap(), preferences);
    }
    fs::write(&path, br#"{"version":1,"density":"compact","appearance":"dark","restore_last_project":false,"last_project_path":null}"#).unwrap();
    let restored = load_preferences_at(&path).unwrap();
    assert_eq!(restored.latency_profile, AudioLatencyProfile::Compatible);
    assert_eq!(restored.density, UiDensity::Compact);
    fs::write(&path, br#"{"version":1,"latency_profile":"turbo"}"#).unwrap();
    assert!(matches!(
        load_preferences_at(&path),
        Err(PersistenceError::Json { .. })
    ));
}

#[test]
fn unbound_candidate_graph_saves_without_creating_an_active_session() {
    let directory = TestDirectory::new();
    let path = directory.file("unbound.moiren");
    let snapshot = ServiceGraph::default().snapshot();
    let document = ProjectDocument::from_graph("New project", &snapshot).unwrap();
    save_project(&path, &document).unwrap();
    let document = load_project(&path).unwrap();
    assert!(document.session_spec().unwrap().is_none());
    let (graph, nodes) = document.graph.unwrap().restore().unwrap();
    let mut service = ServiceGraph::default();
    service.replace(graph, nodes).unwrap();
    assert_eq!(service.snapshot().active_revision, None);
    assert_eq!(service.snapshot().pending_revision, None);
}

#[test]
fn balance_monitor_controls_round_trip_without_changing_legacy_documents() {
    let mut service = ServiceGraph::default();
    let pan = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Pan)
        .unwrap()
        .id();
    service
        .edit(GraphEdit::SetMono {
            node: pan,
            mono: true,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetSwap {
            node: pan,
            swap: true,
        })
        .unwrap();
    let document = GraphDocument::from_snapshot(&service.snapshot()).unwrap();
    let json = serde_json::to_vec(&document).unwrap();
    let decoded: GraphDocument = serde_json::from_slice(&json).unwrap();
    let (graph, nodes) = decoded.restore().unwrap();
    let mut restored_service = ServiceGraph::default();
    restored_service.replace(graph, nodes).unwrap();
    let snapshot = restored_service.snapshot();
    let restored_pan = snapshot
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Pan)
        .unwrap();
    assert!(snapshot.node(restored_pan.id()).unwrap().mono);
    assert!(snapshot.node(restored_pan.id()).unwrap().swap);

    // Version 1 files written before monitor controls omitted these fields.
    let mut legacy = serde_json::to_value(&document).unwrap();
    for node in legacy["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("mono");
        node.as_object_mut().unwrap().remove("swap");
    }
    let legacy: GraphDocument = serde_json::from_value(legacy).unwrap();
    let (_, nodes) = legacy.restore().unwrap();
    assert!(nodes.iter().all(|node| !node.mono && !node.swap));
}

#[test]
fn monitor_controls_on_non_balance_nodes_are_rejected_during_project_load() {
    let mut document = GraphDocument::from_snapshot(&ServiceGraph::default().snapshot()).unwrap();
    document
        .nodes
        .iter_mut()
        .find(|node| node.kind == GraphNodeKind::Source)
        .unwrap()
        .mono = true;
    assert!(
        document
            .restore()
            .unwrap_err()
            .to_string()
            .contains("Mono 和 Swap 只能用于 Balance 节点")
    );
}
