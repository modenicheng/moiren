//! Developer-only capture of the production shell through GPUI's native renderer.
use super::*;
use crate::service::test_backend as meter_backend;
mod mixer_states;
mod node_workstations;

fn meter_service() -> anyhow::Result<ServiceRuntime> {
    use crate::service::{
        GraphEdit, InputSelection, ManagedDriver, RunLimit, ServiceGraph, SessionSpec,
    };
    use moiren_core::graph::NodeKind;
    let backend = meter_backend::FakeBackend::default();
    backend
        .0
        .timed_meter_frames
        .store(96, std::sync::atomic::Ordering::Relaxed);
    let service = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))?;
    let handle = service.handle();
    let mut graph = ServiceGraph::default();
    for _ in 0..4 {
        graph
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Gain,
                channels: 2,
            })
            .map_err(anyhow::Error::msg)?;
    }
    let snapshot = graph.snapshot();
    let edge = snapshot
        .graph
        .edges()
        .iter()
        .find(|edge| snapshot.graph.get_node(edge.dst()).unwrap().kind() == NodeKind::Sink)
        .unwrap();
    let (mut source, destination) = (edge.src_port(), edge.dst_port());
    graph
        .edit(GraphEdit::Disconnect(edge.id()))
        .map_err(anyhow::Error::msg)?;
    for node in snapshot.graph.nodes().iter().skip(4) {
        graph
            .edit(GraphEdit::Connect {
                source,
                destination: node.inputs()[0].id(),
            })
            .map_err(anyhow::Error::msg)?;
        source = node.outputs()[0].id();
    }
    graph
        .edit(GraphEdit::Connect {
            source,
            destination,
        })
        .map_err(anyhow::Error::msg)?;
    let snapshot = graph.snapshot();
    handle
        .try_request(AppRequest::ReplaceGraph {
            graph: snapshot.graph,
            nodes: snapshot.nodes,
        })
        .map_err(|error| anyhow::anyhow!("capture graph: {error:?}"))?;
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone { frequency_hz: 440. },
            output_endpoint_id: "capture-output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.75,
            pan: 0.,
            max_block_frames: 256,
        }))
        .map_err(|error| anyhow::anyhow!("capture start: {error:?}"))?;
    Ok(service)
}

pub fn run_ui_capture() -> anyhow::Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "docs/designs/screenshots/application".into());
    std::fs::create_dir_all(&directory)?;
    if std::env::var_os("MOIREN_CAPTURE_NODES").is_some() {
        return node_workstations::run(&directory);
    }
    let meter_capture = std::env::var_os("MOIREN_CAPTURE_METERS").is_some();
    #[cfg(feature = "ui-profiler")]
    if meter_capture {
        profiler::set_trace_enabled(true);
    }
    // Capture presentation without starting hardware or changing its format.
    #[cfg(windows)]
    let service = if meter_capture {
        meter_service()?
    } else {
        ServiceRuntime::spawn_with_driver(crate::service::ManagedDriver::windows())?
    };
    #[cfg(not(windows))]
    let service = if meter_capture {
        meter_service()?
    } else {
        ServiceRuntime::spawn()?
    };
    let handle = service.handle();
    let initial = crate::ui::state::initial_snapshot(&handle)?;
    let preferences = crate::project::load_preferences().unwrap_or_default();
    let mut owned = OwnedService(Some(service));
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture_failure = failed.clone();
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("embedded font");
            let state = cx.new(|_| UiState::new(handle.clone(), initial));
            // Match production startup so persisted intent and service state
            // agree in captures without writing the user's preferences.
            state.update(cx, |s, _| {
                if !meter_capture { s.restore_preferences(&preferences); }
                s.dispatch(AppRequest::RefreshCatalog);
            });
            let displays = cx.displays();
            for display in &displays { eprintln!("Capture display {:?}: {:?}", display.id(), display.bounds()); }
            let primary = std::env::var_os("MOIREN_CAPTURE_PRIMARY").is_some();
            let display = meter_capture.then(|| displays.iter().find(|display| {
                (display.bounds().origin.x >= px(0.)) == primary
            }).cloned()).flatten();
            let bounds = display.as_ref().map(|display| {
                let bounds = display.bounds();
                WindowBounds::Windowed(Bounds::new(
                    bounds.origin + point((bounds.size.width - px(1440.)) / 2., (bounds.size.height - px(900.)) / 2.),
                    size(px(1440.), px(900.)),
                ))
            }).unwrap_or_else(|| WindowBounds::centered(size(px(1440.), px(900.)), cx));
            let (window, shell) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(bounds),
                    display_id: display.map(|display| display.id()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Moiren · UI capture".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| StudioShell::new(state, window, cx, None)),
            )
            .expect("capture window");
            shell.update(cx, |s, cx| s.spawn_poll(cx));
            cx.spawn(async move |cx| {
                // Shared screenshot block: draw the current frame, save it, and
                // flag the run as failed without aborting the remaining pages.
                let capture = |cx: &mut AsyncApp, path: std::path::PathBuf| {
                    let result = window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        window.draw(cx).clear(cx);
                        window.render_to_image()?.save(&path)?;
                        Ok(())
                    });
                    if !matches!(result, Ok(Ok(()))) {
                        capture_failure.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    eprintln!("Capture {}: {result:?}", path.display());
                };
                cx.background_executor()
                    .timer(Duration::from_millis(1200))
                    .await;
                if meter_capture {
                    for (name, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
                        window.update(cx, |_, window, _| window.resize(size(px(width), px(height)))).unwrap();
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        shell.update(cx, |shell, cx| {
                            shell.set_page(Page::Mixer, cx);
                            shell.inspector_open = false;
                            cx.notify();
                        });
                        // Exclude resize and inspector spring settling from steady-state
                        // cadence. Keep the historical three-second default reproducible.
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        #[cfg(feature = "ui-profiler")]
                        let mut frames = cx.update(|cx| mixer::performance::PlatformTrace::new(cx, window.window_id()));
                        let before = shell.read_with(cx, |shell, _| {
                            let mut trace = shell.mixer.trace.borrow_mut();
                            trace.start_sample();
                            (trace.strip_renders, trace.meter_paints)
                        });
                        let seconds = std::env::var("MOIREN_CAPTURE_SECONDS").ok()
                            .and_then(|value| value.parse::<u64>().ok()).unwrap_or(3).clamp(1, 60);
                        for _ in 0..seconds * 4 {
                            cx.background_executor().timer(Duration::from_millis(250)).await;
                            // Drain the bounded journal regularly; serialize only after
                            // measurement so JSON work cannot masquerade as render cost.
                            #[cfg(feature = "ui-profiler")]
                            frames.collect();
                        }
                        #[cfg(feature = "ui-profiler")]
                        frames.write(&directory.join(format!("platform-{name}.json"))).unwrap();
                        shell.read_with(cx, |shell, _| {
                            let trace = shell.mixer.trace.borrow();
                            eprintln!("Steady {name}: strip renders={}, meter paints={}",
                                trace.strip_renders - before.0, trace.meter_paints - before.1);
                            trace.write(&directory.join(format!("meter-{name}.json"))).unwrap();
                        });
                        capture(cx, directory.join(format!("mixer-{name}.png")));
                    }
                    shell.update(cx, |shell, cx| {
                        let snapshot = &shell.state.read(cx).snapshot;
                        eprintln!("Native capture: phase={:?}, live={}, visible={}, nodes={}", snapshot.phase,
                            snapshot.graph.nodes.iter().filter(|node| shell.node_meters.channel_levels(node.id).is_some()).count(),
                            shell.node_meter_points.borrow().len(), snapshot.graph.nodes.len());
                    });
                    if std::env::var_os("MOIREN_CAPTURE_STATES").is_some() {
                        mixer_states::capture(window, &shell, &directory, &capture, cx)
                            .await.expect("Mixer state captures");
                    }
                    cx.update(|cx| cx.quit());
                    return;
                }
                for (name, width, height) in
                    [("comfortable", 1440., 900.), ("compact", 1024., 720.)]
                {
                    let _ = window.update(cx, |_, window, _| {
                        window.resize(size(px(width), px(height)))
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    for page in [Page::Graph, Page::Mixer, Page::Matrix, Page::Devices] {
                        shell.update(cx, |s, cx| {
                            s.density = if name == "compact" {
                                Density::Compact
                            } else {
                                Density::Comfortable
                            };
                            s.set_page(page, cx);
                        });
                        cx.background_executor()
                            .timer(Duration::from_millis(if page == Page::Mixer {
                                1500
                            } else {
                                100
                            }))
                            .await;
                        if page == Page::Mixer {
                            shell.update(cx, |shell, cx| {
                                let snapshot = &shell.state.read(cx).snapshot;
                                let live = snapshot
                                    .graph
                                    .nodes
                                    .iter()
                                    .filter(|node| shell.node_meters.levels(node.id).is_some())
                                    .count();
                                eprintln!(
                                    "Native mixer meters: {live}/{} live, {} visible, {} topics, phase {:?}",
                                    snapshot.graph.nodes.len(),
                                    shell.node_meter_points.borrow().len(),
                                    snapshot.signal_topics.node_levels.len(),
                                    snapshot.phase,
                                );
                            });
                        }
                        capture(
                            cx,
                            directory.join(format!(
                                "{name}-{}.png",
                                page.title().to_lowercase()
                            )),
                        );
                    }
                    // Inspect a real catalog row without binding resources or
                    // submitting any audio request. The override stays a draft.
                    shell.update(cx, |s, cx| {
                        let id = s.state.read(cx).snapshot.catalog.outputs.first()
                            .map(|row| row.endpoint_id.clone());
                        if let Some(id) = id {
                            s.inspect_device(id.clone(), cx);
                            settings::device_stream_mut(&mut s.settings.device.audio, settings::Direction::Output, &id);
                        }
                    });
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    capture(cx, directory.join(format!("{name}-device-override.png")));
                    let _ = window.update(cx, |_, window, cx| {
                        shell.update(cx, |s, cx| {
                            s.set_page(Page::Graph, cx);
                            s.open_settings(window, cx);
                        });
                    });
                    for (suffix, section) in [
                        ("settings", settings::Section::Audio),
                        ("settings-appearance", settings::Section::Appearance),
                        ("settings-diagnostics", settings::Section::Diagnostics),
                    ] {
                        shell.update(cx, |s, cx| {
                            s.settings.section = section;
                            cx.notify();
                        });
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        capture(cx, directory.join(format!("{name}-{suffix}.png")));
                    }
                }
                shell.update(cx, |shell, cx| {
                    shell.set_page(Page::Graph, cx);
                    shell.sidebar_open = false;
                    shell.inspector_open = false;
                    let order = shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .graph
                        .topological_order()
                        .unwrap();
                    for (index, id) in order.iter().enumerate() {
                        let position = match index {
                            0 => (30., 175.),
                            1 => (290., 575.),
                            2 => (480., 35.),
                            _ => (810. + index.saturating_sub(3) as f32 * 250., 175.),
                        };
                        shell.node_positions.insert(*id, position);
                    }
                    cx.notify();
                });
                for (name, zoom) in [
                    ("graph-fit", None),
                    ("graph-quarter", Some(0.25)),
                    ("graph-fractional", Some(0.89)),
                    ("graph-native-size", Some(1.)),
                    ("graph-double", Some(2.)),
                ] {
                    shell.update(cx, |shell, cx| {
                        shell.camera = match zoom {
                            Some(zoom) => Camera {
                                pan_x: 24.,
                                pan_y: 64.,
                                zoom,
                                adjusted: true,
                            },
                            None => Camera::default(),
                        };
                        cx.notify();
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    capture(cx, directory.join(format!("{name}.png")));
                }
                let _ = window.update(cx, |_, window, _| window.resize(size(px(1440.), px(900.))));
                for (name, page) in [
                    ("node-inspector", Page::Graph),
                    ("mixer-inspector", Page::Mixer),
                    ("connection-inspector", Page::Graph),
                ] {
                    shell.update(cx, |shell, cx| {
                        shell.set_page(page, cx);
                        shell.camera = Camera::default();
                        shell.sidebar_open = page == Page::Graph;
                        shell.inspector_open = true;
                        let gain = shell
                            .state
                            .read(cx)
                            .snapshot
                            .graph
                            .graph
                            .nodes()
                            .iter()
                            .find(|node| node.kind() == moiren_core::graph::NodeKind::Gain)
                            .map(|node| node.id());
                        shell.selected_node = gain;
                        shell.selected_route = None;
                        if name == "connection-inspector" {
                            shell.selected_node = None;
                            let state = shell.state.read(cx);
                            shell.selected_route = state.snapshot.graph.graph.edges().first()
                                .map(|edge| (edge.src_port(), edge.dst_port()));
                        }
                        cx.notify();
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    capture(cx, directory.join(format!("{name}.png")));
                }
                shell.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::EditGraph(crate::service::GraphEdit::AddNode {
                            kind: moiren_core::graph::NodeKind::Compressor,
                            channels: 2,
                        }))
                    });
                });
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                shell.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, _| state.poll());
                    shell.set_page(Page::Graph, cx);
                    shell.inspector_open = true;
                    shell.sidebar_open = true;
                    shell.selected_node = shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .graph
                        .nodes()
                        .iter()
                        .find(|node| node.kind() == moiren_core::graph::NodeKind::Compressor)
                        .map(|node| node.id());
                    if let Some(key) = shell.selected_node {
                        shell.node_positions.insert(key, (480., 370.));
                    }
                    shell.camera = Camera::default();
                    cx.notify();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                capture(cx, directory.join("compressor-inspector.png"));
                for kind in [
                    moiren_core::graph::NodeKind::Source,
                    moiren_core::graph::NodeKind::Bus,
                ] {
                    shell.update(cx, |shell, cx| {
                        shell.state.update(cx, |state, _| {
                            state.dispatch(AppRequest::EditGraph(crate::service::GraphEdit::AddNode {
                                kind,
                                channels: 2,
                            }));
                        });
                    });
                    let deadline = std::time::Instant::now() + Duration::from_secs(3);
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(10))
                            .await;
                        let pending = shell.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| state.poll());
                            shell.state.read(cx).pending_request
                        });
                        if !pending {
                            break;
                        }
                        if std::time::Instant::now() >= deadline {
                            capture_failure.store(true, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }
                    }
                }
                shell.update(cx, |shell, cx| {
                    shell.set_page(Page::Graph, cx);
                    shell.sidebar_open = false;
                    shell.inspector_open = false;
                    shell.selected_route = None;
                    shell.selected_node = None;
                    shell.camera = Camera {
                        pan_x: 24.,
                        pan_y: 64.,
                        zoom: 1.,
                        adjusted: true,
                    };
                    let graph = &shell.state.read(cx).snapshot.graph.graph;
                    let ids = graph.nodes().iter().map(|node| node.id()).collect::<Vec<_>>();
                    for (index, id) in ids.iter().enumerate() {
                        shell.node_positions.insert(
                            *id,
                            (40. + (index % 4) as f32 * 330., 120. + (index / 4) as f32 * 190.),
                        );
                    }
                    cx.notify();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                capture(cx, directory.join("node-families.png"));
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    owned.finish()?;
    anyhow::ensure!(
        !failed.load(std::sync::atomic::Ordering::Relaxed),
        "native UI capture failed"
    );
    Ok(())
}
