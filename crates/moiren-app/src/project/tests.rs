use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use moiren_core::graph::{NodeKind, SendParams, SendTap};

use super::*;
use crate::service::{GraphEdit, InputSelection, RunLimit, ServiceGraph, SessionSpec};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
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
    fn file(&self, filename: &str) -> PathBuf {
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
            .contains("only a balance node")
    );
}
