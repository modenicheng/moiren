use super::*;
use crate::service::{
    DeviceCatalog, DeviceRow, InputSelection, JobPool, RoundBudget, RunLimit, ServiceContext,
    ServiceDriver, ServiceMailbox, SessionSpec,
};
use crate::ui::{Backend, BackendPhase};
use anyhow::{Result, ensure};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{LogicalPosition, LogicalSize, Model};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

#[derive(Default)]
struct Counts {
    starts: AtomicUsize,
    stops: AtomicUsize,
    refreshes: AtomicUsize,
}

#[test]
#[ignore = "opens a desktop window and live default audio, muted; run with --ignored --test-threads=1"]
fn desktop_automatically_runs_and_projects_muted_startup_without_clicking_start() -> Result<()> {
    use moiren_engine::{
        host::{EngineHost, EngineProject, ExecutionState, HostConfig, NodeConfig},
        node::{DefaultDeviceRole, EndpointSelector},
    };
    let mut project = EngineProject::default();
    let output = project
        .graph
        .create_node(moiren_core::graph::NodeKind::Sink, 2)?;
    project.nodes.insert(
        output,
        NodeConfig::Output(moiren_windows_audio::engine_backend::physical_output(
            EndpointSelector::FollowDefault(DefaultDeviceRole::Multimedia),
        )),
    );
    let other_owner = EngineHost::<f32>::open(
        HostConfig::default(),
        project,
        moiren_windows_audio::engine_backend::WindowsBackend::new(Default::default())?,
    )?;
    let other_deadline = Instant::now() + Duration::from_secs(10);
    while other_owner.observer().snapshot().execution != ExecutionState::Running {
        ensure!(
            Instant::now() < other_deadline,
            "shared engine fixture did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let runtime = start_service()?;
    let handle = runtime.handle();
    let mut owned = OwnedService(Some(runtime));
    let deadline = Instant::now() + Duration::from_secs(10);
    let snapshot = loop {
        if let Some(snapshot) = handle.try_snapshot() {
            ensure!(
                snapshot.phase != SessionPhase::Failed,
                "automatic startup failed: {:?}",
                snapshot.error
            );
            if snapshot.phase == SessionPhase::Running {
                break snapshot;
            }
            ensure!(
                Instant::now() < deadline,
                "automatic startup timed out: phase={:?}, catalog={:?}, error={:?}",
                snapshot.phase,
                snapshot.catalog_error,
                snapshot.error
            );
        }
        ensure!(Instant::now() < deadline, "automatic startup timed out");
        std::thread::sleep(Duration::from_millis(5));
    };
    let window = MainWindow::new()?;
    let tray = AppTray::new()?;
    window.window().set_size(LogicalSize::new(1600.0, 956.0));
    let state = Rc::new(RefCell::new(bindings::BindingState::default()));
    bindings::wire(&window, handle.clone(), state.clone());
    bindings::update(&state, &snapshot, &window, &tray);
    ensure!(
        window.global::<Backend>().get_phase() == BackendPhase::Running,
        "UI did not project Running"
    );
    ensure!(
        window.global::<Backend>().get_gain() == 0.0,
        "UI did not project startup mute"
    );
    window.show()?;
    let captured = Rc::new(RefCell::new(None));
    let result = captured.clone();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(250),
        move || {
            let evidence = (|| -> Result<()> {
                let window = weak.upgrade().unwrap();
                let image = window.window().take_snapshot()?;
                let path = std::env::var("MOIREN_STARTUP_SCREENSHOT")
                    .unwrap_or_else(|_| "target/automatic-startup.ppm".into());
                let mut file = std::fs::File::create(&path)?;
                write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
                for pixel in image.as_bytes().as_chunks::<4>().0 {
                    file.write_all(&pixel[..3])?;
                }
                println!("Automatic desktop startup screenshot: {path}");
                Ok(())
            })();
            *result.borrow_mut() = Some(evidence);
            let _ = slint::quit_event_loop();
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    window.hide()?;
    owned.finish()?;
    drop(other_owner);
    captured.borrow_mut().take().unwrap()?;
    ensure!(
        handle.try_snapshot().unwrap().phase == SessionPhase::Exited,
        "service did not close"
    );
    Ok(())
}

struct FixtureDriver {
    counts: Arc<Counts>,
    deadline: Option<Instant>,
    fail_start: bool,
    exiting: bool,
}
impl ServiceDriver for FixtureDriver {
    fn request(&mut self, context: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        match request {
            AppRequest::RefreshCatalog => {
                let refresh = self.counts.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
                if matches!(refresh, 2 | 4) {
                    context.catalog_error =
                        Some("Acceptance fixture: catalog refresh failed".into());
                } else {
                    context.catalog_error = None;
                    context.catalog = Arc::new(if refresh == 1 {
                        DeviceCatalog::default()
                    } else {
                        DeviceCatalog {
                            outputs: vec![DeviceRow {
                                endpoint_id: "acceptance-fixture-output".into(),
                                name: "Acceptance fixture output (no audio)".into(),
                            }],
                            default_output_endpoint_id: Some("acceptance-fixture-output".into()),
                            ..DeviceCatalog::default()
                        }
                    });
                }
            }
            AppRequest::Start(spec) => {
                let start = self.counts.starts.fetch_add(1, Ordering::SeqCst) + 1;
                context.core.start(spec).unwrap();
                self.fail_start = start == 1;
                self.deadline = Some(Instant::now() + Duration::from_millis(750));
            }
            AppRequest::Stop => {
                self.counts.stops.fetch_add(1, Ordering::SeqCst);
                context.core.stop().unwrap();
                self.deadline = Some(Instant::now() + Duration::from_millis(750));
            }
            AppRequest::SetGainPan { .. } => {}
        }
    }
    fn poll(&mut self, context: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
        if !self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }
        self.deadline = None;
        if self.exiting || context.core.phase() == SessionPhase::Stopping {
            if let Some(owner) = context.core.owner_generation() {
                assert!(context.core.owner_reaped(owner));
            }
        } else if self.fail_start {
            assert!(context.core.failed(
                context.core.generation(),
                "Acceptance fixture: session preparation failed".into()
            ));
        } else {
            let generation = context.core.generation();
            assert!(context.core.activated(generation));
            assert!(context.core.owner_started(generation));
        }
    }
    fn begin_exit(&mut self, _: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        self.deadline = Some(Instant::now() + Duration::from_millis(750));
    }
    fn is_drained(&self) -> bool {
        self.deadline.is_none()
    }
}

fn click_transport(window: &MainWindow) {
    let width = window.window().size().width as f32 / window.window().scale_factor();
    let position = LogicalPosition::new(width - 156.0, 22.0);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

#[test]
#[ignore = "controlled native labels with synthetic snapshots and no audio; run explicitly with --ignored --test-threads=1"]
fn controlled_native_active_labels_survive_candidate_changes() -> Result<()> {
    let window = MainWindow::new()?;
    let tray = AppTray::new()?;
    let (service_handle, mailbox) = ServiceMailbox::new(std::thread::current());
    let state = Rc::new(RefCell::new(bindings::BindingState::default()));
    bindings::wire(&window, service_handle.clone(), state.clone());
    let mut snapshot = (*service_handle.try_snapshot().unwrap()).clone();
    snapshot.catalog = Arc::new(DeviceCatalog {
        inputs: vec![
            DeviceRow {
                endpoint_id: "input-a".into(),
                name: "Input A".into(),
            },
            DeviceRow {
                endpoint_id: "input-b".into(),
                name: "Input B".into(),
            },
        ],
        outputs: vec![
            DeviceRow {
                endpoint_id: "output-a".into(),
                name: "Output A".into(),
            },
            DeviceRow {
                endpoint_id: "output-b".into(),
                name: "Output B".into(),
            },
        ],
        processes: vec![
            crate::service::ProcessRow {
                pid: 42,
                creation_time_100ns: 100,
                name: "Application A".into(),
            },
            crate::service::ProcessRow {
                pid: 42,
                creation_time_100ns: 200,
                name: "Application B".into(),
            },
        ],
        default_input_endpoint_id: Some("input-a".into()),
        default_output_endpoint_id: Some("output-a".into()),
        ..Default::default()
    });
    bindings::update(&state, &Arc::new(snapshot.clone()), &window, &tray);
    let backend = window.global::<Backend>();
    for (source, label, kind, icon, detail) in [
        (
            InputSelection::Physical {
                endpoint_id: "input-a".into(),
            },
            "Input A",
            "PHYSICAL INPUT",
            "mic",
            "WASAPI capture · stereo",
        ),
        (
            InputSelection::Process {
                pid: 42,
                creation_time_100ns: 100,
            },
            "Application A",
            "APPLICATION AUDIO",
            "app-window",
            "Process loopback · stereo",
        ),
        (
            InputSelection::Tone {
                frequency_hz: 880.0,
            },
            "880 Hz tone",
            "TEST TONE",
            "radio",
            "880 Hz · 7 seconds",
        ),
    ] {
        snapshot.phase = SessionPhase::Running;
        snapshot.desired = Some(SessionSpec {
            source,
            output_endpoint_id: "output-a".into(),
            limit: RunLimit::Seconds(7),
            gain: 0.01,
            pan: 0.0,
            max_block_frames: 256,
        });
        for mode in 0..3 {
            backend.set_source_mode(mode);
            backend.invoke_select_input(1);
            backend.invoke_select_process(1);
            backend.invoke_select_output(1);
            bindings::update(&state, &Arc::new(snapshot.clone()), &window, &tray);
            ensure!(
                backend.get_source_label() == label,
                "candidate mode {mode} replaced active {label}: {}",
                backend.get_source_label()
            );
            ensure!(
                backend.get_output_label() == "Output A",
                "candidate output replaced active output"
            );
            ensure!(
                backend.get_inspector_source() == label
                    && backend.get_inspector_destination() == "Output A",
                "inspector attributed the candidate"
            );
            let nodes = backend.get_graph_nodes();
            let source_node = nodes
                .row_data(backend.get_graph_input_index() as usize)
                .unwrap();
            ensure!(
                source_node.name == label
                    && source_node.type_label == kind
                    && source_node.icon == icon
                    && source_node.detail == detail,
                "graph source attribution: {source_node:?}"
            );
            let output_node = nodes
                .row_data(backend.get_graph_output_index() as usize)
                .unwrap();
            ensure!(
                output_node.name == "Output A",
                "graph output attributed the candidate"
            );
            ensure!(
                backend.get_input_index() == 1
                    && backend.get_process_index() == 1
                    && backend.get_output_index() == 1
                    && backend.get_source_mode() == mode,
                "candidate selectors were overwritten"
            );
        }
    }
    let mut missing = (*snapshot.catalog).clone();
    missing.outputs.remove(0);
    missing.inputs.remove(0);
    missing.processes.remove(0);
    snapshot.catalog = Arc::new(missing);
    snapshot.desired.as_mut().unwrap().source = InputSelection::Physical {
        endpoint_id: "input-a".into(),
    };
    bindings::update(&state, &Arc::new(snapshot.clone()), &window, &tray);
    ensure!(
        backend.get_source_label().contains("Unavailable")
            && backend.get_source_label().contains("input-a"),
        "missing active input substituted the candidate"
    );
    ensure!(
        backend.get_output_label().contains("Unavailable")
            && backend.get_output_label().contains("output-a"),
        "missing active output substituted the candidate"
    );
    ensure!(
        backend.get_engine_clock() == backend.get_output_label(),
        "clock hid missing active output"
    );
    snapshot.desired.as_mut().unwrap().source = InputSelection::Process {
        pid: 42,
        creation_time_100ns: 100,
    };
    bindings::update(&state, &Arc::new(snapshot.clone()), &window, &tray);
    ensure!(
        backend.get_source_label().contains("Unavailable")
            && !backend.get_source_label().contains("Application B"),
        "process PID reuse substituted a different process"
    );
    snapshot.phase = SessionPhase::Idle;
    snapshot.desired = None;
    backend.set_source_mode(1);
    bindings::update(&state, &Arc::new(snapshot), &window, &tray);
    ensure!(
        backend.get_source_label() == "Application B" && backend.get_output_label() == "Output B",
        "idle candidate labels missing"
    );
    backend.invoke_start_monitor(true, 0);
    let Some(AppRequest::Start(next)) = mailbox.try_request() else {
        anyhow::bail!("candidate Start missing")
    };
    ensure!(
        next.source
            == InputSelection::Process {
                pid: 42,
                creation_time_100ns: 200
            }
            && next.output_endpoint_id == "output-b",
        "next Start did not use candidate B"
    );
    Ok(())
}

#[test]
#[ignore = "renders synthetic equal RMS/peak meters in a native window without audio; run explicitly with --ignored --test-threads=1"]
fn controlled_native_peak_marker_overlays_equal_rms() -> Result<()> {
    let window = MainWindow::new()?;
    window.window().set_size(LogicalSize::new(1600.0, 956.0));
    let backend = window.global::<Backend>();
    backend.set_view(1);
    backend.set_route_meter_l(0.6);
    backend.set_route_meter_r(0.6);
    window.show()?;
    let result = Rc::new(RefCell::new(None));
    let recorded = result.clone();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(200),
        move || {
            let evidence = (|| -> Result<usize> {
                let window = weak.upgrade().unwrap();
                let without_peak = window.window().take_snapshot()?;
                let backend = window.global::<Backend>();
                backend.set_route_peak_l(0.6);
                backend.set_route_peak_r(0.6);
                let image = window.window().take_snapshot()?;
                ensure!(
                    image.width() == without_peak.width()
                        && image.height() == without_peak.height(),
                    "snapshot geometry changed"
                );
                let overlaid = without_peak
                    .as_bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(image.as_bytes().as_chunks::<4>().0)
                    .filter(|(before, after)| {
                        before[..3] == [177, 160, 223] && after[..3] == [224, 131, 122]
                    })
                    .count();
                let directory =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/native-peak-marker");
                std::fs::create_dir_all(&directory)?;
                let mut file = std::fs::File::create(directory.join("peak-over-rms.ppm"))?;
                write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
                for pixel in image.as_bytes().as_chunks::<4>().0 {
                    file.write_all(&pixel[..3])?;
                }
                ensure!(
                    overlaid >= 14,
                    "peak marker covered by RMS fill: {overlaid} overlaid pixels"
                );
                println!("equal RMS/peak native render: {overlaid} violet-to-red pixels");
                Ok(overlaid)
            })();
            *recorded.borrow_mut() = Some(evidence);
            let _ = slint::quit_event_loop();
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    window.hide()?;
    result.borrow_mut().take().unwrap()?;
    Ok(())
}

struct Acceptance {
    stage: usize,
    stage_since: Instant,
    started: Instant,
    counts: Arc<Counts>,
    records: Vec<serde_json::Value>,
    directory: PathBuf,
    error: Option<String>,
}
impl Acceptance {
    fn capture(
        &mut self,
        window: &MainWindow,
        snapshot: &crate::service::AppSnapshot,
    ) -> Result<()> {
        let name = format!("{:02}-{:?}", self.stage, snapshot.phase);
        let image = window.window().take_snapshot()?;
        let mut file = std::fs::File::create(self.directory.join(format!("{name}.ppm")))?;
        write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
        let mut row = Vec::with_capacity(image.width() as usize * 3);
        for pixels in image.as_bytes().chunks_exact(image.width() as usize * 4) {
            row.clear();
            for pixel in pixels.as_chunks::<4>().0 {
                row.extend_from_slice(&pixel[..3]);
            }
            file.write_all(&row)?;
        }
        let backend = window.global::<Backend>();
        self.records.push(serde_json::json!({
            "stage": self.stage, "phase": format!("{:?}", snapshot.phase),
            "ui_phase": format!("{:?}", backend.get_phase()),
            "transport_ready": backend.get_transport_ready(),
            "gain": backend.get_gain(),
            "pan": backend.get_pan(),
            "output_levels": backend.get_output_levels().as_str(),
            "meter_status": backend.get_meter_status().as_str(),
            "left_peak": backend.get_route_peak_l(),
            "right_peak": backend.get_route_peak_r(),
            "catalog_error": backend.get_catalog_error().as_str(),
            "session_error": backend.get_error_text().as_str(),
            "input_index": backend.get_input_index(),
            "output_index": backend.get_output_index(),
            "process_index": backend.get_process_index(),
            "starts": self.counts.starts.load(Ordering::SeqCst),
            "stops": self.counts.stops.load(Ordering::SeqCst),
            "refreshes": self.counts.refreshes.load(Ordering::SeqCst),
            "snapshot": format!("{name}.ppm"),
        }));
        self.stage += 1;
        self.stage_since = Instant::now();
        Ok(())
    }
    fn tick(
        &mut self,
        window: &MainWindow,
        handle: &crate::service::AppHandle,
        snapshot: &crate::service::AppSnapshot,
    ) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(25),
            "native fixture timed out at stage {}",
            self.stage
        );
        if self.stage_since.elapsed() < Duration::from_millis(200) {
            return Ok(false);
        }
        let backend = window.global::<Backend>();
        match self.stage {
            0 if self.counts.refreshes.load(Ordering::SeqCst) == 1 => {
                ensure!(
                    backend.get_inputs().row_count() == 0
                        && backend.get_outputs().row_count() == 0
                        && backend.get_processes().row_count() == 0,
                    "catalog should be empty"
                );
                ensure!(
                    backend.get_input_index() == -1
                        && backend.get_output_index() == -1
                        && backend.get_process_index() == -1,
                    "empty catalog selection"
                );
                ensure!(
                    !backend.get_tone_valid() && !backend.get_monitor_valid(),
                    "empty catalog may not start"
                );
                click_transport(window);
                backend.invoke_start_tone(440.0, 10);
                backend.invoke_start_monitor(true, 0);
                ensure!(
                    self.counts.starts.load(Ordering::SeqCst) == 0,
                    "invalid start dispatched"
                );
                self.capture(window, snapshot)?;
                backend.invoke_refresh_catalog();
            }
            1 if !backend.get_catalog_error().is_empty() => {
                ensure!(
                    snapshot.phase == SessionPhase::Idle,
                    "catalog error changed phase"
                );
                self.capture(window, snapshot)?;
                backend.invoke_refresh_catalog();
            }
            2 if backend.get_tone_valid() && backend.get_catalog_error().is_empty() => {
                ensure!(
                    backend.get_output_index() == 0,
                    "default identity selection"
                );
                backend.set_source_mode(2);
                backend.set_view(1);
                backend.invoke_set_gain_pan(0.005, 0.0);
                self.capture(window, snapshot)?;
                click_transport(window);
                click_transport(window);
            }
            3 if snapshot.phase == SessionPhase::Starting => {
                ensure!(
                    backend.get_gain() <= 0.006,
                    "controlled Starting gain changed: UI={} session={:?}",
                    backend.get_gain(),
                    snapshot.desired.as_ref().map(|spec| spec.gain)
                );
                ensure!(
                    !backend.get_transport_ready(),
                    "Starting action must be disabled"
                );
                ensure!(
                    self.counts.starts.load(Ordering::SeqCst) == 1,
                    "duplicate pending start"
                );
                self.capture(window, snapshot)?;
                click_transport(window);
                click_transport(window);
            }
            4 if snapshot.phase == SessionPhase::Failed => {
                ensure!(
                    self.counts.starts.load(Ordering::SeqCst) == 1,
                    "duplicate Starting start"
                );
                ensure!(
                    !backend.get_error_text().is_empty()
                        && backend.get_phase() == BackendPhase::Failed,
                    "failure missing from UI"
                );
                self.capture(window, snapshot)?;
                click_transport(window);
            }
            5 if snapshot.phase == SessionPhase::Starting => {
                self.capture(window, snapshot)?;
            }
            6 if snapshot.phase == SessionPhase::Running => {
                ensure!(
                    backend.get_gain() <= 0.006,
                    "controlled Running gain changed: UI={} session={:?}",
                    backend.get_gain(),
                    snapshot.desired.as_ref().map(|spec| spec.gain)
                );
                ensure!(
                    backend.get_error_text().is_empty(),
                    "new session did not clear failure"
                );
                self.capture(window, snapshot)?;
                backend.invoke_refresh_catalog();
            }
            7 if snapshot.phase == SessionPhase::Running
                && !backend.get_catalog_error().is_empty() =>
            {
                ensure!(
                    backend.get_error_text().is_empty() && backend.get_output_index() == 0,
                    "catalog failure corrupted running session"
                );
                self.capture(window, snapshot)?;
                backend.invoke_refresh_catalog();
            }
            8 if snapshot.phase == SessionPhase::Running
                && backend.get_catalog_error().is_empty() =>
            {
                self.capture(window, snapshot)?;
                click_transport(window);
                click_transport(window);
            }
            9 if snapshot.phase == SessionPhase::Stopping => {
                ensure!(
                    !backend.get_transport_ready(),
                    "Stopping action must be disabled"
                );
                ensure!(
                    self.counts.stops.load(Ordering::SeqCst) == 1,
                    "duplicate pending stop"
                );
                self.capture(window, snapshot)?;
                click_transport(window);
                click_transport(window);
            }
            10 if snapshot.phase == SessionPhase::Idle => {
                ensure!(
                    self.counts.stops.load(Ordering::SeqCst) == 1,
                    "duplicate Stopping stop"
                );
                self.capture(window, snapshot)?;
                click_transport(window);
            }
            11 if snapshot.phase == SessionPhase::Running => {
                self.capture(window, snapshot)?;
                handle.request_exit();
            }
            12 if snapshot.phase == SessionPhase::Exiting => {
                ensure!(
                    !backend.get_transport_ready(),
                    "Exiting action must be disabled"
                );
                self.capture(window, snapshot)?;
            }
            13 if snapshot.phase == SessionPhase::Exited => {
                self.capture(window, snapshot)?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}

#[test]
#[ignore = "opens a native acceptance window; run explicitly with --ignored --test-threads=1"]
fn controlled_native_catalog_and_seven_phases() -> Result<()> {
    let counts = Arc::new(Counts::default());
    let service = ServiceRuntime::spawn_with_driver(FixtureDriver {
        counts: counts.clone(),
        deadline: None,
        fail_start: false,
        exiting: false,
    })?;
    let handle = service.handle();
    let mut owned = OwnedService(Some(service));
    let window = MainWindow::new()?;
    let tray = AppTray::new()?;
    let state = Rc::new(RefCell::new(bindings::BindingState::default()));
    bindings::wire(&window, handle.clone(), state.clone());
    window.window().set_size(LogicalSize::new(1600.0, 956.0));
    window.show()?;
    tray.show()?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/native-acceptance");
    std::fs::create_dir_all(&directory)?;
    let acceptance = Rc::new(RefCell::new(Acceptance {
        stage: 0,
        stage_since: Instant::now(),
        started: Instant::now(),
        counts: counts.clone(),
        records: Vec::new(),
        directory: directory.clone(),
        error: None,
    }));
    let observed = acceptance.clone();
    let weak = window.as_weak();
    let weak_tray = tray.as_weak();
    let service_handle = handle.clone();
    bindings::dispatch(&state, &handle, AppRequest::RefreshCatalog);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            if let (Some(window), Some(tray), Some(snapshot)) = (
                weak.upgrade(),
                weak_tray.upgrade(),
                service_handle.try_snapshot(),
            ) {
                bindings::update(&state, &snapshot, &window, &tray);
                bindings::update_signals(
                    &state,
                    &service_handle,
                    &snapshot,
                    &window,
                    &WindowPresentation {
                        visible: true,
                        minimized: false,
                        page: 0,
                    },
                );
                let result = observed
                    .borrow_mut()
                    .tick(&window, &service_handle, &snapshot);
                match result {
                    Ok(true) => {
                        let _ = slint::quit_event_loop();
                    }
                    Err(error) => {
                        observed.borrow_mut().error = Some(error.to_string());
                        service_handle.request_exit();
                        let _ = slint::quit_event_loop();
                    }
                    Ok(false) => {}
                }
            }
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    owned.finish()?;
    let acceptance = acceptance.borrow();
    std::fs::write(
        directory.join("evidence.json"),
        serde_json::to_vec_pretty(&acceptance.records)?,
    )?;
    ensure!(
        acceptance.error.is_none(),
        "{}",
        acceptance.error.as_deref().unwrap_or("")
    );
    ensure!(acceptance.stage == 14, "incomplete native stages");
    ensure!(
        counts.starts.load(Ordering::SeqCst) == 3
            && counts.stops.load(Ordering::SeqCst) == 1
            && counts.refreshes.load(Ordering::SeqCst) == 5,
        "unexpected native request counts"
    );
    ensure!(
        handle.try_snapshot().unwrap().phase == SessionPhase::Exited,
        "service did not join"
    );
    Ok(())
}

#[test]
#[ignore = "plays a finite low-gain tone on the default output; run explicitly with --ignored --test-threads=1"]
fn native_audio_stop_and_quit_join_owners() -> Result<()> {
    let service = start_service()?;
    let handle = service.handle();
    let mut owned = OwnedService(Some(service));
    let window = MainWindow::new()?;
    let tray = AppTray::new()?;
    let state = Rc::new(RefCell::new(bindings::BindingState::default()));
    bindings::wire(&window, handle.clone(), state.clone());
    let exit_handle = handle.clone();
    window.on_exit_app(move || exit_handle.request_exit());
    window.window().set_size(LogicalSize::new(1180.0, 760.0));
    window.show()?;
    tray.show()?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/native-audio-acceptance");
    std::fs::create_dir_all(&directory)?;
    let acceptance = Rc::new(RefCell::new(Acceptance {
        stage: 0,
        stage_since: Instant::now(),
        started: Instant::now(),
        counts: Arc::default(),
        records: Vec::new(),
        directory: directory.clone(),
        error: None,
    }));
    let exit_started = Rc::new(RefCell::new(None::<Instant>));
    let recorded_exit = exit_started.clone();
    let mut hidden = false;
    let mut analysis_ticket =
        None::<crate::service::SubscriptionTicket<crate::service::LevelTopic>>;
    let mut analysis = None::<moiren_signal::Subscriber<crate::service::LevelTopic>>;
    let mut analysis_baseline = None::<(u64, u64)>;
    let mut first_window = None::<moiren_engine::meter::signal::AudioWindow>;
    let mut restarted_window = None::<moiren_engine::meter::signal::AudioWindow>;
    let mut analysis_message = None::<(
        moiren_engine::meter::signal::AudioWindow,
        moiren_signal::PublicationMeta,
        [moiren_engine::meter::signal::ChannelLevel; 2],
    )>;
    let mut recorded_running = false;
    let observed = acceptance.clone();
    let weak = window.as_weak();
    let weak_tray = tray.as_weak();
    let service_handle = handle.clone();
    bindings::dispatch(&state, &handle, AppRequest::RefreshCatalog);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            if let (Some(window), Some(tray), Some(snapshot)) =
                (weak.upgrade(), weak_tray.upgrade(), service_handle.try_snapshot())
            {
                bindings::update(&state, &snapshot, &window, &tray);
                bindings::update_signals(
                    &state,
                    &service_handle,
                    &snapshot,
                    &window,
                    &WindowPresentation {
                        visible: !hidden,
                        minimized: false,
                        page: 1,
                    },
                );
                let result = (|| -> Result<bool> {
                    let mut acceptance = observed.borrow_mut();
                    ensure!(
                        acceptance.started.elapsed() < Duration::from_secs(20),
                        "native audio fixture timed out at stage {}: phase={:?} gain={} desired={:?} subscribers={} attempts={} meter={} output_regions={:?}",
                        acceptance.stage, snapshot.phase, window.global::<Backend>().get_gain(), snapshot.desired,
                        snapshot.signal_topics.tone_output.business_subscribers(), snapshot.signal_topics.tone_output.stats().attempts,
                        window.global::<Backend>().get_meter_status(), window.get_output_meter_regions().iter().collect::<Vec<_>>()
                    );
                    ensure!(
                        snapshot.phase != SessionPhase::Failed,
                        "native session failed: {:?}",
                        snapshot.error
                    );
                    let backend = window.global::<Backend>();
                    if acceptance.stage == 1 && snapshot.phase == SessionPhase::Running && !recorded_running {
                        acceptance.records.push(serde_json::json!({
                            "stage": 1, "running_diagnostic": true,
                            "gain": backend.get_gain(), "view": backend.get_view(),
                            "subscribers": snapshot.signal_topics.tone_output.business_subscribers(),
                            "attempts": snapshot.signal_topics.tone_output.stats().attempts,
                            "publisher": format!("{:?}", snapshot.signal_topics.tone_output.status()),
                            "meter_status": backend.get_meter_status().as_str(),
                            "body_clip": format!("{:?}", window.get_meter_body_clip()),
                            "shell": [window.get_shell_width(), window.get_shell_height()],
                            "output_regions": format!("{:?}", window.get_output_meter_regions().iter().collect::<Vec<_>>()),
                        }));
                        recorded_running = true;
                    }
                    if let Some(subscriber) = analysis.as_mut()
                        && let Some(message) = subscriber.read_latest()
                    {
                        if let Some(first) = first_window {
                            if message.header().timeline_epoch != first.timeline_epoch
                                && restarted_window.is_none()
                            {
                                restarted_window = Some(*message.header());
                            }
                        } else {
                            first_window = Some(*message.header());
                        }
                        analysis_message = Some((*message.header(), message.meta(), message.elements().try_into().map_err(|_| anyhow::anyhow!("analysis window is not stereo"))?));
                    }
                    if acceptance.stage_since.elapsed() < Duration::from_millis(100) {
                        return Ok(false);
                    }
                    match acceptance.stage {
                        0 if backend.get_tone_valid() => {
                            if analysis.is_none() {
                                if analysis_ticket.is_none() {
                                    analysis_ticket = Some(service_handle.subscribe_topic(snapshot.signal_topics.tone_output.clone(), Default::default()).map_err(|error| anyhow::anyhow!("analysis admission: {error:?}"))?);
                                }
                                analysis = analysis_ticket.as_mut().unwrap().try_take()?;
                                if analysis.is_none() { return Ok(false) }
                            }
                            backend.set_source_mode(2);
                            backend.set_view(1);
                            backend.invoke_set_gain_pan(0.005, 0.0);
                            acceptance.capture(&window, &snapshot)?;
                            backend.invoke_start_tone(440.0, 10);
                        }
                        1 | 3 | 5
                            if snapshot.phase == SessionPhase::Running
                                && backend.get_route_peak_l() > 0.0
                                && backend.get_route_peak_r() > 0.0 =>
                        {
                            ensure!(backend.get_gain() <= 0.006, "native fixture gain changed at stage {}: UI={} session={:?} reply={:?}", acceptance.stage, backend.get_gain(), snapshot.desired.as_ref().map(|spec| spec.gain), snapshot.control.last_result);
                            let stage = acceptance.stage;
                            if stage == 1 {
                                let Some((_, meta, _)) = analysis_message else { return Ok(false) };
                                analysis_baseline = Some((meta.sequence, meta.activation_generation));
                                ensure!(snapshot.signal_topics.tone_output.business_subscribers() == 2, "UI and analysis demands missing");
                            }
                            if stage == 5 {
                                let Some(second) = restarted_window else { return Ok(false) };
                                let first = first_window.unwrap();
                                let (_, meta, _) = analysis_message.unwrap();
                                let (sequence, generation) = analysis_baseline.unwrap();
                                ensure!(second.timeline_epoch != first.timeline_epoch, "retained subscriber confused restarted frame domain");
                                ensure!(meta.sequence > sequence && meta.activation_generation == generation, "retained subscriber continuity lost across restart");
                                ensure!(snapshot.signal_topics.tone_output.business_subscribers() == 2, "restart did not retain independent analysis demand");
                                acceptance.records.push(serde_json::json!({
                                    "stage": 5, "retained_analysis_restart": true,
                                    "first_timeline_epoch": first.timeline_epoch,
                                    "second_timeline_epoch": second.timeline_epoch,
                                    "first_frame_start": first.frame_start, "first_frame_end": first.frame_end,
                                    "second_frame_start": second.frame_start, "second_frame_end": second.frame_end,
                                    "generation": generation, "sequence": meta.sequence,
                                }));
                                analysis = None;
                                analysis_ticket = None;
                            }
                            acceptance.capture(&window, &snapshot)?;
                            if stage == 1 {
                                bindings::hide_signals(&state, &window);
                                window.hide()?;
                                hidden = true;
                            } else if stage == 3 {
                                backend.invoke_stop();
                            } else {
                                *recorded_exit.borrow_mut() = Some(Instant::now());
                                window.invoke_exit_app();
                            }
                        }
                        2 if snapshot.phase == SessionPhase::Running => {
                            let topic = &snapshot.signal_topics.tone_output;
                            ensure!(hidden && topic.business_subscribers() == 1, "hide did not release only UI demand");
                            let Some((header, meta, levels)) = analysis_message else { return Ok(false) };
                            let (sequence, generation) = analysis_baseline.unwrap();
                            if meta.sequence <= sequence { return Ok(false) }
                            ensure!(meta.activation_generation == generation && topic.demand_snapshot().generation == generation, "hidden analysis generation changed");
                            ensure!(header.frame_end - header.frame_start == 1536, "incomplete audio window");
                            for level in &levels {
                                ensure!((level.peak - 0.005).abs() < 0.00001 && (level.rms - 0.005 / 2.0_f32.sqrt()).abs() < 0.0001, "incorrect native analysis level");
                            }
                            acceptance.records.push(serde_json::json!({
                                "stage": 2, "phase": "Running", "window_hidden": true,
                                "subscribers": topic.business_subscribers(), "generation": generation,
                                "previous_sequence": sequence, "sequence": meta.sequence,
                                "levels": levels.iter().map(|level| [level.peak, level.rms]).collect::<Vec<_>>(),
                            }));
                            acceptance.stage = 3;
                            acceptance.stage_since = Instant::now();
                            window.show()?;
                            hidden = false;
                        }
                        4 if snapshot.phase == SessionPhase::Idle => {
                            ensure!(snapshot.signal_topics.tone_output.business_subscribers() == 1, "Stop lost retained analysis demand");
                            acceptance.capture(&window, &snapshot)?;
                            backend.invoke_start_tone(440.0, 10);
                        }
                        6 if snapshot.phase == SessionPhase::Exited => {
                            acceptance.capture(&window, &snapshot)?;
                            return Ok(true);
                        }
                        _ => {}
                    }
                    Ok(false)
                })();
                match result {
                    Ok(true) => {
                        let _ = slint::quit_event_loop();
                    }
                    Err(error) => {
                        observed.borrow_mut().error = Some(error.to_string());
                        service_handle.request_exit();
                        let _ = slint::quit_event_loop();
                    }
                    Ok(false) => {}
                }
            }
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    owned.finish()?;
    let acceptance = acceptance.borrow();
    let exit_ms = exit_started
        .borrow()
        .map(|started| started.elapsed().as_millis());
    std::fs::write(
        directory.join("evidence.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "records": acceptance.records, "exit_to_join_ms": exit_ms,
            "service_joined": handle.try_snapshot().unwrap().phase == SessionPhase::Exited,
            "default_output": handle.try_snapshot().unwrap().catalog.default_output_endpoint_id,
            "software_gain": 0.005,
            "error": acceptance.error,
        }))?,
    )?;
    ensure!(
        acceptance.error.is_none(),
        "{}",
        acceptance.error.as_deref().unwrap_or("")
    );
    ensure!(acceptance.stage == 7, "native audio acceptance incomplete");
    ensure!(
        exit_ms.is_some_and(|elapsed| elapsed < 2000),
        "native cleanup exceeded bound"
    );
    Ok(())
}
