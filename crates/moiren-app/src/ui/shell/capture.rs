//! Developer-only capture of the production shell through GPUI's native renderer.
use super::*;
use crate::service::test_backend as meter_backend;
mod mixer_states;

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

/// A plausible program spectrum for the capture on the analyzer's 121-point
/// grid: a pink-ish slope with a low-end bump, a tone at 1 kHz and a
/// presence peak.
fn demo_spectrum_curve() -> Vec<f32> {
    let body = |index: f32| {
        let position = index / 120.;
        let peak = |center: f32, width: f32, height: f32| {
            height * (-((index - center) / width).powi(2)).exp()
        };
        -22. - 26. * position + peak(28., 6., 9.) + peak(68., 2.5, 16.) + peak(92., 8., 5.)
    };
    (0..121)
        .map(|index| body(index as f32).clamp(-96., 0.))
        .collect()
}

/// Where a drag of the first mixer strip's fader starts: the thumb's center
/// on the registered track. `None` before the strips laid out.
fn fader_press(shell: &StudioShell, cx: &App) -> Option<Point<Pixels>> {
    let node = shell.mixer.presentation.order.first()?;
    let slider = shell.mixer.presentation.faders.get(node)?.state.read(cx);
    let bounds = slider.bounds();
    (bounds.size.height > px(0.)).then(|| {
        // The fader's value space is inverted: 1 is the track top.
        point(
            bounds.center().x,
            bounds.top() + bounds.size.height * (1. - slider.value().end()),
        )
    })
}

/// The gesture `MOIREN_CAPTURE_DRAG` traces on the first strip's fader.
#[derive(Clone, Copy, PartialEq)]
enum Sweep {
    /// A held pointer drag: the kit slider's drag-and-drop path.
    Drag,
    /// A precision-touchpad pan: pixel wheel deltas through the fader's
    /// gesture surface, which reach the screen as nested-model rebuilds.
    Pan,
}

/// The `MOIREN_CAPTURE_DRAG` gesture: sweeps the fader at `press` along a
/// ±40 px, one-second sine until `until` passes. A drag presses, moves the
/// pointer twice per frame and releases at the end; a pan scrolls once per
/// frame by the sweep's travel since the previous frame. The events take the
/// real input path, so the trace prices a live gesture frame by frame.
fn sweep_fader(
    window: &mut Window,
    press: Point<Pixels>,
    sweep: Sweep,
    until: Instant,
    cx: &mut App,
) {
    if sweep == Sweep::Drag {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: press,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    }
    sweep_frame(window, press, sweep, Instant::now(), 0., until);
}

fn sweep_frame(
    window: &mut Window,
    press: Point<Pixels>,
    sweep: Sweep,
    started: Instant,
    last: f32,
    until: Instant,
) {
    window.on_next_frame(move |window, cx| {
        let now = Instant::now();
        let offset = |elapsed: f32| 40. * (std::f32::consts::TAU * elapsed).sin();
        let at = |elapsed: f32| press + point(px(0.), px(offset(elapsed)));
        let elapsed = now.duration_since(started).as_secs_f32();
        let done = now >= until;
        match sweep {
            Sweep::Drag if done => window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: at(elapsed),
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            ),
            Sweep::Drag => {
                // Two moves per frame: real pointers report faster than the
                // display refreshes, so a frame folds several Changes into
                // one rebuild.
                for elapsed in [(elapsed - 0.0035).max(0.), elapsed] {
                    window.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent {
                            position: at(elapsed),
                            pressed_button: Some(MouseButton::Left),
                            ..Default::default()
                        }),
                        cx,
                    );
                }
                DispatchEventResult::default()
            }
            Sweep::Pan if done => return,
            Sweep::Pan => window.dispatch_event(
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position: press,
                    delta: ScrollDelta::Pixels(point(px(0.), px(offset(elapsed) - offset(last)))),
                    touch_phase: TouchPhase::Moved,
                    ..Default::default()
                }),
                cx,
            ),
        };
        if !done {
            sweep_frame(window, press, sweep, started, elapsed, until);
        }
    });
}

/// `MOIREN_CAPTURE_ECHO`: how long an edit takes to come back to the UI.
/// Dispatches a no-op strip edit 40 times, 60 ms apart, and times each from
/// the dispatch to the shell notification that delivers the snapshot which
/// consumed it: the service round trip plus however long the control pump
/// takes to pick the publish up. Nothing here drains the state itself, so
/// the number is the production pump's.
async fn echo_probe(shell: &Entity<StudioShell>, cx: &mut AsyncApp) {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    let in_flight: Rc<Cell<Option<(crate::service::RequestId, Instant)>>> = Rc::default();
    let samples: Rc<RefCell<Vec<f64>>> = Rc::default();
    let _delivery = cx.update(|cx| {
        let (in_flight, samples) = (in_flight.clone(), samples.clone());
        cx.observe(shell, move |shell, cx| {
            let Some((request, sent)) = in_flight.get() else {
                return;
            };
            if shell.read(cx).state.read(cx).snapshot.consumed_request_id >= request {
                samples
                    .borrow_mut()
                    .push(sent.elapsed().as_secs_f64() * 1000.);
                in_flight.set(None);
            }
        })
    });
    for _ in 0..40 {
        let request = shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let node = state.snapshot.graph.nodes.first()?;
                let edit = crate::service::GraphEdit::SetMixer {
                    node: node.id,
                    state: node.mixer,
                };
                state.try_dispatch(AppRequest::EditGraph(edit)).ok()
            })
        });
        in_flight.set(request.map(|request| (request, Instant::now())));
        cx.background_executor()
            .timer(Duration::from_millis(60))
            .await;
    }
    in_flight.set(None);
    let mut samples = samples.take();
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p).round() as usize];
    if samples.is_empty() {
        eprintln!("Echo latency: no edit came back");
    } else {
        eprintln!(
            "Echo latency: p50={:.2}ms p95={:.2}ms max={:.2}ms (n={})",
            at(0.5),
            at(0.95),
            at(1.),
            samples.len()
        );
    }
}

pub fn run_ui_capture() -> anyhow::Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "docs/designs/screenshots/application".into());
    std::fs::create_dir_all(&directory)?;
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
                |window, cx| cx.new(|cx| {
                    let mut shell = StudioShell::new(state, window, cx, None);
                    // Capture runs must not touch the developer's real files.
                    shell.autosave.path = None;
                    shell
                }),
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
                    // `MOIREN_CAPTURE_DRAG` traces a live fader gesture instead of
                    // the steady state: `pan` sweeps it with touchpad pixel scrolls,
                    // `inspector` drags with the inspector open on the dragged strip,
                    // any other value drags with the inspector closed.
                    let (sweep, inspector, label, suffix) =
                        match std::env::var("MOIREN_CAPTURE_DRAG").ok().as_deref() {
                            None => (None, false, "Steady", ""),
                            Some("pan") => (Some(Sweep::Pan), false, "Pan", "-pan"),
                            Some("inspector") => {
                                (Some(Sweep::Drag), true, "Drag", "-drag-inspector")
                            }
                            Some(_) => (Some(Sweep::Drag), false, "Drag", "-drag"),
                        };
                    for (name, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
                        window.update(cx, |_, window, _| window.resize(size(px(width), px(height)))).unwrap();
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        shell.update(cx, |shell, cx| {
                            shell.set_page(Page::Mixer, cx);
                            shell.chrome.inspector_open = false;
                            if inspector {
                                shell.selection.node = shell.mixer.presentation.order.first().copied();
                                shell.open_inspector();
                            }
                            cx.notify();
                        });
                        // Exclude resize and inspector spring settling from steady-state
                        // cadence. Keep the historical three-second default reproducible.
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        #[cfg(feature = "ui-profiler")]
                        let mut frames = cx.update(|cx| performance::PlatformTrace::new(cx, window.window_id()));
                        let before = shell.read_with(cx, |shell, cx| {
                            let mut trace = shell.trace.borrow_mut();
                            trace.start_sample();
                            (trace.strip_renders, trace.meter_paints, trace.inspector_renders,
                                shell.state.read(cx).snapshot.graph.candidate_revision)
                        });
                        let seconds = std::env::var("MOIREN_CAPTURE_SECONDS").ok()
                            .and_then(|value| value.parse::<u64>().ok()).unwrap_or(3).clamp(1, 60);
                        if let Some(sweep) = sweep {
                            let press = shell.read_with(cx, fader_press);
                            let until = std::time::Instant::now() + Duration::from_secs(seconds);
                            match press {
                                Some(press) => {
                                    // Inactive windows throttle frame callbacks to
                                    // ~30 fps; the sweep must run at display rate.
                                    let _ = cx.update_window(window, |_, window, cx| {
                                        window.activate_window();
                                        sweep_fader(window, press, sweep, until, cx)
                                    });
                                }
                                None => capture_failure.store(true, std::sync::atomic::Ordering::Relaxed),
                            }
                        }
                        for _ in 0..seconds * 4 {
                            cx.background_executor().timer(Duration::from_millis(250)).await;
                            // Drain the bounded journal regularly; serialize only after
                            // measurement so JSON work cannot masquerade as render cost.
                            #[cfg(feature = "ui-profiler")]
                            frames.collect();
                        }
                        #[cfg(feature = "ui-profiler")]
                        frames.write(&directory.join(format!("platform-{name}{suffix}.json"))).unwrap();
                        shell.read_with(cx, |shell, cx| {
                            let trace = shell.trace.borrow();
                            let revisions = shell.state.read(cx).snapshot.graph.candidate_revision - before.3;
                            eprintln!("{label} {name}: strip renders={}, inspector renders={}, meter paints={}, revisions={revisions}",
                                trace.strip_renders - before.0, trace.inspector_renders - before.2,
                                trace.meter_paints - before.1);
                            trace.write(&directory.join(format!("meter-{name}{suffix}.json"))).unwrap();
                        });
                        capture(cx, directory.join(format!("mixer-{name}{suffix}.png")));
                    }
                    shell.update(cx, |shell, cx| {
                        let state = shell.state.read(cx);
                        let snapshot = &state.snapshot;
                        eprintln!("Native capture: phase={:?}, live={}, visible={}, nodes={}", snapshot.phase,
                            snapshot.graph.nodes.iter().filter(|node| state.node_meters.channel_levels(node.id).is_some()).count(),
                            shell.metering.node_meter_points.borrow().len(), snapshot.graph.nodes.len());
                    });
                    if std::env::var_os("MOIREN_CAPTURE_STATES").is_some() {
                        mixer_states::capture(window, &shell, &directory, &capture, cx)
                            .await.expect("Mixer state captures");
                    }
                    if std::env::var_os("MOIREN_CAPTURE_ECHO").is_some() {
                        echo_probe(&shell, cx).await;
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
                            s.chrome.density = if name == "compact" {
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
                                let state = shell.state.read(cx);
                                let snapshot = &state.snapshot;
                                let live = snapshot
                                    .graph
                                    .nodes
                                    .iter()
                                    .filter(|node| state.node_meters.levels(node.id).is_some())
                                    .count();
                                eprintln!(
                                    "Native mixer meters: {live}/{} live, {} visible, {} topics, phase {:?}",
                                    snapshot.graph.nodes.len(),
                                    shell.metering.node_meter_points.borrow().len(),
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
                            settings::device_stream_mut(&mut s.settings.editor.device.audio, settings::Direction::Output, &id);
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
                        ("settings-interface", settings::Section::Interface),
                        ("settings-diagnostics", settings::Section::Diagnostics),
                    ] {
                        shell.update(cx, |s, cx| {
                            s.settings.editor.section = section;
                            cx.notify();
                        });
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        capture(cx, directory.join(format!("{name}-{suffix}.png")));
                    }
                }
                shell.update(cx, |shell, cx| {
                    shell.set_page(Page::Graph, cx);
                    shell.chrome.sidebar_open = false;
                    shell.chrome.inspector_open = false;
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
                        shell.graph_view.node_positions.insert(*id, position);
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
                        shell.graph_view.camera = match zoom {
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
                // Card states speak through the outline alone, and the fault
                // tooltip follows the hovered card (design 18 B1): fault the
                // first card the way the service would and hover it. No
                // await before the capture — the control-side poll would
                // put the service's own snapshot back — and `capture`
                // draws synchronously, so the rebuilt panel shows the fault
                // outline and the meter frame the tooltip in that draw.
                shell.update(cx, |shell, cx| {
                    shell.graph_view.camera = Camera {
                        pan_x: 24.,
                        pan_y: 64.,
                        zoom: 1.,
                        adjusted: true,
                    };
                    let mut snapshot = (*shell.state.read(cx).snapshot).clone();
                    let id = snapshot.graph.graph.topological_order().unwrap()[0];
                    std::sync::Arc::make_mut(&mut snapshot.graph).faults.push(
                        crate::service::GraphFault {
                            subject: crate::service::GraphSubject::Node(id),
                            cause: "Output endpoint unavailable".into(),
                            hint: Some("Choose another output".into()),
                        },
                    );
                    shell
                        .state
                        .update(cx, |state, _| state.snapshot = std::sync::Arc::new(snapshot));
                    shell.graph_view.hovered_node = Some(id);
                    cx.notify();
                });
                capture(cx, directory.join("graph-fault-tooltip.png"));
                shell.update(cx, |shell, cx| {
                    let mut snapshot = (*shell.state.read(cx).snapshot).clone();
                    std::sync::Arc::make_mut(&mut snapshot.graph).faults.clear();
                    shell
                        .state
                        .update(cx, |state, _| state.snapshot = std::sync::Arc::new(snapshot));
                    shell.graph_view.hovered_node = None;
                    cx.notify();
                });
                // The output card's binding chip hangs its endpoint menu off
                // the chip rect the last canvas prepaint published.
                shell.update(cx, |shell, cx| {
                    let sink = shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .graph
                        .nodes()
                        .iter()
                        .find(|node| node.kind() == moiren_core::graph::NodeKind::Sink)
                        .map(|node| node.id());
                    let anchor = shell.graph_view.canvas_frame.read().and_then(|frame| {
                        let index = frame
                            .world
                            .nodes
                            .iter()
                            .position(|node| node.node_id.is_some() && node.node_id == sink)?;
                        let chip = frame.nodes[index].chip.as_ref()?;
                        Some(frame.window_rect(chip.bounds))
                    });
                    if let (Some(node), Some(anchor)) = (sink, anchor) {
                        shell.graph_view.binding_menu = Some(graph::BindingMenu { node, anchor });
                        cx.notify();
                    }
                });
                capture(cx, directory.join("graph-binding-menu.png"));
                shell.update(cx, |shell, cx| {
                    shell.graph_view.binding_menu = None;
                    cx.notify();
                });
                // The spectrum analyzer's live body (design 18 B3): put an
                // analyzer on the canvas and hand it a curve the way a
                // relayed window would — synchronously, so the rebuilt
                // panel and the meter frame both paint it in this draw.
                let analyzer = shell.update(cx, |shell, cx| {
                    let mut snapshot = (*shell.state.read(cx).snapshot).clone();
                    let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
                    let id = graph
                        .graph
                        .create_node(moiren_core::graph::NodeKind::SpectrumAnalyzer, 2)
                        .expect("analyzer card");
                    graph.nodes.push(crate::service::GraphNodeState {
                        id,
                        label: "Spectrum analyzer".into(),
                        gain: 1.,
                        pan: 0.,
                        mono: false,
                        swap: false,
                        mixer: Default::default(),
                        compressor: Default::default(),
                        builtin: crate::service::BuiltinState::SpectrumAnalyzer(
                            Default::default(),
                        ),
                        source: None,
                        output_endpoint_id: None,
                        tint: None,
                    });
                    shell.graph_view.node_positions.insert(id, (420., 230.));
                    let curve = demo_spectrum_curve();
                    shell.state.update(cx, |state, _| {
                        state.snapshot = std::sync::Arc::new(snapshot);
                        state.node_spectra.inject_curve(id, &curve);
                    });
                    cx.notify();
                    id
                });
                capture(cx, directory.join("graph-spectrum.png"));
                shell.update(cx, |shell, cx| {
                    let mut snapshot = (*shell.state.read(cx).snapshot).clone();
                    let graph = std::sync::Arc::make_mut(&mut snapshot.graph);
                    let _ = graph.graph.remove_node(analyzer);
                    graph.nodes.retain(|node| node.id != analyzer);
                    shell.graph_view.node_positions.remove(&analyzer);
                    shell
                        .state
                        .update(cx, |state, _| state.snapshot = std::sync::Arc::new(snapshot));
                    cx.notify();
                });
                let _ = window.update(cx, |_, window, _| window.resize(size(px(1440.), px(900.))));
                for (name, page) in [
                    ("node-inspector", Page::Graph),
                    ("mixer-inspector", Page::Mixer),
                    ("connection-inspector", Page::Graph),
                ] {
                    shell.update(cx, |shell, cx| {
                        shell.set_page(page, cx);
                        shell.graph_view.camera = Camera::default();
                        shell.chrome.sidebar_open = page == Page::Graph;
                        shell.chrome.inspector_open = true;
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
                        shell.selection.node = gain;
                        shell.selection.route = None;
                        if name == "connection-inspector" {
                            shell.selection.node = None;
                            let state = shell.state.read(cx);
                            shell.selection.route = state.snapshot.graph.graph.edges().first()
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
                    shell.chrome.inspector_open = true;
                    shell.chrome.sidebar_open = true;
                    shell.selection.node = shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .graph
                        .nodes()
                        .iter()
                        .find(|node| node.kind() == moiren_core::graph::NodeKind::Compressor)
                        .map(|node| node.id());
                    if let Some(key) = shell.selection.node {
                        shell.graph_view.node_positions.insert(key, (480., 370.));
                    }
                    shell.graph_view.camera = Camera::default();
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
                    shell.chrome.sidebar_open = false;
                    shell.chrome.inspector_open = false;
                    shell.selection.route = None;
                    shell.selection.node = None;
                    shell.graph_view.camera = Camera {
                        pan_x: 24.,
                        pan_y: 64.,
                        zoom: 1.,
                        adjusted: true,
                    };
                    let graph = &shell.state.read(cx).snapshot.graph.graph;
                    let ids = graph.nodes().iter().map(|node| node.id()).collect::<Vec<_>>();
                    for (index, id) in ids.iter().enumerate() {
                        shell.graph_view.node_positions.insert(
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
