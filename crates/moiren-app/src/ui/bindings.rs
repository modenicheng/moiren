use super::studio::{StudioModels, StudioState};
use super::{AudioEndpoint, Backend, BackendPhase, MainWindow, ProcessSource, SelectOption};
use crate::service::*;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Selected identities outlive row positions. A vanished identity remains
/// selected but invalid until the user deliberately chooses another row.
#[derive(Default, Debug)]
pub struct SelectionState {
    pub input_id: Option<String>,
    pub output_id: Option<String>,
    pub process_key: Option<(u32, u64)>,
    input_initialized: bool,
    output_initialized: bool,
}
impl SelectionState {
    pub fn refresh(&mut self, catalog: &DeviceCatalog) {
        if !self.input_initialized && !catalog.inputs.is_empty() {
            self.input_id = catalog.default_input_endpoint_id.clone();
            self.input_initialized = true;
        }
        if !self.output_initialized && !catalog.outputs.is_empty() {
            self.output_id = catalog.default_output_endpoint_id.clone();
            self.output_initialized = true;
        }
    }
    pub fn input_index(&self, catalog: &DeviceCatalog) -> i32 {
        index(&catalog.inputs, self.input_id.as_deref())
    }
    pub fn output_index(&self, catalog: &DeviceCatalog) -> i32 {
        index(&catalog.outputs, self.output_id.as_deref())
    }
    pub fn process_index(&self, catalog: &DeviceCatalog) -> i32 {
        catalog
            .processes
            .iter()
            .position(|p| Some((p.pid, p.creation_time_100ns)) == self.process_key)
            .map_or(-1, |i| i as i32)
    }
    pub fn select_process(&mut self, catalog: &DeviceCatalog, row: i32) {
        self.process_key = usize::try_from(row)
            .ok()
            .and_then(|i| catalog.processes.get(i))
            .map(|p| (p.pid, p.creation_time_100ns));
    }
    pub fn process_source(&self, catalog: &DeviceCatalog) -> Option<InputSelection> {
        let i = usize::try_from(self.process_index(catalog)).ok()?;
        let p = catalog.processes.get(i)?;
        Some(InputSelection::Process {
            pid: p.pid,
            creation_time_100ns: p.creation_time_100ns,
        })
    }
}
/// Inspector identity is separate from transport routing and row positions.
#[derive(Default, Debug)]
pub struct DetailSelection {
    capture: bool,
    endpoint_id: Option<String>,
}
impl DetailSelection {
    pub fn select(&mut self, capture: bool, endpoint_id: String) {
        self.capture = capture;
        self.endpoint_id = Some(endpoint_id);
    }
    pub fn endpoint<'a>(&self, catalog: &'a DeviceCatalog) -> Option<&'a DeviceRow> {
        let rows = if self.capture {
            &catalog.inputs
        } else {
            &catalog.outputs
        };
        rows.iter()
            .find(|row| Some(&row.endpoint_id) == self.endpoint_id.as_ref())
    }
    fn initialize(&mut self, selection: &SelectionState) {
        if self.endpoint_id.is_none() {
            if let Some(id) = selection.output_id.as_ref() {
                self.select(false, id.clone());
            } else if let Some(id) = selection.input_id.as_ref() {
                self.select(true, id.clone());
            }
        }
    }
}
fn index(rows: &[DeviceRow], id: Option<&str>) -> i32 {
    rows.iter()
        .position(|row| Some(row.endpoint_id.as_str()) == id)
        .map_or(-1, |i| i as i32)
}
fn endpoint_at(rows: &[DeviceRow], row: i32) -> Option<String> {
    usize::try_from(row)
        .ok()
        .and_then(|i| rows.get(i))
        .map(|r| r.endpoint_id.clone())
}

#[derive(Default)]
pub(super) struct BindingState {
    selection: SelectionState,
    detail: DetailSelection,
    catalog: Arc<DeviceCatalog>,
    pending_request_id: Option<RequestId>,
    transport_pending: bool,
    dispatch_error: Option<String>,
    studio: StudioState,
    studio_models: StudioModels,
    snapshot: Option<Arc<AppSnapshot>>,
    signals: super::meters::UiSignals,
}
impl BindingState {
    fn observe_snapshot(&mut self, snapshot: &Arc<AppSnapshot>) {
        if self
            .pending_request_id
            .is_some_and(|id| snapshot.consumed_request_id >= id)
        {
            self.pending_request_id = None;
            self.transport_pending = false;
        }
        if self.pending_request_id.is_none()
            && self
                .snapshot
                .as_ref()
                .is_none_or(|previous| previous.generation != snapshot.generation)
            && let Some(spec) = &snapshot.desired
        {
            self.studio
                .set_parameters(spec.gain as f32, spec.pan as f32);
        }
    }
}
pub(super) type SharedState = Rc<RefCell<BindingState>>;
pub(super) fn hide_signals(state: &SharedState, window: &MainWindow) {
    state.borrow_mut().signals.hide();
    super::meters::clear_backend(window);
}
pub(super) fn update_signals(
    state: &SharedState,
    handle: &AppHandle,
    snapshot: &AppSnapshot,
    window: &MainWindow,
    presentation: &super::WindowPresentation,
) {
    let display = state
        .borrow_mut()
        .signals
        .update(handle, snapshot, window, presentation);
    display.apply(window);
}

/// Create owned model data under a short borrow, then release it before any
/// reactive Slint setter. Setters may synchronously reenter host callbacks.
pub(super) fn update(
    state: &SharedState,
    snapshot: &Arc<AppSnapshot>,
    window: &MainWindow,
    tray: &super::AppTray,
) {
    let (catalog_changed, catalog, input, output, process, detail, detail_capture, pending, error) = {
        let mut state = state.borrow_mut();
        let changed = !Arc::ptr_eq(&state.catalog, &snapshot.catalog);
        if changed {
            state.selection.refresh(&snapshot.catalog);
            state.catalog = snapshot.catalog.clone();
            let BindingState {
                detail, selection, ..
            } = &mut *state;
            detail.initialize(selection);
        }
        state.observe_snapshot(snapshot);
        state.snapshot = Some(snapshot.clone());
        (
            changed,
            state.catalog.clone(),
            state.selection.input_index(&state.catalog),
            state.selection.output_index(&state.catalog),
            state.selection.process_index(&state.catalog),
            state
                .detail
                .endpoint(&state.catalog)
                .map(|row| {
                    endpoint(
                        row,
                        if state.detail.capture {
                            "mic"
                        } else {
                            "speaker"
                        },
                        None,
                    )
                })
                .unwrap_or_else(|| AudioEndpoint {
                    name: "No Device".into(),
                    ..Default::default()
                }),
            state.detail.capture,
            state.transport_pending,
            state.dispatch_error.clone(),
        )
    };
    for backend in [window.global::<Backend>(), tray.global::<Backend>()] {
        if catalog_changed {
            backend.set_inputs(endpoint_model(
                &catalog.inputs,
                "mic",
                catalog.default_input_endpoint_id.as_deref(),
            ));
            backend.set_outputs(endpoint_model(
                &catalog.outputs,
                "speaker",
                catalog.default_output_endpoint_id.as_deref(),
            ));
            backend.set_processes(ModelRc::new(VecModel::from(
                catalog
                    .processes
                    .iter()
                    .map(|p| ProcessSource {
                        key: format!("{}:{}", p.pid, p.creation_time_100ns).into(),
                        name: p.name.clone().into(),
                        detail: format!("PID {}", p.pid).into(),
                    })
                    .collect::<Vec<_>>(),
            )));
            backend.set_input_options(options(&catalog.inputs));
            backend.set_output_options(options(&catalog.outputs));
            backend.set_process_options(ModelRc::new(VecModel::from(
                catalog
                    .processes
                    .iter()
                    .map(|p| SelectOption {
                        text: p.name.clone().into(),
                        detail: format!("PID {}", p.pid).into(),
                    })
                    .collect::<Vec<_>>(),
            )));
        }
        backend.set_detail_endpoint(detail.clone());
        backend.set_detail_capture(detail_capture);
        backend.set_input_index(input);
        backend.set_output_index(output);
        backend.set_process_index(process);
        backend.set_tone_valid(output >= 0);
        backend.set_monitor_valid(
            output >= 0
                && if backend.get_source_mode() == 0 {
                    input >= 0
                } else {
                    process >= 0
                },
        );
        backend.set_phase(match snapshot.phase {
            SessionPhase::Idle => BackendPhase::Idle,
            SessionPhase::Starting => BackendPhase::Starting,
            SessionPhase::Running => BackendPhase::Running,
            SessionPhase::Stopping => BackendPhase::Stopping,
            SessionPhase::Failed => BackendPhase::Failed,
            SessionPhase::Exiting => BackendPhase::Exiting,
            SessionPhase::Exited => BackendPhase::Exited,
        });
        backend.set_request_pending(pending);
        backend.set_error_text(
            error
                .as_deref()
                .or(snapshot.error.as_deref())
                .unwrap_or("")
                .into(),
        );
        backend.set_catalog_error(snapshot.catalog_error.as_deref().unwrap_or("").into());
        backend.set_tone_duration(match snapshot.desired.as_ref() {
            Some(SessionSpec {
                source: InputSelection::Tone { .. },
                limit: RunLimit::Seconds(limit),
                ..
            }) => i32::from(*limit),
            _ => 0,
        });
        backend.set_tone_progress(match snapshot.desired.as_ref() {
            Some(SessionSpec {
                source: InputSelection::Tone { .. },
                limit: RunLimit::Seconds(limit),
                ..
            }) if snapshot.phase == SessionPhase::Running => {
                (snapshot.elapsed.as_secs_f64() / f64::from(*limit)).clamp(0.0, 1.0) as f32
            }
            _ => 0.0,
        });
        let secs = snapshot.elapsed.as_secs();
        backend.set_elapsed(if secs == 0 && snapshot.phase != SessionPhase::Running {
            "—".into()
        } else {
            format!("{:02}:{:02}", secs / 60, secs % 60).into()
        });
    }
    tray.set_session_active(snapshot.phase == SessionPhase::Running && !pending);
    publish_studio(state, window);
}

fn publish_studio(state: &SharedState, window: &MainWindow) {
    let backend = window.global::<Backend>();
    let mode = backend.get_source_mode();
    let (models, projection, source, output, source_detail, gain, pan, snapshot) = {
        let state = state.borrow();
        let session = state
            .snapshot
            .as_ref()
            .filter(|snapshot| {
                matches!(
                    snapshot.phase,
                    SessionPhase::Starting | SessionPhase::Running
                )
            })
            .and_then(|snapshot| snapshot.desired.as_ref());
        let source = match session.map(|spec| &spec.source) {
            Some(InputSelection::Physical { endpoint_id }) => {
                endpoint_label(&state.catalog.inputs, endpoint_id, "input")
            }
            Some(InputSelection::Process {
                pid,
                creation_time_100ns,
            }) => state
                .catalog
                .processes
                .iter()
                .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
                .map_or_else(
                    || format!("Unavailable application · PID {pid} · {creation_time_100ns}"),
                    |row| row.name.clone(),
                ),
            Some(InputSelection::Tone { frequency_hz }) => format!("{frequency_hz:.0} Hz tone"),
            None => match mode {
                2 => "440 Hz test tone".to_owned(),
                1 => state
                    .selection
                    .process_source(&state.catalog)
                    .and_then(|source| {
                        let InputSelection::Process {
                            pid,
                            creation_time_100ns,
                        } = source
                        else {
                            return None;
                        };
                        state
                            .catalog
                            .processes
                            .iter()
                            .find(|row| {
                                row.pid == pid && row.creation_time_100ns == creation_time_100ns
                            })
                            .map(|row| row.name.clone())
                    })
                    .unwrap_or_else(|| "Select an application".to_owned()),
                _ => state
                    .catalog
                    .inputs
                    .iter()
                    .find(|row| Some(&row.endpoint_id) == state.selection.input_id.as_ref())
                    .map_or_else(|| "Select an input".to_owned(), |row| row.name.clone()),
            },
        };
        let output = session.map_or_else(
            || {
                state
                    .catalog
                    .outputs
                    .iter()
                    .find(|row| Some(&row.endpoint_id) == state.selection.output_id.as_ref())
                    .map_or_else(|| "Select an output".to_owned(), |row| row.name.clone())
            },
            |spec| endpoint_label(&state.catalog.outputs, &spec.output_endpoint_id, "output"),
        );
        let displayed_mode = session.map_or(mode, |spec| match spec.source {
            InputSelection::Physical { .. } => 0,
            InputSelection::Process { .. } => 1,
            InputSelection::Tone { .. } => 2,
        });
        let kind = match displayed_mode {
            1 => "APPLICATION AUDIO",
            2 => "TEST TONE",
            _ => "PHYSICAL INPUT",
        };
        let source_detail = session.and_then(|spec| {
            if let InputSelection::Tone { frequency_hz } = spec.source {
                Some(match spec.limit {
                    RunLimit::Seconds(seconds) => {
                        format!("{frequency_hz:.0} Hz · {seconds} seconds")
                    }
                    RunLimit::UntilStopped => format!("{frequency_hz:.0} Hz · until stopped"),
                })
            } else {
                None
            }
        });
        (
            state.studio_models.clone(),
            state.studio.projection(&source, kind, &output),
            source,
            output,
            source_detail,
            state.studio.gain,
            state.studio.pan,
            state.snapshot.clone(),
        )
    };
    models.publish(&backend, projection);
    if let Some(detail) = source_detail {
        let nodes = backend.get_graph_nodes();
        if let Ok(index) = usize::try_from(backend.get_graph_input_index())
            && let Some(mut node) = nodes.row_data(index)
            && node.detail != detail
        {
            node.detail = detail.into();
            nodes.set_row_data(index, node);
        }
    }
    backend.set_source_label(source.clone().into());
    backend.set_output_label(output.clone().into());
    backend.set_gain(gain);
    backend.set_pan(pan);
    let Some(snapshot) = snapshot else {
        return;
    };
    if matches!(
        snapshot.phase,
        SessionPhase::Starting | SessionPhase::Running
    ) && snapshot.desired.is_some()
    {
        backend.set_inspector_state(format!("{:?} · stereo", snapshot.phase).into());
    }
    backend.set_session_status(if snapshot.phase == SessionPhase::Running {
        format!("Running · session {} · {source}", snapshot.generation.0).into()
    } else {
        backend.get_phase_text()
    });
    backend.set_control_status(if snapshot.control.accepted_pending > 0 {
        format!(
            "Accepted · {} pending engine application",
            snapshot.control.accepted_pending
        )
        .into()
    } else if let Some(reply) = snapshot.control.last_result {
        format!(
            "{:?} · request {} · revision {} · frame {}",
            reply.code, reply.request_id, reply.plan_revision, reply.effective_frame
        )
        .into()
    } else if snapshot.phase == SessionPhase::Running {
        "Session parameters active".into()
    } else {
        "Candidate parameters · apply on Start".into()
    });
    backend.set_engine_format(if snapshot.phase == SessionPhase::Running {
        "48 kHz · engine".into()
    } else {
        "—".into()
    });
    backend.set_engine_latency(if snapshot.phase == SessionPhase::Running {
        snapshot
            .desired
            .as_ref()
            .map(|spec| format!("{} frame blocks", spec.max_block_frames))
            .unwrap_or_default()
            .into()
    } else {
        "—".into()
    });
    backend.set_engine_clock(if snapshot.phase == SessionPhase::Running {
        output.into()
    } else {
        "—".into()
    });
}

fn endpoint_label(rows: &[DeviceRow], id: &str, direction: &str) -> String {
    rows.iter().find(|row| row.endpoint_id == id).map_or_else(
        || format!("Unavailable {direction} · {id}"),
        |row| row.name.clone(),
    )
}

fn set_parameters(
    state: &SharedState,
    handle: &AppHandle,
    window: &MainWindow,
    gain: f32,
    pan: f32,
) {
    let running = {
        let mut state = state.borrow_mut();
        if !state.studio.set_parameters(gain, pan) {
            return;
        }
        state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.phase == SessionPhase::Running)
    };
    if running {
        dispatch(
            state,
            handle,
            AppRequest::SetGainPan {
                gain: gain.into(),
                pan: pan.into(),
            },
        );
    }
    publish_studio(state, window);
}

fn locate_selection(state: &SharedState, window: &MainWindow) {
    let focus = {
        let state = state.borrow();
        state.studio.projection("", "", "").focus
    };
    if let Some((x, y)) = focus {
        let backend = window.global::<Backend>();
        backend.set_focus_x(x);
        backend.set_focus_y(y);
        backend.set_focus_generation(backend.get_focus_generation().wrapping_add(1));
    }
}
fn endpoint_model(rows: &[DeviceRow], kind: &str, default: Option<&str>) -> ModelRc<AudioEndpoint> {
    ModelRc::new(VecModel::from(
        rows.iter()
            .map(|row| endpoint(row, kind, default))
            .collect::<Vec<_>>(),
    ))
}
fn endpoint(row: &DeviceRow, kind: &str, default: Option<&str>) -> AudioEndpoint {
    AudioEndpoint {
        id: row.endpoint_id.clone().into(),
        name: row.name.clone().into(),
        kind: kind.into(),
        is_default: Some(row.endpoint_id.as_str()) == default,
        note: "格式在启动时检测".into(),
        ..Default::default()
    }
}
fn options(rows: &[DeviceRow]) -> ModelRc<SelectOption> {
    ModelRc::new(VecModel::from(
        rows.iter()
            .map(|row| SelectOption {
                text: row.name.clone().into(),
                detail: "".into(),
            })
            .collect::<Vec<_>>(),
    ))
}
pub(super) fn dispatch(state: &SharedState, handle: &AppHandle, request: AppRequest) {
    let transport = matches!(request, AppRequest::Start(_) | AppRequest::Stop);
    let stopping = matches!(request, AppRequest::Stop);
    if transport && state.borrow().transport_pending {
        return;
    }
    let result = handle.try_request_tracked(request);
    let mut state = state.borrow_mut();
    match result {
        Ok(id) => {
            if stopping
                && let Some(generation) =
                    state.snapshot.as_ref().map(|snapshot| snapshot.generation)
            {
                state.signals.stop(generation);
            }
            state.dispatch_error = None;
            if transport {
                state.pending_request_id = Some(id);
                state.transport_pending = true;
            }
        }
        Err(error) => state.dispatch_error = Some(format!("请求未接受：{error:?}")),
    }
}
pub(super) fn wire(window: &MainWindow, handle: AppHandle, state: SharedState) {
    let backend = window.global::<Backend>();
    let models = state.borrow().studio_models.clone();
    models.install(&backend);
    publish_studio(&state, window);
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_select_node(move |id| {
        let selected = callback_state.borrow_mut().studio.select_node(&id);
        if let Some(window) = weak.upgrade() {
            publish_studio(&callback_state, &window);
        }
        selected
    });
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_select_wire(move |id| {
        callback_state.borrow_mut().studio.select_wire(&id);
        if let Some(window) = weak.upgrade() {
            publish_studio(&callback_state, &window);
        }
    });
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_clear_selection(move || {
        callback_state.borrow_mut().studio.clear_selection();
        if let Some(window) = weak.upgrade() {
            publish_studio(&callback_state, &window);
        }
    });
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_move_node(move |id, x, y| {
        callback_state.borrow_mut().studio.move_node(&id, x, y);
        if let Some(window) = weak.upgrade() {
            publish_studio(&callback_state, &window);
        }
    });
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_select_library_node(move |name| {
        callback_state.borrow_mut().studio.select_library(&name);
        if let Some(window) = weak.upgrade() {
            let backend = window.global::<Backend>();
            match name.as_str() {
                "Physical input" => backend.set_source_mode(0),
                "Application audio" => backend.set_source_mode(1),
                "Test tone" => backend.set_source_mode(2),
                _ => {}
            }
            publish_studio(&callback_state, &window);
            locate_selection(&callback_state, &window);
        }
    });
    let callback_state = state.clone();
    let weak = window.as_weak();
    backend.on_locate_selection(move || {
        if let Some(window) = weak.upgrade() {
            locate_selection(&callback_state, &window);
        }
    });
    let callback_state = state.clone();
    let app_handle = handle.clone();
    let weak = window.as_weak();
    backend.on_set_route_gain(move |gain| {
        if let Some(window) = weak.upgrade() {
            let pan = callback_state.borrow().studio.pan;
            set_parameters(&callback_state, &app_handle, &window, gain, pan);
        }
    });
    let s = state.clone();
    backend
        .on_select_detail(move |capture, id| s.borrow_mut().detail.select(capture, id.to_string()));
    for kind in 0..3 {
        let state = state.clone();
        let callback = move |row| {
            let mut state = state.borrow_mut();
            let catalog = state.catalog.clone();
            match kind {
                0 => state.selection.input_id = endpoint_at(&catalog.inputs, row),
                1 => state.selection.output_id = endpoint_at(&catalog.outputs, row),
                _ => state.selection.select_process(&catalog, row),
            }
        };
        match kind {
            0 => backend.on_select_input(callback),
            1 => backend.on_select_output(callback),
            _ => backend.on_select_process(callback),
        }
    }
    let s = state.clone();
    let h = handle.clone();
    backend.on_refresh_catalog(move || dispatch(&s, &h, AppRequest::RefreshCatalog));
    let s = state.clone();
    let h = handle.clone();
    backend.on_stop(move || dispatch(&s, &h, AppRequest::Stop));
    let s = state.clone();
    let h = handle.clone();
    let weak = window.as_weak();
    backend.on_set_gain_pan(move |gain, pan| {
        if let Some(window) = weak.upgrade() {
            set_parameters(&s, &h, &window, gain, pan);
        }
    });
    for tone in [false, true] {
        let s = state.clone();
        let h = handle.clone();
        let weak = window.as_weak();
        let start = move |frequency: f32, seconds: i32, forever: bool| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let backend = window.global::<Backend>();
            let source = {
                let state = s.borrow();
                if tone {
                    Some(InputSelection::Tone {
                        frequency_hz: frequency.into(),
                    })
                } else if backend.get_source_mode() == 0 {
                    (state.selection.input_index(&state.catalog) >= 0).then(|| {
                        InputSelection::Physical {
                            endpoint_id: state.selection.input_id.clone().unwrap(),
                        }
                    })
                } else {
                    state.selection.process_source(&state.catalog)
                }
            };
            let output = {
                let state = s.borrow();
                (state.selection.output_index(&state.catalog) >= 0)
                    .then(|| state.selection.output_id.clone().unwrap())
            };
            if let (Some(source), Some(output_endpoint_id)) = (source, output) {
                dispatch(
                    &s,
                    &h,
                    AppRequest::Start(SessionSpec {
                        source,
                        output_endpoint_id,
                        limit: if forever {
                            RunLimit::UntilStopped
                        } else {
                            RunLimit::Seconds(seconds.clamp(1, 600) as u16)
                        },
                        gain: backend.get_gain().into(),
                        pan: backend.get_pan().into(),
                        max_block_frames: 256,
                    }),
                );
            }
        };
        if tone {
            backend.on_start_tone(move |f, s| start(f, s, false));
        } else {
            backend.on_start_monitor(move |forever, s| start(0.0, s, forever));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_startup_parameters_are_projected_from_the_service() {
        let (handle, _) = ServiceMailbox::new(std::thread::current());
        let mut snapshot = (*handle.try_snapshot().unwrap()).clone();
        snapshot.generation = SessionGeneration(1);
        snapshot.desired = Some(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.0,
            pan: 0.0,
            max_block_frames: 256,
        });
        let mut state = BindingState::default();
        state.observe_snapshot(&Arc::new(snapshot));
        assert_eq!(state.studio.gain, 0.0);
    }
    #[test]
    fn unrelated_publication_cannot_acknowledge_transport_behind_round_budget() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        for _ in 0..20 {
            handle.try_request(AppRequest::RefreshCatalog).unwrap();
        }
        let state = Rc::new(RefCell::new(BindingState::default()));
        dispatch(&state, &handle, AppRequest::Stop);
        for _ in 0..16 {
            assert_eq!(mailbox.try_request(), Some(AppRequest::RefreshCatalog));
        }
        mailbox.publish((*handle.try_snapshot().unwrap()).clone());
        state
            .borrow_mut()
            .observe_snapshot(&handle.try_snapshot().unwrap());
        assert!(
            state.borrow().transport_pending,
            "unrelated publication must not acknowledge queued Stop"
        );
        dispatch(&state, &handle, AppRequest::Stop);
        let mut stops = 0;
        while let Some(request) = mailbox.try_request() {
            stops += usize::from(request == AppRequest::Stop);
        }
        assert_eq!(stops, 1);
    }
    #[test]
    fn old_phase_and_older_receipt_cannot_release_a_new_pending_transport() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let state = Rc::new(RefCell::new(BindingState::default()));
        let old_phase = handle.try_snapshot().unwrap();
        dispatch(&state, &handle, AppRequest::Stop);
        assert_eq!(mailbox.try_request(), Some(AppRequest::Stop));
        mailbox.publish((*old_phase).clone());
        let acknowledged = handle.try_snapshot().unwrap();
        state.borrow_mut().observe_snapshot(&old_phase);
        assert!(state.borrow().transport_pending);
        state.borrow_mut().observe_snapshot(&acknowledged);
        assert!(!state.borrow().transport_pending);
        dispatch(&state, &handle, AppRequest::Stop);
        state.borrow_mut().observe_snapshot(&acknowledged);
        assert!(state.borrow().transport_pending);
    }
    #[test]
    fn repeated_transport_is_coalesced_while_request_is_unacknowledged() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let state = Rc::new(RefCell::new(BindingState::default()));
        dispatch(&state, &handle, AppRequest::Stop);
        dispatch(&state, &handle, AppRequest::Stop);
        assert_eq!(mailbox.try_request(), Some(AppRequest::Stop));
        assert_eq!(mailbox.try_request(), None);
        assert!(state.borrow().transport_pending);
    }
    #[test]
    fn busy_dispatch_keeps_transport_retryable_and_exit_independent() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        while handle.try_request(AppRequest::RefreshCatalog).is_ok() {}
        let state = Rc::new(RefCell::new(BindingState::default()));
        dispatch(&state, &handle, AppRequest::Stop);
        assert!(!state.borrow().transport_pending);
        assert!(
            state
                .borrow()
                .dispatch_error
                .as_ref()
                .unwrap()
                .contains("Busy")
        );
        mailbox.try_request();
        dispatch(&state, &handle, AppRequest::Stop);
        assert!(state.borrow().transport_pending);
        assert_eq!(state.borrow().dispatch_error, None);
        handle.request_exit();
        assert!(mailbox.exit_requested());
    }
}
