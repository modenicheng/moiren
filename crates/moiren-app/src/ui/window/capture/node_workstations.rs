//! Reproducible native workstation captures, using a real service and fake IO.
//! No snapshot, meter, recording progress, or processor readout is fabricated.
use super::*;
use crate::service::{
    AppSnapshot, BackendError, BuiltinState, CatalogBackend, DeviceCatalog, DeviceRow, GraphEdit,
    GraphSnapshot, InputSelection, ManagedDriver, ProcessRow, RecordingPhase, RecordingSnapshot,
    RunLimit, ServiceGraph, SessionSpec,
};
use crate::ui::monitoring::MeterStatus;
use crate::ui::pages::graph_canvas::{CanvasWorld, NodeLayout};
use gpui_kit::test::TestWindowExt;
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::Ordering};

mod mapper_editing;
mod rack_editing;
mod recording_formats;
mod spectrum_live;
mod tone_precision;
mod wav_validation;

const ISOLATED_CONFIG: &str = "MOIREN_CAPTURE_NODES_CONFIG";
const NODE_FILTER: &str = "MOIREN_CAPTURE_NODE";
const SCENARIOS: [&str; 10] = [
    "mapper-editing",
    "rack-populated",
    "rack-editing",
    "recording-states",
    "recording-formats",
    "bus-expanded",
    "overview",
    "meter32",
    "input-long",
    "spectrum-live",
];
const LONG_INPUT_NAME: &str = "Capture studio interface - multichannel USB audio input with an intentionally long selected device name (test backend)";
const OVERVIEW_ZOOM: f32 = 0.5;
const INPUT: &str = "capture-input";
const OUTPUT: &str = "capture-output";
const PROCESS_PID: u32 = 42_000;
const PROCESS_CREATED: u64 = 1;
const TIMEOUT: Duration = Duration::from_secs(10);
const KINDS: [NodeKind; 15] = [
    NodeKind::Source,
    NodeKind::Gain,
    NodeKind::Pan,
    NodeKind::Eq,
    NodeKind::Gate,
    NodeKind::Compressor,
    NodeKind::Limiter,
    NodeKind::Delay,
    NodeKind::ChannelMapper,
    NodeKind::Bus,
    NodeKind::Rack,
    NodeKind::Meter,
    NodeKind::SpectrumAnalyzer,
    NodeKind::Recorder,
    NodeKind::Sink,
];

/// StudioShell::new reads the ordinary preference path. Relaunch with a fresh
/// config directory before creating threads; process-local env changes would
/// require unsafe mutation and could race the service or native renderer.
pub(super) fn run(directory: &Path) -> anyhow::Result<()> {
    let requested = match std::env::var(NODE_FILTER) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let filter = capture_filter(requested.as_deref())?;
    if let Some(config) = std::env::var_os(ISOLATED_CONFIG) {
        let config = PathBuf::from(config);
        let expected = config.join("Moiren").join("preferences.json");
        anyhow::ensure!(
            crate::project::preferences_path()? == expected && !expected.exists(),
            "node capture requires an empty isolated preference directory"
        );
        return run_native(directory, filter);
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let config = std::env::temp_dir().join(format!(
        "moiren-node-capture-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&config)?;
    let result = std::process::Command::new(std::env::current_exe()?)
        .arg(directory)
        .env("MOIREN_CAPTURE_NODES", "1")
        .env(ISOLATED_CONFIG, &config)
        .env("APPDATA", &config)
        .env("XDG_CONFIG_HOME", &config)
        .env_remove("MOIREN_CAPTURE_METERS")
        .env_remove("MOIREN_CAPTURE_STATES")
        .status();
    // This directory stays empty: capture disables all preference writes.
    // A nonempty directory is preserved for diagnosis, never recursively removed.
    let cleanup = std::fs::remove_dir(&config);
    anyhow::ensure!(result?.success(), "native node capture child failed");
    cleanup?;
    Ok(())
}

#[derive(Default)]
struct CaptureCatalog {
    long_names: bool,
}

impl CatalogBackend for CaptureCatalog {
    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        Ok(DeviceCatalog {
            inputs: vec![DeviceRow {
                endpoint_id: INPUT.into(),
                name: if self.long_names {
                    LONG_INPUT_NAME
                } else {
                    "Capture input (test backend)"
                }
                .into(),
            }],
            outputs: vec![DeviceRow {
                endpoint_id: OUTPUT.into(),
                name: "Capture output (test backend)".into(),
            }],
            processes: vec![ProcessRow {
                pid: PROCESS_PID,
                creation_time_100ns: PROCESS_CREATED,
                name: "Capture application (test fixture)".into(),
            }],
            default_input_endpoint_id: Some(INPUT.into()),
            default_output_endpoint_id: Some(OUTPUT.into()),
            ..Default::default()
        })
    }
}

fn fixture() -> anyhow::Result<GraphSnapshot> {
    chain_fixture(&KINDS, 2)
}

fn chain_fixture(kinds: &[NodeKind], channels: usize) -> anyhow::Result<GraphSnapshot> {
    let mut graph = ServiceGraph::default();
    for node in graph.snapshot().nodes {
        graph
            .edit(GraphEdit::RemoveNode(node.id))
            .map_err(anyhow::Error::msg)?;
    }
    for &kind in kinds {
        graph
            .edit(GraphEdit::AddNode { kind, channels })
            .map_err(anyhow::Error::msg)?;
        let id = graph
            .snapshot()
            .nodes
            .last()
            .expect("added capture node")
            .id;
        if kind == NodeKind::Bus {
            graph
                .edit(GraphEdit::AddBusInput(id))
                .map_err(anyhow::Error::msg)?;
        }
    }
    let snapshot = graph.snapshot();
    for pair in snapshot.graph.nodes().windows(2) {
        graph
            .edit(GraphEdit::Connect {
                source: pair[0].outputs()[0].id(),
                destination: pair[1].inputs()[0].id(),
            })
            .map_err(anyhow::Error::msg)?;
    }
    let input = snapshot
        .node_of_kind(NodeKind::Source)
        .expect("input")
        .0
        .id();
    let output = snapshot
        .node_of_kind(NodeKind::Sink)
        .expect("output")
        .0
        .id();
    graph
        .edit(GraphEdit::BindSource {
            node: input,
            source: InputSelection::Physical {
                endpoint_id: INPUT.into(),
            },
        })
        .map_err(anyhow::Error::msg)?;
    graph
        .edit(GraphEdit::BindOutput {
            node: output,
            endpoint_id: OUTPUT.into(),
        })
        .map_err(anyhow::Error::msg)?;
    // Validate the actual production preparation, including all builtin states.
    graph
        .preparation()
        .engine_project(256)
        .map_err(anyhow::Error::msg)?;
    Ok(graph.snapshot())
}

fn run_native(directory: &Path, filter: Option<&'static str>) -> anyhow::Result<()> {
    let graph = if filter == Some("meter32") {
        // No stereo-only processors: 32 real constant-source channels pass
        // through the managed engine and its ordinary meter subscriptions.
        chain_fixture(&[NodeKind::Source, NodeKind::Meter, NodeKind::Sink], 32)?
    } else if filter == Some("mapper-editing") {
        mapper_editing::fixture()?
    } else if filter == Some("spectrum-live") {
        spectrum_live::fixture()?
    } else {
        fixture()?
    };
    let backend = meter_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let service = ServiceRuntime::spawn_with_driver(
        ManagedDriver::new(move |_| {
            if filter == Some("spectrum-live") {
                Ok(Box::new(spectrum_live::ToneBackend(backend.clone())))
            } else {
                Ok(Box::new(backend.clone()))
            }
        })
        .with_catalog(Arc::new(CaptureCatalog {
            long_names: filter == Some("input-long"),
        })),
    )?;
    let handle = service.handle();
    let mut owned = OwnedService(Some(service));
    let initial = crate::ui::state::initial_snapshot(&handle)?;
    let directory = directory.to_owned();
    let outcome = Arc::new(std::sync::Mutex::new(None));
    let completed = outcome.clone();
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::i18n::apply(crate::i18n::UiLanguage::English);
            if let Err(error) = register_fonts(cx) {
                *completed.lock().expect("capture result") = Some(Err(error.to_string()));
                cx.quit();
                return;
            }
            let state = cx.new(|_| UiState::new(handle, initial));
            let bounds = WindowBounds::centered(size(px(1440.), px(900.)), cx);
            let opened = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(bounds),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Moiren - Node workstations (test backend)".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut shell = StudioShell::new(state, window, cx, None);
                        shell.preferences_path = None;
                        shell.project_name = "Node workstations - test backend".into();
                        shell.sidebar_open = false;
                        shell.inspector_open = false;
                        shell
                    })
                },
            );
            let (window, shell) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    *completed.lock().expect("capture result") = Some(Err(error.to_string()));
                    cx.quit();
                    return;
                }
            };
            shell.update(cx, |shell, cx| shell.spawn_poll(cx));
            cx.spawn(async move |cx| {
                let result = capture(window, &shell, &directory, graph, filter, cx).await;
                if let Err(error) = &result {
                    eprintln!("Node capture failed: {error:#}");
                }
                *completed.lock().expect("capture result") =
                    Some(result.map_err(|error| format!("{error:#}")));
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    owned.finish()?;
    let result = outcome
        .lock()
        .expect("capture result")
        .take()
        .ok_or_else(|| anyhow::anyhow!("capture closed before all images were saved"))?;
    result.map_err(anyhow::Error::msg)
}

fn dispatch(
    shell: &Entity<StudioShell>,
    request: AppRequest,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    shell
        .update(cx, |shell, cx| {
            shell
                .state
                .update(cx, |state, _| state.try_dispatch(request))
        })
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("capture request: {error:?}"))
}

async fn wait_for(
    shell: &Entity<StudioShell>,
    reason: &str,
    predicate: impl Fn(&StudioShell, &AppSnapshot) -> bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let ready = shell.update(cx, |shell, cx| -> anyhow::Result<bool> {
            shell.state.update(cx, |state, _| state.poll());
            let state = shell.state.read(cx);
            anyhow::ensure!(
                state.dispatch_error.is_none(),
                "capture dispatch failed: {:?}",
                state.dispatch_error
            );
            anyhow::ensure!(
                state.snapshot.error.is_none(),
                "capture service failed: {:?}",
                state.snapshot.error
            );
            anyhow::ensure!(
                state.snapshot.graph.last_error.is_none(),
                "capture graph failed: {:?}",
                state.snapshot.graph.last_error
            );
            anyhow::ensure!(
                state.snapshot.catalog_error.is_none(),
                "capture catalog failed: {:?}",
                state.snapshot.catalog_error
            );
            if let Some(recording) = state.snapshot.recordings.iter().find(|recording| {
                recording.phase == RecordingPhase::Failed || recording.error.is_some()
            }) {
                anyhow::bail!("capture recording failed: {:?}", recording.error);
            }
            Ok(!state.pending_request && predicate(shell, &state.snapshot))
        })?;
        if ready {
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "timed out waiting for {reason}");
        cx.background_executor()
            .timer(Duration::from_millis(25))
            .await;
    }
}

/// Small sequential groups preserve visible signal flow. Larger windows show
/// up to three cards; compact windows show up to two. Wide cards split into
/// smaller groups using the actual canvas width. Inspector captures focus one card.
/// A single-node filter saves its card and Inspector at both densities; the
/// scrolling and long-name scenarios add one interaction frame per density.
async fn capture(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    graph: GraphSnapshot,
    filter: Option<&str>,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let expected_nodes = graph.nodes.len();
    dispatch(
        shell,
        AppRequest::ReplaceGraph {
            graph: graph.graph,
            nodes: graph.nodes,
        },
        cx,
    )?;
    wait_for(
        shell,
        "capture graph and fixed catalog",
        |_, snapshot| {
            snapshot.graph.nodes.len() == expected_nodes
                && snapshot
                    .catalog
                    .inputs
                    .iter()
                    .any(|row| row.endpoint_id == INPUT)
                && snapshot
                    .catalog
                    .outputs
                    .iter()
                    .any(|row| row.endpoint_id == OUTPUT)
        },
        cx,
    )
    .await?;
    if let Some(selected) = filter {
        let edits = shell.update(cx, |shell, cx| {
            expansion_edits(&shell.state.read(cx).snapshot.graph, selected)
        })?;
        for edit in edits {
            let revision = shell.update(cx, |shell, cx| {
                shell.state.read(cx).snapshot.graph.candidate_revision
            });
            dispatch(shell, AppRequest::EditGraph(edit), cx)?;
            wait_for(
                shell,
                "expanded capture configuration",
                |_, snapshot| snapshot.graph.candidate_revision > revision,
                cx,
            )
            .await?;
        }
    }
    dispatch(
        shell,
        AppRequest::Start(SessionSpec {
            source: if filter == Some("spectrum-live") {
                InputSelection::Tone {
                    frequency_hz: spectrum_live::TONE_HZ,
                }
            } else {
                InputSelection::Physical {
                    endpoint_id: INPUT.into(),
                }
            },
            output_endpoint_id: OUTPUT.into(),
            limit: RunLimit::UntilStopped,
            gain: 0.25,
            pan: 0.,
            max_block_frames: 256,
        }),
        cx,
    )?;
    wait_for(
        shell,
        "running capture graph",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
        },
        cx,
    )
    .await?;

    if filter == Some("recording-states") {
        return capture_recording_states(window, shell, directory, cx).await;
    }
    if filter == Some("recording-formats") {
        return recording_formats::capture_formats(window, shell, directory, cx).await;
    }
    if filter == Some("mapper-editing") {
        return mapper_editing::capture_mapper(window, shell, directory, cx).await;
    }
    if filter == Some("rack-editing") {
        return rack_editing::capture_rack(window, shell, directory, cx).await;
    }
    if filter == Some("spectrum-live") {
        return spectrum_live::capture_live(window, shell, directory, cx).await;
    }
    if filter == Some("overview") {
        return capture_overviews(window, shell, directory, cx).await;
    }

    let mut manifest = Vec::new();
    let kinds = KINDS
        .into_iter()
        .filter(|kind| filter.is_none_or(|selected| filtered_kind(selected) == Some(*kind)))
        .collect::<Vec<_>>();
    for (density, width, height, count) in
        [("comfortable", 1440., 900., 3), ("compact", 1024., 720., 2)]
    {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        let groups = if let Some(first) = kinds.first() {
            // Settle the real panel/canvas before packing; window dimensions
            // alone do not include rails and reserved capture margins.
            let (_, measured) = prepare_group(window, shell, &[*first], false, true, cx).await?;
            shell.update(cx, |shell, cx| {
                capture_groups(
                    &shell.state.read(cx).snapshot,
                    &kinds,
                    measured.viewport,
                    count,
                )
            })?
        } else {
            Vec::new()
        };
        for (index, kinds) in groups.into_iter().enumerate() {
            let name = if let Some(selected) = filter {
                format!("nodes-{density}-card-{selected}")
            } else {
                format!(
                    "nodes-{density}-{:02}-{}",
                    index + 1,
                    kinds
                        .iter()
                        .map(|kind| slug(*kind))
                        .collect::<Vec<_>>()
                        .join("-")
                )
            };
            manifest.push(
                capture_group(window, shell, directory, &name, kinds, false, true, cx).await?,
            );
            if filter == Some("meter32") {
                manifest.push(
                    capture_meter_end(
                        window,
                        shell,
                        directory,
                        &format!("{name}-last-channel"),
                        cx,
                    )
                    .await?,
                );
            } else if filter == Some("input-long") {
                manifest.push(
                    capture_input_menu(window, shell, directory, &format!("{name}-menu"), cx)
                        .await?,
                );
            }
        }
        for &kind in &kinds {
            let name = format!(
                "nodes-{density}-inspector-{}",
                filter.unwrap_or_else(|| slug(kind))
            );
            manifest.push(
                capture_group(window, shell, directory, &name, &[kind], true, true, cx).await?,
            );
        }
    }
    // An explicit stopped example verifies the honest unknown state. This
    // changes the service lifecycle, never just the presentation's status text.
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "stopped capture graph",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    // The extra lifecycle example belongs to the full suite, not the quick
    // four-frame iteration path (even when the selected node is a meter).
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)]
        .into_iter()
        .filter(|_| filter.is_none())
    {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        let name = format!("nodes-{density}-inspector-meter-stopped");
        manifest.push(
            capture_group(
                window,
                shell,
                directory,
                &name,
                &[NodeKind::Meter],
                true,
                false,
                cx,
            )
            .await?,
        );
    }
    // Application and tone are Source templates, not additional NodeKinds.
    // Rebind through the service after Stop. In particular, never imply the
    // constant-source fake backend captures a real process's audio.
    let source = shell.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Source)
            .expect("capture source")
            .0
            .id()
    });
    for (variant, selection) in source_variants()
        .into_iter()
        .filter(|(variant, _)| filter.is_none_or(|selected| *variant == selected))
    {
        dispatch(
            shell,
            AppRequest::EditGraph(GraphEdit::BindSource {
                node: source,
                source: selection.clone(),
            }),
            cx,
        )?;
        wait_for(
            shell,
            "stopped source template binding",
            |_, snapshot| {
                snapshot.phase == SessionPhase::Idle
                    && snapshot
                        .graph
                        .node(source)
                        .and_then(|node| node.source.as_ref())
                        == Some(&selection)
            },
            cx,
        )
        .await?;
        for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
            window.update(cx, |_, window, _| {
                window.resize(size(px(width), px(height)))
            })?;
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            for inspector in [false, true] {
                let surface = if inspector { "inspector" } else { "card" };
                let name = format!("nodes-{density}-{surface}-{variant}-stopped");
                let mut frame = capture_group(
                    window,
                    shell,
                    directory,
                    &name,
                    &[NodeKind::Source],
                    inspector,
                    false,
                    cx,
                )
                .await?;
                if filter == Some("tone") {
                    frame["tone"] = tone_precision::closed_evidence(window, shell, source, cx)?;
                }
                manifest.push(frame);
            }
            if filter == Some("tone") {
                manifest.push(
                    tone_precision::capture_binding(window, shell, source, directory, density, cx)
                        .await?,
                );
            }
        }
    }
    let expected_frames = single_node_frame_count(filter);
    anyhow::ensure!(
        filter.is_none() || manifest.len() == expected_frames,
        "single-node capture must save exactly {expected_frames} frames"
    );
    let manifest_name = filter.map_or_else(
        || "node-workstations.json".to_owned(),
        |selected| format!("node-workstations-{selected}.json"),
    );
    std::fs::write(
        directory.join(manifest_name),
        serde_json::to_vec_pretty(&serde_json::json!({
            "filter": filter,
            "backend": "FakeBackend (real engine rendering and managed subscriptions)",
            "preferences": "isolated defaults; persistence disabled",
            "recording": "idle; no file selected or written",
            "source_variants": "Application and tone are service-rebound, stopped configurations; no live process capture is claimed",
            "captures": manifest,
        }))?,
    )?;
    Ok(())
}

/// One focused card per frame, not a whole-world fit. Warm full-size controls
/// first so semantic zoom reuses real heights instead of the 200px placeholder.
async fn capture_overviews(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let mut manifest = Vec::new();
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        for kind in [NodeKind::Gain, NodeKind::Eq] {
            let (ids, full) = prepare_group(window, shell, &[kind], false, true, cx).await?;
            shell.update(cx, |shell, cx| -> anyhow::Result<()> {
                let layout = overview_focus_layout(&shell.state.read(cx).snapshot, kind, &full)?;
                shell.node_positions = layout.positions;
                shell.camera = layout.camera;
                cx.notify();
                Ok(())
            })?;
            let mut previous = None;
            let mut settled = None;
            for _ in 0..8 {
                window.update(cx, |_, window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                })?;
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let measured = window.update(cx, |_, window, _| {
                    overview_measurements(window, ids[0], &full.heights)
                })??;
                if previous.as_ref() == Some(&measured) {
                    settled = Some(measured);
                    break;
                }
                previous = Some(measured);
            }
            let settled = settled.ok_or_else(|| anyhow::anyhow!("overview did not settle"))?;
            let name = format!("nodes-{density}-overview-{}", slug(kind));
            let path = directory.join(format!("{name}.png"));
            let frame = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
                window.refresh();
                window.draw(cx).clear(cx);
                let measured = overview_measurements(window, ids[0], &full.heights)?;
                anyhow::ensure!(measured == settled, "overview changed before saving {name}");
                let view = shell.read(cx);
                let snapshot = &view.state.read(cx).snapshot;
                anyhow::ensure!(
                    snapshot.phase == SessionPhase::Running
                        && view.node_meters.status(ids[0]) == MeterStatus::Live,
                    "overview lost its real live subscription"
                );
                let geometry = verify_overview_bounds(
                    snapshot, &view.node_positions, view.camera, ids[0], &measured,
                )?;
                window.render_to_image()?.save(&path)?;
                Ok(serde_json::json!({
                    "file": format!("{name}.png"),
                    "viewport": [f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)],
                    "canvas": [measured.native.viewport.0, measured.native.viewport.1],
                    "nodes": [slug(kind)],
                    "inspector": false,
                    "zoom": view.camera.zoom,
                    "phase": format!("{:?}", snapshot.phase),
                    "bounds_verified": true,
                    "measured_geometry": geometry,
                }))
            })??;
            manifest.push(frame);
            eprintln!("Capture {}: saved", path.display());
        }
    }
    anyhow::ensure!(manifest.len() == 4, "overview requires four focused frames");
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "stopped overview capture",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    std::fs::write(
        directory.join("node-workstations-overview.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "filter": "overview",
            "backend": "FakeBackend (real engine rendering and managed subscriptions)",
            "preferences": "isolated defaults; persistence disabled",
            "captures": manifest,
        }))?,
    )?;
    Ok(())
}

/// Recording is opt-in and writes only a new capture artifact. Armed is an
/// Idle service recorder with a destination, not an invented runtime phase.
async fn capture_recording_states(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let node = shell.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Recorder)
            .expect("capture recorder")
            .0
            .id()
    });
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = std::path::absolute(directory)?.join(format!(
        "recording-take-{}-{unique}.wav",
        std::process::id()
    ));
    anyhow::ensure!(!path.exists(), "capture WAV destination already exists");
    dispatch(
        shell,
        AppRequest::SetRecordingDestination {
            node,
            path: path.clone(),
        },
        cx,
    )?;
    wait_for(
        shell,
        "armed recording destination and preflight",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.recordings.iter().any(|recording| {
                    recording.node == node
                        && recording.phase == RecordingPhase::Idle
                        && recording.path.as_ref() == Some(&path)
                        && !recording.estimate.preflight_pending
                })
        },
        cx,
    )
    .await?;

    let mut manifest = capture_recording_stage(window, shell, directory, "armed", cx).await?;
    anyhow::ensure!(!path.exists(), "arming must not create the recording file");
    dispatch(shell, AppRequest::StartRecording { node }, cx)?;
    wait_for(
        shell,
        "recording real audio frames",
        |_, snapshot| {
            snapshot.recordings.iter().any(|recording| {
                recording.node == node
                    && recording.phase == RecordingPhase::Recording
                    && recording.written_frames > 0
            })
        },
        cx,
    )
    .await?;

    // Even a visual bounds failure must stop/finalize the owned recording.
    let recording_frames = capture_recording_stage(window, shell, directory, "recording", cx).await;
    dispatch(shell, AppRequest::StopRecording { node }, cx)?;
    wait_for(
        shell,
        "finalized recording with audio still running",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.recordings.iter().any(|recording| {
                    recording.node == node
                        && recording.phase == RecordingPhase::Finished
                        && recording.written_frames > 0
                })
        },
        cx,
    )
    .await?;
    manifest.extend(recording_frames?);
    let saved = shell.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .recordings
            .iter()
            .find(|recording| recording.node == node)
            .expect("saved recorder")
            .clone()
    });
    let wav_bytes = verify_recording_wav(&path, &saved)?;
    manifest.extend(capture_recording_stage(window, shell, directory, "saved", cx).await?);
    anyhow::ensure!(
        manifest.len() == 6,
        "recording lifecycle must save six card frames"
    );
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "stopped capture audio after saving",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    let manifest_path = directory.join("node-workstations-recording-states.json");
    let mut evidence = serde_json::json!({
        "filter": "recording-states",
        "backend": "FakeBackend (real engine audio and managed recording worker)",
        "preferences": "isolated defaults; persistence disabled",
        "recording": {
            "path": path,
            "bytes": wav_bytes,
            "written_frames": saved.written_frames,
            "sample_rate": saved.sample_rate,
            "channels": saved.channels,
            "wav_header_verified": true,
            "capture_wav_removed": false,
            "armed": "Idle with a configured destination; no file is opened before StartRecording",
        },
        "captures": manifest,
    });
    // Persist the real WAV evidence before deleting only this run's unique
    // artifact. On verification/write failure, retain it for diagnosis.
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&evidence)?)?;
    std::fs::remove_file(&path)?;
    anyhow::ensure!(!path.try_exists()?, "capture WAV cleanup failed");
    evidence["recording"]["capture_wav_removed"] = true.into();
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&evidence)?)?;
    eprintln!(
        "Recording artifact verified and removed: {} ({wav_bytes} bytes)",
        path.display()
    );
    Ok(())
}

async fn capture_recording_stage(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    stage: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let expected_phase = match stage {
        "armed" => "Idle",
        "recording" => "Recording",
        "saved" => "Finished",
        _ => anyhow::bail!("unknown recording capture stage"),
    };
    let mut manifest = Vec::new();
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        let name = format!("nodes-{density}-card-recording-{stage}");
        let mut frame = capture_group(
            window,
            shell,
            directory,
            &name,
            &[NodeKind::Recorder],
            false,
            true,
            cx,
        )
        .await?;
        let recording = &frame["recordings"][0];
        anyhow::ensure!(
            recording["phase"].as_str() == Some(expected_phase)
                && recording["path"].is_string()
                && recording["error"].is_null()
                && if stage == "armed" {
                    recording["written_frames"].as_u64() == Some(0)
                } else {
                    recording["written_frames"]
                        .as_u64()
                        .is_some_and(|frames| frames > 0)
                },
            "recording changed state while capturing {stage}"
        );
        frame["recording_stage"] = stage.into();
        manifest.push(frame);
    }
    Ok(manifest)
}

fn verify_recording_wav(path: &Path, saved: &RecordingSnapshot) -> anyhow::Result<u64> {
    use std::io::Read;
    anyhow::ensure!(
        saved.phase == RecordingPhase::Finished
            && saved.error.is_none()
            && saved.path.as_deref() == Some(path),
        "capture recording is not successfully finalized"
    );
    let mut file = std::fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    let mut header = [0u8; 58];
    file.read_exact(&mut header)?;
    let rate = saved
        .sample_rate
        .ok_or_else(|| anyhow::anyhow!("missing confirmed recording rate"))?;
    let channels = saved
        .channels
        .ok_or_else(|| anyhow::anyhow!("missing confirmed recording channels"))?;
    verify_wav_header(&header, bytes, saved.written_frames, rate, channels)?;
    Ok(bytes)
}

/// Match the engine's float32 WAVEFORMATEX/fact header, not a 44-byte PCM WAV.
fn verify_wav_header(
    header: &[u8; 58],
    bytes: u64,
    frames: u64,
    rate: u32,
    channels: usize,
) -> anyhow::Result<()> {
    let u16_at = |start: usize| {
        u16::from_le_bytes(header[start..start + 2].try_into().expect("header field"))
    };
    let u32_at = |start: usize| {
        u32::from_le_bytes(header[start..start + 4].try_into().expect("header field"))
    };
    let data_bytes = frames
        .checked_mul(channels as u64)
        .and_then(|samples| samples.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("recording size overflow"))?;
    anyhow::ensure!(
        frames > 0
            && rate > 0
            && channels > 0
            && data_bytes <= u64::from(u32::MAX) - 50
            && &header[..4] == b"RIFF"
            && &header[8..12] == b"WAVE"
            && &header[12..16] == b"fmt "
            && u32_at(16) == 18
            && u16_at(20) == 3
            && usize::from(u16_at(22)) == channels
            && u32_at(24) == rate
            && u16_at(34) == 32
            && &header[38..42] == b"fact"
            && u32_at(42) == 4
            && u64::from(u32_at(46)) == frames
            && &header[50..54] == b"data"
            && u64::from(u32_at(54)) == data_bytes
            && u64::from(u32_at(4)) + 8 == bytes
            && bytes == 58 + data_bytes,
        "saved WAV header does not match the service's confirmed recording frames/format"
    );
    Ok(())
}

fn source_variants() -> [(&'static str, InputSelection); 2] {
    [
        (
            "application",
            InputSelection::Process {
                pid: PROCESS_PID,
                creation_time_100ns: PROCESS_CREATED,
            },
        ),
        ("tone", InputSelection::Tone { frequency_hz: 440. }),
    ]
}

fn single_node_frame_count(filter: Option<&str>) -> usize {
    if matches!(filter, Some("meter32" | "input-long" | "tone")) {
        6
    } else {
        4
    }
}

fn populated_rack() -> RackSettings {
    RackSettings {
        slots: [
            RackProcessor::Gain { initial: 0.75 },
            RackProcessor::Eq(Default::default()),
            RackProcessor::Compressor(Default::default()),
            RackProcessor::Limiter(Default::default()),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, processor)| RackSlot {
            id: index as u32 + 1,
            bypass: false,
            processor,
        })
        .collect(),
        routes: Vec::new(),
    }
}

/// These are real graph transactions before audio starts, not mutated UI
/// snapshots. Start must subsequently confirm the expanded execution plan.
fn expansion_edits(graph: &GraphSnapshot, filter: &str) -> anyhow::Result<Vec<GraphEdit>> {
    match filter {
        "rack-populated" => {
            let node = graph
                .node_of_kind(NodeKind::Rack)
                .ok_or_else(|| anyhow::anyhow!("capture rack is missing"))?
                .0
                .id();
            Ok(vec![GraphEdit::SetBuiltin {
                node,
                state: BuiltinState::Rack(populated_rack()),
            }])
        }
        "bus-expanded" => {
            let node = graph
                .node_of_kind(NodeKind::Bus)
                .ok_or_else(|| anyhow::anyhow!("capture bus is missing"))?
                .0;
            Ok((node.inputs().len()..6)
                .map(|_| GraphEdit::AddBusInput(node.id()))
                .collect())
        }
        _ => Ok(Vec::new()),
    }
}

fn filtered_kind(filter: &str) -> Option<NodeKind> {
    match filter {
        "rack-populated" => Some(NodeKind::Rack),
        "bus-expanded" => Some(NodeKind::Bus),
        "meter32" => Some(NodeKind::Meter),
        "input-long" => Some(NodeKind::Source),
        _ => KINDS.into_iter().find(|kind| slug(*kind) == filter),
    }
}

/// Validate before starting a service or opening a window. A misspelled filter
/// must not silently fall back to the expensive full capture suite.
fn capture_filter(requested: Option<&str>) -> anyhow::Result<Option<&'static str>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let slugs = KINDS
        .map(slug)
        .into_iter()
        .chain(source_variants().map(|(variant, _)| variant))
        .chain(SCENARIOS)
        .collect::<Vec<_>>();
    slugs
        .iter()
        .copied()
        .find(|slug| *slug == requested.trim())
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid {NODE_FILTER}: {requested:?}; use {}",
                slugs.join(", ")
            )
        })
}

fn slug(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Source => "input",
        NodeKind::Sink => "output",
        NodeKind::Gain => "gain",
        NodeKind::Pan => "balance",
        NodeKind::Eq => "eq",
        NodeKind::Gate => "gate",
        NodeKind::Compressor => "compressor",
        NodeKind::Limiter => "limiter",
        NodeKind::Delay => "delay",
        NodeKind::ChannelMapper => "mapper",
        NodeKind::Bus => "bus",
        NodeKind::Rack => "rack",
        NodeKind::Meter => "meter",
        NodeKind::SpectrumAnalyzer => "spectrum",
        NodeKind::Recorder => "recorder",
    }
}

struct FocusLayout {
    positions: HashMap<NodeId, (f32, f32)>,
    camera: Camera,
    ids: Vec<NodeId>,
}

/// Preserve order and 1:1 geometry; only group membership may shrink when a
/// workstation's intrinsic width grows. The same strict layout checks apply.
fn capture_groups<'a>(
    snapshot: &AppSnapshot,
    kinds: &'a [NodeKind],
    viewport: (f32, f32),
    maximum: usize,
) -> anyhow::Result<Vec<&'a [NodeKind]>> {
    anyhow::ensure!(maximum > 0, "capture group limit must be positive");
    let world = CanvasWorld::positioned(snapshot, &HashMap::new());
    let mut groups = Vec::new();
    let mut start = 0;
    while start < kinds.len() {
        let mut end = start + 1;
        focus_world_layout(&world, &kinds[start..end], viewport)?;
        while end < kinds.len()
            && end - start < maximum
            && focus_world_layout(&world, &kinds[start..=end], viewport).is_ok()
        {
            end += 1;
        }
        groups.push(&kinds[start..end]);
        start = end;
    }
    Ok(groups)
}

/// Bootstrap visibility only: positioned nodes have placeholder heights until
/// their native controls have painted. Never save this provisional layout.
fn focus_layout(
    snapshot: &AppSnapshot,
    kinds: &[NodeKind],
    viewport: (f32, f32),
) -> anyhow::Result<FocusLayout> {
    let world = CanvasWorld::positioned(snapshot, &HashMap::new());
    focus_world_layout(&world, kinds, viewport)
}

fn measured_focus_layout(
    snapshot: &AppSnapshot,
    kinds: &[NodeKind],
    viewport: (f32, f32),
    heights: &BTreeMap<NodeId, f32>,
) -> anyhow::Result<FocusLayout> {
    let world = CanvasWorld::measured(snapshot, &HashMap::new(), heights);
    for kind in kinds {
        let node = world.nodes.iter().find(|node| node.kind == *kind);
        anyhow::ensure!(
            node.and_then(|node| node.measured_height)
                .is_some_and(|height| height.is_finite() && height > 0.),
            "missing or invalid native height for {}",
            slug(*kind),
        );
    }
    focus_world_layout(&world, kinds, viewport)
}

fn overview_focus_layout(
    snapshot: &AppSnapshot,
    kind: NodeKind,
    full: &NativeMeasurements,
) -> anyhow::Result<FocusLayout> {
    let mut layout = measured_focus_layout(snapshot, &[kind], full.viewport, &full.heights)?;
    let world = CanvasWorld::measured(snapshot, &layout.positions, &full.heights);
    let node = world
        .nodes
        .iter()
        .find(|node| node.node_id == Some(layout.ids[0]))
        .expect("measured overview node");
    layout.camera = Camera {
        pan_x: (full.viewport.0 - node.width() * OVERVIEW_ZOOM) / 2.,
        pan_y: (full.viewport.1 - node.height() * OVERVIEW_ZOOM) / 2.,
        zoom: OVERVIEW_ZOOM,
        adjusted: true,
    };
    Ok(layout)
}

/// Calculate against the actual canvas, after panel springs have settled.
/// Insufficient room is a capture failure, never permission to reduce zoom.
fn focus_world_layout(
    world: &CanvasWorld,
    kinds: &[NodeKind],
    viewport: (f32, f32),
) -> anyhow::Result<FocusLayout> {
    anyhow::ensure!(!kinds.is_empty(), "capture group must contain a node");
    anyhow::ensure!(
        viewport.0.is_finite() && viewport.1.is_finite(),
        "capture canvas dimensions must be finite"
    );
    let focused = kinds
        .iter()
        .map(|kind| {
            world
                .nodes
                .iter()
                .find(|node| node.kind == *kind)
                .ok_or_else(|| anyhow::anyhow!("missing capture node: {kind:?}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let gap = 36.;
    let width = focused.iter().map(|node| node.width()).sum::<f32>()
        + focused.len().saturating_sub(1) as f32 * gap;
    let height = focused.iter().map(|node| node.height()).fold(0., f32::max);
    let margin = 24.;
    // The toolbar ends at y=48. Reduce only the top gutter for taller bodies;
    // retain clearance at both ends of the actually measured card.
    let top = (viewport.1 - height - 12.).min(72.);
    anyhow::ensure!(
        width + 2. * margin <= viewport.0 && top >= 56.,
        "capture group ({width} x {height}) does not fit canvas ({} x {}) at 100% zoom",
        viewport.0,
        viewport.1,
    );
    let ids = focused
        .iter()
        .map(|node| node.node_id.expect("service node"))
        .collect::<Vec<_>>();
    let first = KINDS
        .iter()
        .position(|kind| *kind == kinds[0])
        .expect("known kind");
    let mut positions = HashMap::new();
    for node in &world.nodes {
        let index = KINDS
            .iter()
            .position(|kind| *kind == node.kind)
            .expect("known kind");
        let x = if index < first {
            -10_000. - (KINDS.len() - index) as f32 * 500.
        } else {
            10_000. + index as f32 * 500.
        };
        positions.insert(node.node_id.expect("service node"), (x, 0.));
    }
    let mut x = 0.;
    for node in focused {
        positions.insert(node.node_id.expect("service node"), (x, 0.));
        x += node.width() + gap;
    }
    Ok(FocusLayout {
        positions,
        camera: Camera {
            pan_x: (viewport.0 - width) / 2.,
            pan_y: top,
            zoom: 1.,
            adjusted: true,
        },
        ids,
    })
}

#[derive(Clone, PartialEq)]
struct NativeMeasurements {
    viewport: (f32, f32),
    heights: BTreeMap<NodeId, f32>,
    bodies: BTreeMap<NodeId, Bounds<f32>>,
    meters: BTreeMap<NodeId, Bounds<f32>>,
}

/// Reconstruct full heights only at 1:1. Compact overlays have a fixed clipped
/// viewport and must never overwrite measurements of the expanded content.
fn native_measurements(window: &Window, ids: &[NodeId]) -> anyhow::Result<NativeMeasurements> {
    let mut measured = observed_native_geometry(window, ids)?;
    measured.heights = measured
        .bodies
        .iter()
        .map(|(id, body)| (*id, (body.size.height + 68.).ceil()))
        .collect();
    Ok(measured)
}

/// Observe the same native body that populates graph-private heights without
/// widening production APIs. Geometry here is canvas-local at the current zoom.
fn observed_native_geometry(window: &Window, ids: &[NodeId]) -> anyhow::Result<NativeMeasurements> {
    let canvas = window
        .try_find("graph-canvas-input")
        .ok_or_else(|| anyhow::anyhow!("capture canvas has not painted"))?;
    let bounds = canvas.bounds();
    anyhow::ensure!(
        canvas.visible()
            && bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= window.viewport_size().width
            && bounds.bottom() <= window.viewport_size().height,
        "capture canvas is clipped by the native window"
    );
    let mut measurements = NativeMeasurements {
        viewport: (f32::from(bounds.size.width), f32::from(bounds.size.height)),
        heights: BTreeMap::new(),
        bodies: BTreeMap::new(),
        meters: BTreeMap::new(),
    };
    for id in ids {
        let body = window
            .try_find(format!("node-controls-{id:?}"))
            .ok_or_else(|| anyhow::anyhow!("focused native controls have not painted"))?;
        let body_bounds = body.bounds();
        let width = f32::from(body_bounds.size.width);
        let height = f32::from(body_bounds.size.height);
        anyhow::ensure!(
            body.visible() && width.is_finite() && height.is_finite() && width > 0. && height > 0.,
            "focused native controls have invalid bounds"
        );
        measurements.bodies.insert(
            *id,
            Bounds {
                origin: point(
                    f32::from(body_bounds.left() - bounds.left()),
                    f32::from(body_bounds.top() - bounds.top()),
                ),
                size: size(width, height),
            },
        );
        if let Some(meter) = window.try_find(format!("card-meter-{id:?}")) {
            let meter = meter.bounds();
            measurements.meters.insert(
                *id,
                Bounds {
                    origin: point(
                        f32::from(meter.left() - bounds.left()),
                        f32::from(meter.top() - bounds.top()),
                    ),
                    size: size(f32::from(meter.size.width), f32::from(meter.size.height)),
                },
            );
        }
    }
    Ok(measurements)
}

#[derive(Clone, PartialEq)]
struct OverviewMeasurements {
    native: NativeMeasurements,
    focus: Bounds<f32>,
}

fn overview_measurements(
    window: &Window,
    id: NodeId,
    full_heights: &BTreeMap<NodeId, f32>,
) -> anyhow::Result<OverviewMeasurements> {
    let mut native = observed_native_geometry(window, &[id])?;
    native.heights = full_heights.clone();
    let canvas = window.find("graph-canvas-input").bounds();
    let focus = window
        .try_find(format!("card-focus-{id:?}"))
        .ok_or_else(|| anyhow::anyhow!("overview focus action has not painted"))?;
    anyhow::ensure!(focus.visible(), "overview focus action is not visible");
    let bounds = focus.bounds();
    Ok(OverviewMeasurements {
        native,
        focus: Bounds {
            origin: point(
                f32::from(bounds.left() - canvas.left()),
                f32::from(bounds.top() - canvas.top()),
            ),
            size: size(f32::from(bounds.size.width), f32::from(bounds.size.height)),
        },
    })
}

fn contains_native_bounds(outer: Bounds<f32>, inner: Bounds<f32>) -> bool {
    inner.size.width > 0.
        && inner.size.height > 0.
        && inner.left() >= outer.left()
        && inner.right() <= outer.right()
        && inner.top() >= outer.top()
        && inner.bottom() <= outer.bottom()
}

fn pixel_bounds(bounds: Bounds<Pixels>) -> Bounds<f32> {
    Bounds {
        origin: point(f32::from(bounds.left()), f32::from(bounds.top())),
        size: size(f32::from(bounds.size.width), f32::from(bounds.size.height)),
    }
}

fn meter_scroll_geometry(
    window: &mut Window,
    id: NodeId,
) -> anyhow::Result<(Bounds<f32>, Bounds<f32>)> {
    let scope = window.within(format!("card-meter-{id:?}"));
    let viewport = scope
        .try_find("node-meter-viewport")
        .ok_or_else(|| anyhow::anyhow!("meter scroll viewport is not observed"))?;
    // The pinned Kit snapshot exposes bounds, not scroll_offset(). Observe the
    // natural inner canvas as well, deriving offset from its actual translation.
    let content = scope.try_find("node-meter-content").ok_or_else(|| {
        anyhow::anyhow!(
            "observe natural meter inner div as node-meter-content to verify channel 32"
        )
    })?;
    anyhow::ensure!(
        viewport.visible() && content.visible(),
        "meter scroll surface is not visible"
    );
    Ok((
        pixel_bounds(viewport.bounds()),
        pixel_bounds(content.bounds()),
    ))
}

fn verify_meter_scroll_end(
    viewport: Bounds<f32>,
    before: Bounds<f32>,
    after: Bounds<f32>,
) -> anyhow::Result<Bounds<f32>> {
    // Every channel retains its 38px numeric cell; this verifies the entire
    // final cell, including label, readout and clip, rather than partial visibility.
    anyhow::ensure!(
        before.size == after.size && after.size.width >= 32. * 38.,
        "meter natural content must retain all 32 readable channel cells"
    );
    anyhow::ensure!(
        (before.left() - viewport.left()).abs() <= 1.
            && after.left() < before.left() - 1.
            && (after.right() - viewport.right()).abs() <= 1.,
        "meter did not scroll from its start to its last channel"
    );
    let last = Bounds {
        origin: point(after.right() - 38., after.top()),
        size: size(38., after.size.height),
    };
    anyhow::ensure!(
        contains_native_bounds(viewport, last),
        "channel 32 is clipped by the meter viewport"
    );
    Ok(last)
}

async fn capture_meter_end(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    name: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    let (id, camera) = shell.update(cx, |shell, cx| {
        (
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node_of_kind(NodeKind::Meter)
                .expect("capture meter")
                .0
                .id(),
            shell.camera,
        )
    });
    let (native, viewport, before) =
        window.update(cx, |_, window, cx| -> anyhow::Result<_> {
            window.render_frame(cx);
            let native = native_measurements(window, &[id])?;
            let (viewport, before) = meter_scroll_geometry(window, id)?;
            window.within(format!("card-meter-{id:?}")).scroll(
                "node-meter-viewport",
                gpui_kit::ScrollDelta::Pixels(point(px(-100_000.), px(0.))),
                cx,
            );
            Ok((native, viewport, before))
        })??;
    cx.background_executor()
        .timer(Duration::from_millis(80))
        .await;
    let path = directory.join(format!("{name}.png"));
    let frame = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        window.render_frame(cx);
        let (end_viewport, after) = meter_scroll_geometry(window, id)?;
        anyhow::ensure!(end_viewport == viewport, "scroll changed the meter viewport");
        let last = verify_meter_scroll_end(viewport, before, after)?;
        // Another wheel at the boundary must not leak to graph pan/zoom.
        window.within(format!("card-meter-{id:?}")).scroll("node-meter-viewport",
            gpui_kit::ScrollDelta::Pixels(point(px(-100_000.), px(0.))), cx);
        let (edge_viewport, edge_content) = meter_scroll_geometry(window, id)?;
        anyhow::ensure!(edge_viewport == viewport && edge_content == after, "meter did not remain at its scroll limit");
        anyhow::ensure!(native_measurements(window, &[id])? == native, "scroll reflowed the card");
        let view = shell.read(cx);
        anyhow::ensure!(view.camera == camera, "meter wheel changed graph camera");
        let snapshot = &view.state.read(cx).snapshot;
        anyhow::ensure!(snapshot.phase == SessionPhase::Running
            && view.node_meters.status(id) == MeterStatus::Live
            && view.node_meters.channel_levels(id).is_some_and(|levels| levels.len() == 32),
            "last-channel capture lost real 32-channel readings");
        let geometry = verify_native_bounds(snapshot, &view.node_positions, view.camera, &[id], &native)?;
        window.render_to_image()?.save(&path)?;
        Ok(serde_json::json!({
            "file": format!("{name}.png"), "nodes": ["meter"], "channels": 32,
            "viewport": [f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)],
            "inspector": view.inspector_open, "zoom": view.camera.zoom,
            "camera_unchanged": true, "bounds_verified": true, "measured_geometry": geometry,
            "scroll_offset": [after.left() - viewport.left(), after.top() - viewport.top()],
            "scroll_evidence": "observed natural content relative to viewport; stable at native wheel limit",
            "natural_content_width": after.size.width, "meter_viewport_width": viewport.size.width,
            "last_channel": 32, "last_channel_fully_visible": true,
            "last_channel_cell": [last.left(), last.top(), last.size.width, last.size.height],
            "phase": format!("{:?}", snapshot.phase),
        }))
    })??;
    eprintln!("Capture {}: saved", path.display());
    Ok(frame)
}

async fn capture_input_menu(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    name: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    let (id, camera) = shell.update(cx, |shell, cx| {
        (
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node_of_kind(NodeKind::Source)
                .expect("capture input")
                .0
                .id(),
            shell.camera,
        )
    });
    window.update(cx, |_, window, cx| {
        window.click(format!("card-device-{id:?}"), cx)
    })?;
    cx.background_executor()
        .timer(Duration::from_millis(150))
        .await;
    let path = directory.join(format!("{name}.png"));
    let frame = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        window.render_frame(cx);
        let trigger = window.find(format!("card-device-{id:?}"));
        let choice = window.try_find(format!("card-resource-{id:?}-0"))
            .ok_or_else(|| anyhow::anyhow!("long input menu did not open"))?;
        let bounds = Bounds { origin: point(0., 0.), size: size(f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)) };
        let choice_bounds = pixel_bounds(choice.bounds());
        anyhow::ensure!(choice.visible() && contains_native_bounds(bounds, choice_bounds)
            && choice_bounds.size.width <= 301., "long input menu item overflowed its 300px viewport");
        let native = native_measurements(window, &[id])?;
        let canvas = window.find("graph-canvas-input").bounds();
        let mut trigger_bounds = pixel_bounds(trigger.bounds());
        trigger_bounds.origin.x -= f32::from(canvas.left());
        trigger_bounds.origin.y -= f32::from(canvas.top());
        anyhow::ensure!(trigger.visible() && contains_native_bounds(native.bodies[&id], trigger_bounds), "long selected input overflowed the card");
        let view = shell.read(cx);
        let snapshot = &view.state.read(cx).snapshot;
        anyhow::ensure!(view.camera == camera && snapshot.catalog.inputs.iter().any(|row| row.endpoint_id == INPUT && row.name == LONG_INPUT_NAME), "long-name capture must use the unchanged service catalog and camera");
        let geometry = verify_native_bounds(snapshot, &view.node_positions, view.camera, &[id], &native)?;
        window.render_to_image()?.save(&path)?;
        Ok(serde_json::json!({
            "file": format!("{name}.png"), "nodes": ["input"], "resource_menu": true,
            "viewport": [f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)],
            "catalog_input_name": LONG_INPUT_NAME, "camera_unchanged": true,
            "bounds_verified": true, "measured_geometry": geometry,
            "ellipsis": "production truncate_label; inspect screenshot glyphs",
        }))
    })??;
    window.update(cx, |_, window, cx| {
        window.click(format!("card-device-{id:?}"), cx)
    })?;
    eprintln!("Capture {}: saved", path.display());
    Ok(frame)
}

/// Semantic zoom intentionally crops the *full editor*, never its replacement
/// focus action. Keep the normal 100% validator strict and separate.
fn verify_overview_bounds(
    snapshot: &AppSnapshot,
    positions: &HashMap<NodeId, (f32, f32)>,
    camera: Camera,
    id: NodeId,
    measured: &OverviewMeasurements,
) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(
        camera.zoom == OVERVIEW_ZOOM && camera.adjusted,
        "overview requires explicit 0.5 zoom"
    );
    let native = &measured.native;
    anyhow::ensure!(
        native
            .heights
            .get(&id)
            .is_some_and(|h| h.is_finite() && *h > 0.),
        "overview needs a full-size measured height"
    );
    let world = CanvasWorld::measured(snapshot, positions, &native.heights);
    let node = world
        .nodes
        .iter()
        .find(|node| node.node_id == Some(id))
        .ok_or_else(|| anyhow::anyhow!("overview node disappeared"))?;
    let card = NodeLayout::new(node).project(camera).card;
    anyhow::ensure!(
        card.left() >= 24.
            && card.right() <= native.viewport.0 - 24.
            && card.top() >= 56.
            && card.bottom() <= native.viewport.1 - 12.,
        "overview card is clipped"
    );
    let body = *native
        .bodies
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("missing overview body"))?;
    anyhow::ensure!(
        contains_native_bounds(card, body)
            && (body.left() - card.left() - 10. * OVERVIEW_ZOOM).abs() <= 1.
            && (body.top() - card.top() - 58. * OVERVIEW_ZOOM).abs() <= 1.
            && (body.size.width - (card.size.width - 20. * OVERVIEW_ZOOM)).abs() <= 1.
            && (body.size.height - (card.size.height - 58. * OVERVIEW_ZOOM - 10.).max(1.)).abs()
                <= 1.,
        "overview body does not match measured semantic-zoom geometry"
    );
    anyhow::ensure!(
        contains_native_bounds(body, measured.focus),
        "overview Edit node action is clipped"
    );
    if let Some(meter) = native.meters.get(&id) {
        anyhow::ensure!(
            contains_native_bounds(body, *meter),
            "overview meter is clipped"
        );
    }
    Ok(serde_json::json!({
        "node": slug(node.kind),
        "full_measured_height": native.heights[&id],
        "card": [card.left(), card.top(), card.size.width, card.size.height],
        "native_body": [body.left(), body.top(), body.size.width, body.size.height],
        "focus_action": [measured.focus.left(), measured.focus.top(), measured.focus.size.width, measured.focus.size.height],
        "focus_action_fully_visible": true,
        "meter_rendered": native.meters.contains_key(&id),
    }))
}

fn verify_native_bounds(
    snapshot: &AppSnapshot,
    positions: &HashMap<NodeId, (f32, f32)>,
    camera: Camera,
    ids: &[NodeId],
    measured: &NativeMeasurements,
) -> anyhow::Result<Vec<serde_json::Value>> {
    anyhow::ensure!(
        camera.zoom == 1. && camera.adjusted,
        "node capture requires an explicit 100% camera"
    );
    let world = CanvasWorld::measured(snapshot, positions, &measured.heights);
    let mut geometry = Vec::new();
    for id in ids {
        let node = world
            .nodes
            .iter()
            .find(|node| node.node_id == Some(*id))
            .ok_or_else(|| anyhow::anyhow!("focused node disappeared before capture"))?;
        let height = measured.heights.get(id).copied();
        anyhow::ensure!(
            height.is_some_and(|height| height.is_finite() && height > 0.),
            "missing native height for {}",
            slug(node.kind)
        );
        let card = NodeLayout::new(node).project(camera).card;
        anyhow::ensure!(
            card.left() >= 24.
                && card.right() <= measured.viewport.0 - 24.
                && card.top() >= 56.
                && card.bottom() <= measured.viewport.1 - 12.,
            "{} measured card is clipped at 100%: {card:?}, canvas {:?}",
            slug(node.kind),
            measured.viewport
        );
        let body = measured
            .bodies
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("missing native body for {}", slug(node.kind)))?;
        anyhow::ensure!(
            body.left() >= card.left()
                && body.right() <= card.right()
                && body.top() >= card.top()
                && body.bottom() <= card.bottom()
                && (body.top() - card.top() - 58.).abs() <= 1.,
            "{} native body is outside its measured card: {body:?}, card {card:?}",
            slug(node.kind)
        );
        let rack_stages = snapshot
            .graph
            .node(*id)
            .and_then(|node| node.builtin.rack())
            .map(|rack| rack.slots.len());
        let vertical_meter = matches!(
            node.kind,
            NodeKind::Bus | NodeKind::Recorder | NodeKind::SpectrumAnalyzer
        ) || (node.kind == NodeKind::Rack
            && rack_stages.is_some_and(|stages| stages > 0));
        let meter = measured.meters.get(id);
        let channels = snapshot
            .graph
            .graph
            .get_node(*id)
            .map_err(|error| anyhow::anyhow!("capture node: {error:?}"))?
            .channels();
        if node.kind == NodeKind::Meter && channels == 32 {
            let meter =
                meter.ok_or_else(|| anyhow::anyhow!("32-channel meter viewport did not paint"))?;
            anyhow::ensure!(
                contains_native_bounds(*body, *meter)
                    && meter.size.width <= crate::ui::widgets::meters::node_meter_width(8) + 1.,
                "32-channel meter viewport exceeds the eight-channel cap or its body"
            );
        }
        if vertical_meter {
            let meter = meter.ok_or_else(|| {
                anyhow::anyhow!("{} vertical meter did not paint", slug(node.kind))
            })?;
            anyhow::ensure!(
                meter.left() >= body.left()
                    && meter.right() <= body.right()
                    && meter.size.width > 0.
                    && meter.size.height > 0.
                    && (meter.top() - body.top()).abs() <= 1.
                    && (meter.bottom() - body.bottom()).abs() <= 1.,
                "{} vertical meter does not follow its expanded body: {meter:?}, body {body:?}",
                slug(node.kind)
            );
        }
        geometry.push(serde_json::json!({
            "node": slug(node.kind),
            "card": [card.left(), card.top(), card.size.width, card.size.height],
            "native_body": [body.left(), body.top(), body.size.width, body.size.height],
            "meter": meter.map(|meter| [meter.left(), meter.top(), meter.size.width, meter.size.height]),
            "vertical_meter_verified": vertical_meter,
            "input_streams": node.input_count,
            "rack_stages": rack_stages,
            "channels": channels,
            "meter_viewport_channel_cap": if node.kind == NodeKind::Meter && channels == 32 { Some(8) } else { None },
        }));
    }
    Ok(geometry)
}

/// Measurement is deferred by the native body renderer. Yield between draws,
/// recenter with measured geometry, then require a second unchanged frame.
async fn settle_measured_focus(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    kinds: &[NodeKind],
    ids: &[NodeId],
    cx: &mut AsyncApp,
) -> anyhow::Result<NativeMeasurements> {
    let mut previous: Option<NativeMeasurements> = None;
    for _ in 0..8 {
        window.update(cx, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(40))
            .await;
        let measured = window.update(cx, |_, window, _| native_measurements(window, ids))??;
        let ready = shell.update(cx, |shell, cx| -> anyhow::Result<bool> {
            anyhow::ensure!(
                shell.camera.zoom == 1. && shell.camera.adjusted,
                "native measurement requires an explicit 100% camera"
            );
            let snapshot = &shell.state.read(cx).snapshot;
            let layout =
                measured_focus_layout(snapshot, kinds, measured.viewport, &measured.heights)?;
            if previous.as_ref() == Some(&measured)
                && shell.camera == layout.camera
                && shell.node_positions == layout.positions
            {
                verify_native_bounds(
                    snapshot,
                    &shell.node_positions,
                    shell.camera,
                    ids,
                    &measured,
                )?;
                return Ok(true);
            }
            shell.node_positions = layout.positions;
            shell.camera = layout.camera;
            cx.notify();
            Ok(false)
        })?;
        if ready {
            return Ok(measured);
        }
        previous = Some(measured);
    }
    anyhow::bail!("focused native card measurements did not settle at 100% zoom")
}

async fn prepare_group(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    kinds: &[NodeKind],
    inspector: bool,
    running: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<(Vec<NodeId>, NativeMeasurements)> {
    shell.update(cx, |shell, cx| {
        shell.set_page(Page::Graph, cx);
        shell.sidebar_open = false;
        shell.inspector_open = inspector;
        shell.density = if shell.compact {
            Density::Compact
        } else {
            Density::Comfortable
        };
        shell.selected_nodes.clear();
        shell.selected_route = None;
        shell.selected_node = None;
        cx.notify();
    });
    cx.background_executor()
        .timer(Duration::from_millis(900))
        .await;
    window.update(cx, |_, window, cx| window.draw(cx).clear(cx))?;
    let ids = shell.update(cx, |shell, cx| -> anyhow::Result<Vec<NodeId>> {
        let viewport = shell
            .canvas_size
            .ok_or_else(|| anyhow::anyhow!("canvas has not laid out"))?;
        let FocusLayout {
            positions,
            camera,
            ids,
        } = focus_layout(&shell.state.read(cx).snapshot, kinds, viewport)?;
        shell.node_positions = positions;
        shell.camera = camera;
        shell.selected_node = inspector.then_some(ids[0]);
        shell.selected_nodes.extend(shell.selected_node);
        cx.notify();
        Ok(ids)
    })?;
    // A draw registers visibility; only the production meter pump subscribes.
    window.update(cx, |_, window, cx| window.draw(cx).clear(cx))?;
    settle_measured_focus(window, shell, kinds, &ids, cx).await?;
    if running {
        wait_for(
            shell,
            "fresh focused-node meter subscriptions",
            |shell, snapshot| {
                snapshot.phase == SessionPhase::Running
                    && ids.iter().all(|id| {
                        snapshot.signal_topics.node_levels.contains_key(id)
                            && shell.node_meters.status(*id) == MeterStatus::Live
                            && snapshot.graph.graph.get_node(*id).is_ok_and(|node| {
                                shell
                                    .node_meters
                                    .channel_levels(*id)
                                    .is_some_and(|levels| levels.len() == node.channels())
                            })
                    })
            },
            cx,
        )
        .await?;
        // The constant-source backend reaches stable display ballistics before saving.
        cx.background_executor()
            .timer(Duration::from_millis(800))
            .await;
    } else {
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
    // Live status and the inspector may reflow content after subscription.
    if kinds == [NodeKind::Meter]
        && shell.update(cx, |shell, cx| {
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .get_node(ids[0])
                .is_ok_and(|node| node.channels() == 32)
        })
    {
        let camera = shell.update(cx, |shell, _| shell.camera);
        window.update(cx, |_, window, cx| {
            window.within(format!("card-meter-{:?}", ids[0])).scroll(
                "node-meter-viewport",
                gpui_kit::ScrollDelta::Pixels(point(px(100_000.), px(0.))),
                cx,
            );
        })?;
        anyhow::ensure!(
            shell.update(cx, |shell, _| shell.camera) == camera,
            "resetting the meter scroll changed graph camera"
        );
    }
    let settled = settle_measured_focus(window, shell, kinds, &ids, cx).await?;
    Ok((ids, settled))
}

#[allow(clippy::too_many_arguments)]
async fn capture_group(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    name: &str,
    kinds: &[NodeKind],
    inspector: bool,
    running: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    // A preceding native click can leave its tooltip over the next frame.
    // Move to noninteractive titlebar space before settling and capturing.
    window.update(cx, |_, window, cx| {
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: point(px(2.), px(2.)),
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    })?;
    let (ids, settled) = prepare_group(window, shell, kinds, inspector, running, cx).await?;
    let path = directory.join(format!("{name}.png"));
    let (dimensions, geometry, recordings) =
        window.update(cx, |_, window, cx| -> anyhow::Result<_> {
            window.refresh();
            window.draw(cx).clear(cx);
            let measured = native_measurements(window, &ids)?;
            anyhow::ensure!(
                measured == settled,
                "native layout changed before saving {name}"
            );
            let view = shell.read(cx);
            let geometry = verify_native_bounds(
                &view.state.read(cx).snapshot,
                &view.node_positions,
                view.camera,
                &ids,
                &measured,
            )?;
            let recordings = view
                .state
                .read(cx)
                .snapshot
                .recordings
                .iter()
                .filter(|recording| ids.contains(&recording.node))
                .map(|recording| {
                    serde_json::json!({
                        "phase": format!("{:?}", recording.phase),
                        "path": recording.path,
                        "format": recording.format.label(),
                        "sample_rate": recording.sample_rate,
                        "channels": recording.channels,
                        "written_frames": recording.written_frames,
                        "error": recording.error,
                    })
                })
                .collect::<Vec<_>>();
            window.render_to_image()?.save(&path)?;
            Ok((
                (
                    f32::from(window.viewport_size().width),
                    f32::from(window.viewport_size().height),
                ),
                geometry,
                recordings,
            ))
        })??;
    eprintln!("Capture {}: saved", path.display());
    shell.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        let live = ids
            .iter()
            .filter(|id| shell.node_meters.status(**id) == MeterStatus::Live)
            .count();
        anyhow::ensure!(
            if running {
                snapshot.phase == SessionPhase::Running && live == ids.len()
            } else {
                snapshot.phase == SessionPhase::Idle && live == 0
            },
            "capture lifecycle changed while saving {name}"
        );
        let channel_levels = ids
            .iter()
            .map(|id| {
                let node = snapshot.graph.graph.get_node(*id).expect("capture node");
                let levels = shell.node_meters.channel_levels(*id);
                let clips = shell.node_meters.channel_clips(*id);
                if running && node.kind() == NodeKind::Meter && node.channels() == 32 {
                    anyhow::ensure!(
                        levels.is_some_and(|levels| levels.len() == 32)
                            && clips.len() == 32
                            && clips.iter().all(|clip| *clip),
                        "32-channel capture requires 32 real live readings and clip latches"
                    );
                }
                Ok(serde_json::json!({
                    "node": slug(node.kind()),
                    "channels": node.channels(),
                    "live_channels": levels.map_or(0, |levels| levels.len()),
                    "clips": clips,
                }))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(serde_json::json!({
            "file": format!("{name}.png"),
            "viewport": [dimensions.0, dimensions.1],
            "nodes": kinds.iter().map(|kind| slug(*kind)).collect::<Vec<_>>(),
            "inspector": inspector,
            "zoom": shell.camera.zoom,
            "canvas": [settled.viewport.0, settled.viewport.1],
            "bounds_verified": true,
            "measured_geometry": geometry,
            "phase": format!("{:?}", snapshot.phase),
            "live_levels": live,
            "channel_levels": channel_levels,
            "recordings": recordings,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureCatalog, FocusLayout, KINDS, LONG_INPUT_NAME, NativeMeasurements,
        OverviewMeasurements, SCENARIOS, capture_filter, capture_groups, chain_fixture,
        expansion_edits, filtered_kind, fixture, focus_layout, measured_focus_layout,
        overview_focus_layout, populated_rack, slug, source_variants, verify_meter_scroll_end,
        verify_native_bounds, verify_overview_bounds, verify_wav_header,
    };
    use crate::service::{AppSnapshot, CatalogBackend, ServiceGraph};
    use crate::ui::pages::graph_canvas::{CanvasWorld, NodeLayout};
    use crate::ui::window::test_harness::{finish_service, initial_projection, spawn_service};
    use gpui_kit::{Bounds, point, size};
    use moiren_core::graph::{NodeId, NodeKind};
    use std::collections::BTreeMap;
    use std::io::Cursor;

    #[::core::prelude::v1::test]
    fn long_name_catalog_changes_only_fixture_display_names() {
        let ordinary = CaptureCatalog::default().catalog().unwrap();
        let long = CaptureCatalog { long_names: true }.catalog().unwrap();
        assert_eq!(filtered_kind("input-long"), Some(NodeKind::Source));
        assert_eq!(long.inputs[0].name, LONG_INPUT_NAME);
        assert!(LONG_INPUT_NAME.len() > 100);
        assert_eq!(long.inputs[0].endpoint_id, ordinary.inputs[0].endpoint_id);
        assert_eq!(long.outputs, ordinary.outputs);
        assert_eq!(long.processes, ordinary.processes);
        assert_eq!(
            long.default_input_endpoint_id,
            ordinary.default_input_endpoint_id
        );
    }

    #[::core::prelude::v1::test]
    fn meter_scroll_evidence_requires_actual_translation_and_fully_visible_last_cell() {
        let viewport = Bounds {
            origin: point(500., 100.),
            size: size(328., 240.),
        };
        let before = Bounds {
            origin: viewport.origin,
            size: size(1240., 240.),
        };
        let after = Bounds {
            origin: point(-412., 100.),
            size: before.size,
        };
        let last = verify_meter_scroll_end(viewport, before, after).unwrap();
        assert_eq!(last.left(), viewport.right() - 38.);
        assert_eq!(last.right(), viewport.right());
        assert!(verify_meter_scroll_end(viewport, before, before).is_err());
        let mut partial = after;
        partial.origin.x += 20.;
        assert!(verify_meter_scroll_end(viewport, before, partial).is_err());
        let squeezed = Bounds {
            origin: viewport.origin,
            size: viewport.size,
        };
        assert!(verify_meter_scroll_end(viewport, squeezed, squeezed).is_err());
    }

    #[::core::prelude::v1::test]
    fn meter32_fixture_prepares_actual_channels_without_stereo_only_processors() {
        let snapshot =
            chain_fixture(&[NodeKind::Source, NodeKind::Meter, NodeKind::Sink], 32).unwrap();
        assert_eq!(filtered_kind("meter32"), Some(NodeKind::Meter));
        assert_eq!(snapshot.graph.nodes().len(), 3);
        assert_eq!(snapshot.graph.edges().len(), 2);
        assert!(
            snapshot
                .graph
                .nodes()
                .iter()
                .all(|node| node.channels() == 32)
        );
        let mut graph = ServiceGraph::default();
        graph.replace(snapshot.graph, snapshot.nodes).unwrap();
        graph.preparation().engine_project(256).unwrap();
    }

    #[::core::prelude::v1::test]
    fn overview_preserves_measured_height_and_rejects_a_partially_clipped_focus_action() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        snapshot.graph = std::sync::Arc::new(fixture().unwrap());
        for viewport in [(1396., 800.), (980., 620.)] {
            for (kind, height) in [(NodeKind::Gain, 320.), (NodeKind::Eq, 500.)] {
                let id = snapshot.graph.node_of_kind(kind).unwrap().0.id();
                let heights = BTreeMap::from([(id, height)]);
                let full_layout =
                    measured_focus_layout(&snapshot, &[kind], viewport, &heights).unwrap();
                let full = measurement_fixture(&snapshot, &full_layout, viewport, heights);
                let layout = overview_focus_layout(&snapshot, kind, &full).unwrap();
                assert_eq!(layout.camera.zoom, 0.5);
                let world = CanvasWorld::measured(&snapshot, &layout.positions, &full.heights);
                let node = world
                    .nodes
                    .iter()
                    .find(|node| node.node_id == Some(id))
                    .unwrap();
                let card = NodeLayout::new(node).project(layout.camera).card;
                let body = Bounds {
                    origin: point(card.left() + 5., card.top() + 29.),
                    size: size(card.size.width - 10., card.size.height - 39.),
                };
                let measured = OverviewMeasurements {
                    native: NativeMeasurements {
                        viewport,
                        heights: full.heights.clone(),
                        bodies: BTreeMap::from([(id, body)]),
                        meters: BTreeMap::new(),
                    },
                    focus: Bounds {
                        origin: body.origin,
                        size: size(body.size.width, 28.),
                    },
                };
                let verify = |measured: &OverviewMeasurements| {
                    verify_overview_bounds(
                        &snapshot,
                        &layout.positions,
                        layout.camera,
                        id,
                        measured,
                    )
                };
                let result = verify(&measured).unwrap();
                assert_eq!(result["full_measured_height"], height);
                assert_eq!(result["focus_action_fully_visible"], true);
                // An observed visible element can still be partially clipped.
                let mut clipped = measured.clone();
                clipped.focus.origin.y = body.bottom() - 1.;
                assert!(verify(&clipped).is_err());
                let mut stale = measured.clone();
                stale
                    .native
                    .heights
                    .insert(id, (body.size.height + 68.).ceil());
                assert!(verify(&stale).is_err());
                assert!(
                    verify_native_bounds(
                        &snapshot,
                        &layout.positions,
                        layout.camera,
                        &[id],
                        &measured.native
                    )
                    .is_err()
                );
            }
        }
        finish_service(runtime);
    }

    #[::core::prelude::v1::test]
    fn meter32_validator_caps_only_the_outer_meter_viewport() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        snapshot.graph = std::sync::Arc::new(
            chain_fixture(&[NodeKind::Source, NodeKind::Meter, NodeKind::Sink], 32).unwrap(),
        );
        let id = snapshot.graph.node_of_kind(NodeKind::Meter).unwrap().0.id();
        let viewport = (2000., 800.);
        let heights = BTreeMap::from([(id, 500.)]);
        let layout =
            measured_focus_layout(&snapshot, &[NodeKind::Meter], viewport, &heights).unwrap();
        let mut measured = measurement_fixture(&snapshot, &layout, viewport, heights);
        let body = measured.bodies[&id];
        let cap = crate::ui::widgets::meters::node_meter_width(8);
        measured.meters.insert(
            id,
            Bounds {
                origin: point(body.right() - cap, body.top()),
                size: size(cap, body.size.height),
            },
        );
        let verify = |measured: &NativeMeasurements| {
            verify_native_bounds(
                &snapshot,
                &layout.positions,
                layout.camera,
                &layout.ids,
                measured,
            )
        };
        assert_eq!(verify(&measured).unwrap()[0]["channels"], 32);
        measured.meters.get_mut(&id).unwrap().size.width += 10.;
        assert!(verify(&measured).is_err());
        finish_service(runtime);
    }

    #[::core::prelude::v1::test]
    fn expanded_variants_apply_real_valid_graph_edits_without_changing_connections() {
        let initial = fixture().unwrap();
        for (variant, kind) in [
            ("rack-populated", NodeKind::Rack),
            ("bus-expanded", NodeKind::Bus),
        ] {
            assert_eq!(filtered_kind(variant), Some(kind));
            let mut graph = ServiceGraph::default();
            graph
                .replace(initial.graph.clone(), initial.nodes.clone())
                .unwrap();
            for edit in expansion_edits(&graph.snapshot(), variant).unwrap() {
                graph.edit(edit).unwrap();
            }
            let snapshot = graph.snapshot();
            assert_eq!(snapshot.graph.edges(), initial.graph.edges());
            assert_eq!(snapshot.graph.nodes().len(), initial.graph.nodes().len());
            let (node, state) = snapshot.node_of_kind(kind).unwrap();
            if kind == NodeKind::Rack {
                assert_eq!(snapshot.graph, initial.graph);
                assert_eq!(state.builtin.rack(), Some(&populated_rack()));
                let rack = state.builtin.rack().unwrap();
                assert_eq!(rack.slots.len(), 4);
                assert!(rack.valid_for(2));
                assert!(rack.slots.iter().all(|slot| !slot.bypass));
            } else {
                assert_eq!(node.inputs().len(), 6);
                assert_eq!(
                    snapshot
                        .graph
                        .edges()
                        .iter()
                        .filter(|edge| edge.dst() == node.id())
                        .count(),
                    1
                );
            }
            graph.preparation().engine_project(256).unwrap();
        }
        assert!(
            expansion_edits(&initial, "recording-states")
                .unwrap()
                .is_empty()
        );
    }

    #[::core::prelude::v1::test]
    fn expanded_rack_bus_recorder_and_spectrum_require_meters_to_follow_measured_content() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        let initial = fixture().unwrap();
        for (variant, kind, rack_slots) in [
            ("rack-populated", NodeKind::Rack, Some(1)),
            ("rack-populated", NodeKind::Rack, Some(2)),
            ("rack-populated", NodeKind::Rack, Some(4)),
            ("bus-expanded", NodeKind::Bus, None),
            ("recorder", NodeKind::Recorder, None),
            ("spectrum", NodeKind::SpectrumAnalyzer, None),
        ] {
            let mut graph = ServiceGraph::default();
            graph
                .replace(initial.graph.clone(), initial.nodes.clone())
                .unwrap();
            for edit in expansion_edits(&graph.snapshot(), variant).unwrap() {
                graph.edit(edit).unwrap();
            }
            if let Some(count) = rack_slots {
                let mut rack = populated_rack();
                rack.slots.truncate(count);
                let node = graph
                    .snapshot()
                    .node_of_kind(NodeKind::Rack)
                    .unwrap()
                    .0
                    .id();
                graph
                    .edit(crate::service::GraphEdit::SetBuiltin {
                        node,
                        state: crate::service::BuiltinState::Rack(rack),
                    })
                    .unwrap();
            }
            snapshot.graph = std::sync::Arc::new(graph.snapshot());
            let id = snapshot.graph.node_of_kind(kind).unwrap().0.id();
            let viewport = (700., 620.);
            let heights = BTreeMap::from([(id, 500.)]);
            let layout = measured_focus_layout(&snapshot, &[kind], viewport, &heights).unwrap();
            let mut measured = measurement_fixture(&snapshot, &layout, viewport, heights);
            assert!(
                verify_native_bounds(
                    &snapshot,
                    &layout.positions,
                    layout.camera,
                    &layout.ids,
                    &measured
                )
                .is_err()
            );
            let body = measured.bodies[&id];
            measured.meters.insert(
                id,
                Bounds {
                    origin: point(body.right() - 100., body.top()),
                    size: size(100., body.size.height),
                },
            );
            let verified = verify_native_bounds(
                &snapshot,
                &layout.positions,
                layout.camera,
                &layout.ids,
                &measured,
            )
            .unwrap();
            assert_eq!(verified[0]["vertical_meter_verified"], true);
            measured.meters.get_mut(&id).unwrap().size.height -= 30.;
            assert!(
                verify_native_bounds(
                    &snapshot,
                    &layout.positions,
                    layout.camera,
                    &layout.ids,
                    &measured
                )
                .is_err()
            );
        }
        finish_service(runtime);
    }

    #[::core::prelude::v1::test]
    fn saved_wav_verification_uses_engine_header_and_rejects_stale_lengths_or_format() {
        use moiren_engine::recorder::WavRecording;
        let wav = WavRecording::from_writer(Cursor::new(Vec::new()), 2, 48_000).unwrap();
        let mut bytes = wav.finish().unwrap().into_inner();
        // Seed the real engine header, then supply deterministic finalized
        // counts for this parser test. Native capture reads a real service WAV.
        let frames = 16u32;
        let data_bytes = frames * 8;
        bytes.resize(58 + data_bytes as usize, 0);
        bytes[4..8].copy_from_slice(&(50 + data_bytes).to_le_bytes());
        bytes[46..50].copy_from_slice(&frames.to_le_bytes());
        bytes[54..58].copy_from_slice(&data_bytes.to_le_bytes());
        let frames = u64::from(frames);
        let header: [u8; 58] = bytes[..58].try_into().unwrap();
        assert!(frames > 0);
        verify_wav_header(&header, bytes.len() as u64, frames, 48_000, 2).unwrap();
        assert!(verify_wav_header(&header, bytes.len() as u64, frames + 1, 48_000, 2).is_err());
        assert!(verify_wav_header(&header, bytes.len() as u64, frames, 44_100, 2).is_err());
        assert!(verify_wav_header(&header, bytes.len() as u64 - 1, frames, 48_000, 2).is_err());
        let mut unfinished = header;
        unfinished[4..8].fill(0);
        assert!(verify_wav_header(&unfinished, bytes.len() as u64, frames, 48_000, 2).is_err());
    }

    /// Synthetic native geometry for pure layout regression tests only. The
    /// capture path always reads observed native elements after a real draw.
    fn measurement_fixture(
        snapshot: &AppSnapshot,
        layout: &FocusLayout,
        viewport: (f32, f32),
        heights: BTreeMap<NodeId, f32>,
    ) -> NativeMeasurements {
        let world = CanvasWorld::measured(snapshot, &layout.positions, &heights);
        let bodies = world
            .nodes
            .iter()
            .filter_map(|node| {
                let id = node.node_id?;
                if !layout.ids.contains(&id) {
                    return None;
                }
                let card = NodeLayout::new(node).project(layout.camera).card;
                Some((
                    id,
                    Bounds {
                        origin: point(card.left() + 10., card.top() + 58.),
                        size: size(card.size.width - 20., heights[&id] - 68.),
                    },
                ))
            })
            .collect();
        NativeMeasurements {
            viewport,
            heights,
            bodies,
            meters: BTreeMap::new(),
        }
    }

    #[::core::prelude::v1::test]
    fn measured_focus_recenters_tall_cards_and_rejects_missing_or_unfittable_heights() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        snapshot.graph = std::sync::Arc::new(fixture().unwrap());
        let kind = NodeKind::Compressor;
        let id = snapshot.graph.node_of_kind(kind).unwrap().0.id();
        let viewport = (700., 620.);
        let provisional = focus_layout(&snapshot, &[kind], viewport).unwrap();
        assert_eq!(provisional.camera.pan_y, 72.);

        let heights = BTreeMap::from([(id, 550.)]);
        let measured = measured_focus_layout(&snapshot, &[kind], viewport, &heights).unwrap();
        assert_eq!(measured.camera.pan_y, 58.);
        assert_eq!(measured.camera.zoom, 1.);
        let geometry = measurement_fixture(&snapshot, &measured, viewport, heights);
        let verified = verify_native_bounds(
            &snapshot,
            &measured.positions,
            measured.camera,
            &measured.ids,
            &geometry,
        )
        .unwrap();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0]["card"][3], 550.);
        assert!(
            verify_native_bounds(
                &snapshot,
                &provisional.positions,
                provisional.camera,
                &provisional.ids,
                &geometry,
            )
            .is_err()
        );
        assert!(measured_focus_layout(&snapshot, &[kind], viewport, &BTreeMap::new()).is_err());
        for height in [0., -1., f32::NAN, f32::INFINITY, 553.] {
            assert!(
                measured_focus_layout(
                    &snapshot,
                    &[kind],
                    viewport,
                    &BTreeMap::from([(id, height)]),
                )
                .is_err()
            );
        }
        finish_service(runtime);
    }

    #[::core::prelude::v1::test]
    fn native_bounds_reject_clipped_content_stale_geometry_and_reduced_zoom() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        snapshot.graph = std::sync::Arc::new(fixture().unwrap());
        let kinds = [NodeKind::Source, NodeKind::Eq];
        let heights = kinds
            .into_iter()
            .zip([320., 500.])
            .map(|(kind, height)| (snapshot.graph.node_of_kind(kind).unwrap().0.id(), height))
            .collect::<BTreeMap<_, _>>();
        let viewport = (980., 620.);
        let layout = measured_focus_layout(&snapshot, &kinds, viewport, &heights).unwrap();
        let measured = measurement_fixture(&snapshot, &layout, viewport, heights);
        let verify = |camera, measured: &NativeMeasurements| {
            verify_native_bounds(&snapshot, &layout.positions, camera, &layout.ids, measured)
        };
        assert_eq!(verify(layout.camera, &measured).unwrap().len(), 2);
        let mut zoomed = layout.camera;
        zoomed.zoom = 0.75;
        assert!(verify(zoomed, &measured).is_err());
        for axis in [true, false] {
            let mut clipped = measured.clone();
            let body = clipped.bodies.get_mut(&layout.ids[0]).unwrap();
            if axis {
                body.size.width += 1000.;
            } else {
                body.size.height += 1000.;
            }
            assert!(verify(layout.camera, &clipped).is_err());
        }
        let mut stale = measured.clone();
        stale.bodies.get_mut(&layout.ids[0]).unwrap().origin.y += 20.;
        assert!(verify(layout.camera, &stale).is_err());
        let mut missing = measured.clone();
        missing.heights.remove(&layout.ids[0]);
        assert!(verify(layout.camera, &missing).is_err());
        finish_service(runtime);
    }

    #[::core::prelude::v1::test]
    fn single_node_filter_accepts_all_templates_and_rejects_unknown_or_multiple_slugs() {
        assert_eq!(capture_filter(None).unwrap(), None);
        for slug in KINDS
            .map(slug)
            .into_iter()
            .chain(["application", "tone"])
            .chain(SCENARIOS)
        {
            assert_eq!(capture_filter(Some(slug)).unwrap(), Some(slug));
        }
        assert_eq!(capture_filter(Some(" input ")).unwrap(), Some("input"));
        for invalid in ["", "unknown", "input,eq", "input eq"] {
            assert!(capture_filter(Some(invalid)).is_err());
        }
    }

    #[::core::prelude::v1::test]
    fn tone_precision_adds_only_two_open_binding_frames_to_its_filter() {
        assert_eq!(super::single_node_frame_count(Some("tone")), 6);
        for filter in ["meter32", "input-long"] {
            assert_eq!(super::single_node_frame_count(Some(filter)), 6);
        }
        for filter in ["application", "input", "eq", "spectrum", "recorder"] {
            assert_eq!(super::single_node_frame_count(Some(filter)), 4);
        }
    }

    #[::core::prelude::v1::test]
    fn capture_fixture_contains_every_node_kind_in_one_valid_connected_chain() {
        let snapshot = fixture().unwrap();
        assert_eq!(snapshot.graph.nodes().len(), KINDS.len());
        assert_eq!(snapshot.graph.edges().len(), KINDS.len() - 1);
        for kind in KINDS {
            assert_eq!(
                snapshot
                    .graph
                    .nodes()
                    .iter()
                    .filter(|node| node.kind() == kind)
                    .count(),
                1
            );
        }
        let mut slugs = KINDS.map(slug).to_vec();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), KINDS.len());
        let catalog = CaptureCatalog::default().catalog().unwrap();
        assert_eq!(catalog.inputs.len(), 1);
        assert_eq!(catalog.outputs.len(), 1);
        assert_eq!(catalog.processes.len(), 1);
    }

    #[::core::prelude::v1::test]
    fn source_templates_rebind_through_service_graph_without_changing_topology() {
        use crate::service::{GraphEdit, InputSelection, ServiceGraph};
        use moiren_core::graph::NodeKind;
        let snapshot = fixture().unwrap();
        let topology = snapshot.graph.clone();
        let source = snapshot.node_of_kind(NodeKind::Source).unwrap().0.id();
        let mut graph = ServiceGraph::default();
        graph.replace(snapshot.graph, snapshot.nodes).unwrap();
        let catalog = CaptureCatalog::default().catalog().unwrap();
        for (_, selection) in source_variants() {
            if let InputSelection::Process {
                pid,
                creation_time_100ns,
            } = &selection
            {
                assert!(
                    catalog
                        .processes
                        .iter()
                        .any(|row| row.pid == *pid
                            && row.creation_time_100ns == *creation_time_100ns)
                );
            }
            graph
                .edit(GraphEdit::BindSource {
                    node: source,
                    source: selection.clone(),
                })
                .unwrap();
            let rebound = graph.snapshot();
            assert_eq!(rebound.node(source).unwrap().source, Some(selection));
            assert_eq!(rebound.graph, topology);
            assert_eq!(rebound.active_revision, None);
        }
    }

    #[::core::prelude::v1::test]
    fn focus_groups_fit_current_native_card_sizes_without_overview_scaling() {
        let runtime = spawn_service();
        let mut snapshot = (*initial_projection(&runtime.handle())).clone();
        snapshot.graph = std::sync::Arc::new(fixture().unwrap());
        for (viewport, count) in [((1396., 800.), 3), ((980., 620.), 2), ((700., 620.), 1)] {
            let groups = capture_groups(&snapshot, &KINDS, viewport, count).unwrap();
            assert_eq!(groups.concat(), KINDS);
            for kinds in groups {
                let FocusLayout {
                    positions,
                    camera,
                    ids,
                } = focus_layout(&snapshot, kinds, viewport).unwrap();
                let world =
                    crate::ui::pages::graph_canvas::CanvasWorld::positioned(&snapshot, &positions);
                assert_eq!(camera.zoom, 1.);
                for node in world
                    .nodes
                    .iter()
                    .filter(|node| node.node_id.is_some_and(|id| ids.contains(&id)))
                {
                    let (x, y) = camera.world_to_canvas((node.x, node.y));
                    assert!(x >= 0. && y >= 56.);
                    assert!(x + node.width() * camera.zoom <= viewport.0);
                    assert!(y + node.height() * camera.zoom <= viewport.1);
                }
            }
        }
        let wide = [
            NodeKind::SpectrumAnalyzer,
            NodeKind::Recorder,
            NodeKind::Sink,
        ];
        assert!(focus_layout(&snapshot, &wide, (1396., 800.)).is_err());
        let groups = capture_groups(&snapshot, &wide, (1396., 800.), 3).unwrap();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(groups.concat(), wide);
        assert!(capture_groups(&snapshot, &KINDS, (100., 100.), 3).is_err());
        assert!(capture_groups(&snapshot, &KINDS, (1396., 800.), 0).is_err());
        assert!(focus_layout(&snapshot, &KINDS, (1396., 800.)).is_err());
        assert!(focus_layout(&snapshot, &KINDS[..1], (100., 100.)).is_err());
        finish_service(runtime);
    }
}
