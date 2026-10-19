use super::*;

pub(super) struct DesktopOwner {
    pub(super) _desktop: Entity<Desktop>,
}
impl Global for DesktopOwner {}

pub(super) struct Desktop {
    state: Entity<UiState>,
    tray: Option<super::super::tray::AppTray>,
    window: Option<AnyWindowHandle>,
    pub(super) shell: Option<WeakEntity<StudioShell>>,
    pub(super) saved: crate::project::WindowState,
    resume: Option<PresentationSnapshot>,
    pub(super) state_path: Option<std::path::PathBuf>,
    state_save: Option<Task<()>>,
    preference_saves: Vec<Task<()>>,
    shutting_down: bool,
}

#[derive(Clone)]
struct PresentationSnapshot {
    page: Page,
    camera: Camera,
    node_positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    mixer_order: Vec<moiren_core::graph::NodeId>,
    mixer_width: f32,
    mixer_dock_width: f32,
    pending_node_spawn: Option<graph::PendingNodeSpawn>,
    selected_node: Option<moiren_core::graph::NodeId>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    sidebar_open: bool,
    inspector_open: bool,
    compact: bool,
    device_tab: devices::DeviceTab,
    selected_device: Option<String>,
    project_path: Option<std::path::PathBuf>,
    project_name: String,
    project_message: Option<String>,
    pending_project: Option<(
        crate::service::RequestId,
        crate::project::ProjectDocument,
        std::path::PathBuf,
    )>,
    preferences: crate::project::AppPreferences,
}

impl PresentationSnapshot {
    fn capture(shell: &mut StudioShell) -> Self {
        Self {
            page: shell.presentation.page,
            camera: shell.camera,
            node_positions: std::mem::take(&mut shell.node_positions),
            mixer_order: shell.mixer.order.clone(),
            mixer_width: shell.mixer.width,
            mixer_dock_width: shell.mixer.dock_width,
            pending_node_spawn: shell.pending_node_spawn.take(),
            selected_node: shell.selected_node.take(),
            selected_route: shell.selected_route,
            sidebar_open: shell.sidebar_open,
            inspector_open: shell.inspector_open,
            compact: shell.compact,
            device_tab: shell.device_tab,
            selected_device: shell.selected_device.take(),
            project_path: shell.project_path.take(),
            project_name: std::mem::take(&mut shell.project_name),
            project_message: shell.project_message.take(),
            pending_project: shell.pending_project.take(),
            preferences: shell.preferences.clone(),
        }
    }

    fn restore(self, shell: &mut StudioShell) {
        shell.presentation.page = self.page;
        shell.camera = self.camera;
        shell.node_positions = self.node_positions;
        shell.mixer.order = self.mixer_order;
        shell.mixer.width = self.mixer_width;
        shell.mixer.dock_width = self.mixer_dock_width;
        shell.pending_node_spawn = self.pending_node_spawn;
        shell.node_name_selection = None;
        shell.selected_node = self.selected_node;
        shell.selected_route = self.selected_route;
        shell.sidebar_open = self.sidebar_open;
        shell.inspector_open = self.inspector_open;
        shell.compact = self.compact;
        shell.device_tab = self.device_tab;
        shell.selected_device = self.selected_device;
        shell.project_path = self.project_path;
        shell.project_name = self.project_name;
        shell.project_message = self.project_message;
        shell.project_busy = self.pending_project.is_some();
        shell.pending_project = self.pending_project;
        shell.density = match self.preferences.density {
            crate::project::UiDensity::Compact => Density::Compact,
            crate::project::UiDensity::Comfortable => Density::Comfortable,
        };
        shell.preferences = self.preferences;
    }
}

impl Desktop {
    pub(super) fn new(
        state: Entity<UiState>,
        tray: Option<super::super::tray::AppTray>,
        cx: &mut Context<Self>,
    ) -> Self {
        let state_path = crate::project::window_state_path().ok();
        let saved = state_path
            .as_ref()
            .and_then(|path| crate::project::load_window_state_at(path).ok())
            .unwrap_or_default();
        cx.set_quit_mode(if tray.is_some() {
            QuitMode::Explicit
        } else {
            QuitMode::LastWindowClosed
        });
        cx.on_app_quit(|desktop, cx| {
            desktop.shutting_down = true;
            if let (Some(window), Some(shell)) = (desktop.window, desktop.shell.clone()) {
                let state = shell.update(cx, |shell, cx| {
                    window.update(cx, |_, window, _| {
                        if shell.presentation.visible {
                            capture_window_state(window, shell.presentation.page)
                        } else {
                            shell.window_state.clone()
                        }
                    })
                });
                if let Ok(Ok(state)) = state {
                    desktop.saved = state;
                }
            }
            desktop.save_window_state(cx);
            let save = desktop.state_save.take();
            let preferences = std::mem::take(&mut desktop.preference_saves);
            async move {
                if let Some(save) = save {
                    save.await;
                }
                for pending in preferences {
                    pending.await;
                }
            }
        })
        .detach();
        Self {
            state,
            tray,
            window: None,
            shell: None,
            saved,
            resume: None,
            state_path,
            state_save: None,
            preference_saves: Vec::new(),
            shutting_down: false,
        }
    }

    pub(super) fn show_window(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if let Some(handle) = self.window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return Ok(());
        }
        let state = self.state.clone();
        let desktop = cx.weak_entity();
        let saved = self.saved.clone();
        let (bounds, display_id) = restored_placement(&saved, cx);
        let resume = self.resume.clone();
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("Moiren".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_min_size: Some(size(px(940.0), px(600.0))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut shell = StudioShell::new(state, window, cx, Some(desktop));
                    shell.presentation.page = page(saved.workspace);
                    if let Some(resume) = resume {
                        resume.restore(&mut shell);
                    }
                    shell.window_state = capture_window_state(window, shell.presentation.page);
                    shell
                })
            },
        )?;
        let weak_shell = shell.downgrade();
        window.update(cx, |_, window, cx| {
            #[cfg(windows)]
            if let Err(error) = super::super::branding::set_window_icon(window) {
                eprintln!("Moiren window icon: {error}");
            }
            window.on_window_should_close(cx, move |window, cx| {
                let _ = weak_shell.update(cx, |shell, cx| shell.prepare_close(window, cx));
                true
            });
        })?;
        let preferences = std::mem::take(&mut self.preference_saves);
        if !preferences.is_empty() {
            let pending = cx.background_executor().spawn(async move {
                for pending in preferences {
                    pending.await;
                }
            });
            shell.update(cx, |shell, _| shell.preference_save = Some(pending));
        }
        self.window = Some(window);
        self.shell = Some(shell.downgrade());
        self.resume = None;
        Ok(())
    }

    pub(super) fn prepare_close(&mut self, shell: &mut StudioShell, _cx: &mut Context<Self>) {
        self.saved = shell.window_state.clone();
    }

    pub(super) fn window_released(&mut self, shell: &mut StudioShell, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        shell
            .preferences_quitting
            .store(true, std::sync::atomic::Ordering::Release);
        self.saved = shell.window_state.clone();
        self.resume = Some(PresentationSnapshot::capture(shell));
        self.window = None;
        self.shell = None;
        if let Some(pending) = shell.preference_save.take() {
            self.preference_saves.push(pending);
        }
        self.save_window_state(cx);
    }

    fn save_window_state(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.state_path.clone() else {
            return;
        };
        let saved = self.saved.clone();
        let previous = self.state_save.take();
        let executor = cx.background_executor().clone();
        self.state_save = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = crate::project::save_window_state_at(path, &saved) {
                eprintln!("Moiren window state: {error}");
            }
        }));
    }

    pub(super) fn spawn_poll(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let delay = this
                    .update(
                        cx,
                        |desktop, _| if desktop.window.is_some() { 33 } else { 100 },
                    )
                    .unwrap_or(100);
                cx.background_executor()
                    .timer(Duration::from_millis(delay))
                    .await;
                if this.update(cx, |desktop, cx| desktop.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let changed = self.state.update(cx, |state, _| state.poll());
        if self.state.read(cx).snapshot.phase == SessionPhase::Exited {
            cx.quit();
            return;
        }
        if let Some(tray) = self.tray.as_mut() {
            let actions = tray.poll_actions();
            for action in actions {
                match action {
                    super::super::tray::TrayAction::ShowWindow => {
                        if let Err(error) = self.show_window(cx) {
                            eprintln!("Moiren show window: {error}");
                        }
                    }
                    super::super::tray::TrayAction::StopSession => {
                        self.state.update(cx, |state, _| {
                            state.dispatch(AppRequest::Stop);
                        })
                    }
                    super::super::tray::TrayAction::ExitApp => cx.quit(),
                }
            }
        }
        if let Some(tray) = self.tray.as_ref() {
            tray.set_session_active(self.state.read(cx).snapshot.phase == SessionPhase::Running);
        }
        if let Some(shell) = self.shell.clone() {
            let saved = shell.update(cx, |shell, cx| {
                shell.finish_project_load(cx);
                let visible = shell.presentation.visible
                    && !shell.presentation.minimized
                    && !shell.settings_open
                    && matches!(shell.presentation.page, Page::Graph | Page::Mixer);
                let points = shell.meter_points.get();
                let meter_changed = shell.meters.update_points(
                    &shell.state.read(cx).snapshot,
                    visible && points.0,
                    visible && points.1,
                );
                let node_meter_changed = shell.update_node_meters(visible, cx);
                let meter_changed = meter_changed || node_meter_changed;
                if changed || meter_changed {
                    cx.notify();
                }
                shell.window_state.clone()
            });
            if let Ok(saved) = saved {
                self.saved = saved;
            }
        }
    }
}

pub(super) fn workspace(page: Page) -> crate::project::WorkspaceDocument {
    match page {
        Page::Graph => crate::project::WorkspaceDocument::Graph,
        Page::Mixer => crate::project::WorkspaceDocument::Mixer,
        Page::Matrix => crate::project::WorkspaceDocument::Matrix,
        Page::Devices => crate::project::WorkspaceDocument::Devices,
    }
}

fn page(workspace: crate::project::WorkspaceDocument) -> Page {
    match workspace {
        crate::project::WorkspaceDocument::Graph => Page::Graph,
        crate::project::WorkspaceDocument::Mixer => Page::Mixer,
        crate::project::WorkspaceDocument::Matrix => Page::Matrix,
        crate::project::WorkspaceDocument::Devices => Page::Devices,
    }
}

pub(super) fn capture_window_state(window: &Window, page: Page) -> crate::project::WindowState {
    let bounds = window.window_bounds();
    let rect = bounds.get_bounds();
    let maximized = matches!(bounds, WindowBounds::Maximized(_));
    #[cfg(windows)]
    let maximized = super::native::saved_maximized(window).unwrap_or(maximized);
    crate::project::WindowState {
        x: rect.origin.x.into(),
        y: rect.origin.y.into(),
        width: rect.size.width.into(),
        height: rect.size.height.into(),
        maximized,
        workspace: workspace(page),
    }
}

fn restored_placement(
    saved: &crate::project::WindowState,
    cx: &App,
) -> (WindowBounds, Option<DisplayId>) {
    let dimensions = size(px(saved.width.max(940.0)), px(saved.height.max(600.0)));
    let mut rect = Bounds {
        origin: point(px(saved.x), px(saved.y)),
        size: dimensions,
    };
    let displays = cx.displays();
    let display = displays
        .iter()
        .find(|display| {
            let screen = display.visible_bounds();
            rect.origin.x >= screen.origin.x
                && rect.origin.x + px(100.0) <= screen.origin.x + screen.size.width
                && rect.origin.y >= screen.origin.y
                && rect.origin.y + px(40.0) <= screen.origin.y + screen.size.height
        })
        .or_else(|| displays.first());
    if let Some(display) = display {
        let screen = display.visible_bounds();
        rect.size.width = rect.size.width.min(screen.size.width);
        rect.size.height = rect.size.height.min(screen.size.height);
        rect.origin.x = rect
            .origin
            .x
            .max(screen.origin.x)
            .min(screen.origin.x + screen.size.width - rect.size.width);
        rect.origin.y = rect
            .origin
            .y
            .max(screen.origin.y)
            .min(screen.origin.y + screen.size.height - rect.size.height);
    }
    (
        if saved.maximized {
            WindowBounds::Maximized(rect)
        } else {
            WindowBounds::Windowed(rect)
        },
        display.map(|display| display.id()),
    )
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use super::restored_placement;
    use gpui_kit::WindowBounds;

    #[gpui_kit::test]
    fn restore_clamps_disconnected_monitor_geometry_and_preserves_maximization(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let saved = crate::project::WindowState {
                x: -10000.0,
                y: -10000.0,
                width: 10000.0,
                height: 10000.0,
                maximized: true,
                ..Default::default()
            };
            let (bounds, id) = restored_placement(&saved, cx);
            assert!(matches!(bounds, WindowBounds::Maximized(_)));
            let rect = bounds.get_bounds();
            let display = cx
                .displays()
                .into_iter()
                .find(|display| Some(display.id()) == id)
                .unwrap();
            let screen = display.visible_bounds();
            assert!(rect.origin.x >= screen.origin.x && rect.origin.y >= screen.origin.y);
            assert!(rect.origin.x + rect.size.width <= screen.origin.x + screen.size.width);
            assert!(rect.origin.y + rect.size.height <= screen.origin.y + screen.size.height);
        });
    }
}
