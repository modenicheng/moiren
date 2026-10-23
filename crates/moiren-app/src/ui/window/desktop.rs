use super::*;

pub(super) struct DesktopOwner {
    pub(super) _desktop: Entity<Desktop>,
}
impl Global for DesktopOwner {}

pub(super) struct Desktop {
    state: Entity<UiState>,
    tray: Option<super::super::tray::AppTray>,
    window: Option<AnyWindowHandle>,
    pub(super) flyout: Option<AnyWindowHandle>,
    pub(super) flyout_view: Option<WeakEntity<tray_mixer::TrayMixer>>,
    /// Whether the flyout has been observed active at least once; focus-loss
    /// dismissal only counts after a real activation.
    flyout_active_seen: bool,
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
            flyout: None,
            flyout_view: None,
            flyout_active_seen: false,
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

    /// Toggle the tray mixer flyout: a live window closes, otherwise a new
    /// one opens anchored to the tray icon.
    pub(super) fn toggle_mixer_flyout(
        &mut self,
        anchor: &super::super::tray::TrayAnchor,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if let Some(handle) = self.flyout.take() {
            if handle
                .update(cx, |_, window, _| window.remove_window())
                .is_ok()
            {
                return Ok(());
            }
            self.flyout_view = None;
        }
        let strips = tray_mixer::strip_ids(&self.state.read(cx).snapshot.graph).len();
        let content = tray_mixer::preferred_size(strips);
        let (rect, display_id) = self.flyout_placement(anchor, content, cx);
        let state = self.state.clone();
        let desktop = cx.weak_entity();
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(rect),
                display_id,
                titlebar: None,
                kind: WindowKind::PopUp,
                is_movable: false,
                focus: true,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| tray_mixer::TrayMixer::new(state, Some(desktop), window, cx)),
        )?;
        self.flyout = Some(window);
        self.flyout_view = Some(view.downgrade());
        self.flyout_active_seen = false;
        // Shown from a poll timer, the popup can lose the foreground race to
        // the shell window; force activation so the first click elsewhere is
        // already a focus loss (and Escape reaches the view).
        let _ = window.update(cx, |_, window, _| window.activate_window());
        Ok(())
    }

    /// Window placement for the flyout anchor. On Windows the tray rect is
    /// physical pixels; the flyout is placed in physical space and handed to
    /// gpui as logical pixels for the matched monitor (gpui converts window
    /// bounds with the target display's own scale factor). Without anchor
    /// geometry the flyout rests against the first display's work area.
    fn flyout_placement(
        &self,
        anchor: &super::super::tray::TrayAnchor,
        content: Size<Pixels>,
        cx: &Context<Self>,
    ) -> (WindowBounds, Option<DisplayId>) {
        #[cfg(windows)]
        if let Some((monitor, work, scale)) =
            native::monitor_geometry_at(anchor.x + anchor.width / 2., anchor.y + anchor.height / 2.)
        {
            let geometry = [MonitorGeometry {
                monitor,
                work,
                scale,
            }];
            if let Some((rect, scale)) = tray_flyout_bounds(anchor, content, &geometry) {
                let logical = Bounds {
                    origin: point(px((rect.x / scale) as f32), px((rect.y / scale) as f32)),
                    size: content,
                };
                // Identify the gpui display by its logical work area.
                let display_id = cx
                    .displays()
                    .iter()
                    .find(|display| {
                        let visible = display.visible_bounds();
                        (f64::from(f32::from(visible.origin.x)) - work.x / scale).abs() < 2.
                            && (f64::from(f32::from(visible.origin.y)) - work.y / scale).abs() < 2.
                    })
                    .map(|display| display.id());
                return (WindowBounds::Windowed(logical), display_id);
            }
        }
        let display = cx.displays().first().cloned();
        (
            WindowBounds::Windowed(Bounds {
                origin: display
                    .as_ref()
                    .map(|display| display.visible_bounds().origin)
                    .unwrap_or_else(|| point(px(0.), px(0.))),
                size: content,
            }),
            display.as_ref().map(|display| display.id()),
        )
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
                    .update(cx, |desktop, _| {
                        if desktop.window.is_some() || desktop.flyout.is_some() {
                            33
                        } else {
                            100
                        }
                    })
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

    /// Closes the flyout once it has lost activation after having held it.
    /// Checked from the poll (not the activation callback) so the window is
    /// never destroyed mid-event-dispatch; the next poll lands within 33 ms.
    fn dismiss_flyout_on_focus_loss(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.flyout else {
            return;
        };
        match handle.update(cx, |_, window, _| window.is_window_active()) {
            Ok(true) => self.flyout_active_seen = true,
            Ok(false) if self.flyout_active_seen => {
                self.flyout = None;
                self.flyout_view = None;
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            Ok(false) => {}
            Err(_) => {
                self.flyout = None;
                self.flyout_view = None;
            }
        }
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
                    super::super::tray::TrayAction::ShowMixer(anchor) => {
                        if let Err(error) = self.toggle_mixer_flyout(&anchor, cx) {
                            eprintln!("Moiren tray mixer: {error}");
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
        self.dismiss_flyout_on_focus_loss(cx);
        if let Some(shell) = self.shell.clone() {
            let saved = shell.update(cx, |shell, cx| {
                shell.finish_project_load(cx);
                if changed || shell.pump_meters(cx) {
                    cx.notify();
                }
                shell.window_state.clone()
            });
            if let Ok(saved) = saved {
                self.saved = saved;
            }
        }
        if let Some(view) = self.flyout_view.clone() {
            let pumped = view.update(cx, |flyout, cx| {
                if changed || flyout.pump_meters(cx) {
                    cx.notify();
                }
            });
            if pumped.is_err() {
                self.flyout = None;
                self.flyout_view = None;
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

/// A rectangle in physical screen pixels (Win32 virtual-screen space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PhysicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Monitor geometry used to anchor the tray flyout.
#[derive(Clone, Copy)]
struct MonitorGeometry {
    /// Full monitor rectangle, physical pixels.
    monitor: PhysicalRect,
    /// Work area (taskbar excluded), physical pixels.
    work: PhysicalRect,
    /// Physical pixels per logical pixel.
    scale: f64,
}

/// Places the flyout against the tray anchor in physical screen space:
/// centered horizontally on the icon, growing away from the taskbar edge
/// nearest the icon, clamped into the work area. Returns the physical rect
/// and the matched monitor's scale; None when no monitor contains the
/// anchor (topology changed mid-flight).
fn tray_flyout_bounds(
    anchor: &super::super::tray::TrayAnchor,
    content: Size<Pixels>,
    monitors: &[MonitorGeometry],
) -> Option<(PhysicalRect, f64)> {
    let center_x = anchor.x + anchor.width / 2.;
    let center_y = anchor.y + anchor.height / 2.;
    let geometry = monitors.iter().find(|geometry| {
        geometry.monitor.x <= center_x
            && center_x < geometry.monitor.x + geometry.monitor.width
            && geometry.monitor.y <= center_y
            && center_y < geometry.monitor.y + geometry.monitor.height
    })?;
    let scale = geometry.scale;
    let width = (f64::from(f32::from(content.width)) * scale).min(geometry.work.width);
    let height = (f64::from(f32::from(content.height)) * scale).min(geometry.work.height);
    // Bottom taskbar (the common case): the flyout's bottom rests on the
    // work area; a top taskbar mirrors that from the top edge.
    let origin_y = if center_y > geometry.monitor.y + geometry.monitor.height / 2. {
        geometry.work.y + geometry.work.height - height
    } else {
        geometry.work.y
    };
    let origin_x = (center_x - width / 2.)
        .max(geometry.work.x)
        .min(geometry.work.x + geometry.work.width - width);
    let origin_y = origin_y
        .max(geometry.work.y)
        .min(geometry.work.y + geometry.work.height - height);
    Some((
        PhysicalRect {
            x: origin_x,
            y: origin_y,
            width,
            height,
        },
        scale,
    ))
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use super::{MonitorGeometry, PhysicalRect, restored_placement, tray_flyout_bounds};
    use crate::ui::tray::TrayAnchor;
    use gpui_kit::{WindowBounds, px, size};

    fn geometry(
        monitor: (f64, f64, f64, f64),
        work: (f64, f64, f64, f64),
        scale: f64,
    ) -> MonitorGeometry {
        MonitorGeometry {
            monitor: PhysicalRect {
                x: monitor.0,
                y: monitor.1,
                width: monitor.2,
                height: monitor.3,
            },
            work: PhysicalRect {
                x: work.0,
                y: work.1,
                width: work.2,
                height: work.3,
            },
            scale,
        }
    }

    fn anchor(x: f64, y: f64) -> TrayAnchor {
        TrayAnchor {
            x,
            y,
            width: 24.,
            height: 24.,
        }
    }

    #[test]
    fn flyout_rests_on_work_area_above_bottom_taskbar() {
        // 1920x1080 at 100%, taskbar 40px at the bottom, tray at x=960.
        let monitors = [geometry((0., 0., 1920., 1080.), (0., 0., 1920., 1040.), 1.)];
        let (rect, scale) =
            tray_flyout_bounds(&anchor(960., 1044.), size(px(360.), px(352.)), &monitors)
                .expect("anchor on monitor");
        assert_eq!(scale, 1.);
        assert_eq!(
            rect,
            PhysicalRect {
                x: 792.,
                y: 688.,
                width: 360.,
                height: 352.
            }
        );
    }

    #[test]
    fn flyout_hangs_below_top_taskbar() {
        let monitors = [geometry(
            (0., 0., 1920., 1080.),
            (0., 40., 1920., 1040.),
            1.,
        )];
        let (rect, _) = tray_flyout_bounds(&anchor(100., 8.), size(px(280.), px(352.)), &monitors)
            .expect("anchor on monitor");
        assert_eq!(rect.y, 40.);
        // Centering would push past the left work-area edge; clamp instead.
        assert_eq!(rect.x, 0.);
    }

    #[test]
    fn flyout_scales_content_and_matches_secondary_monitor() {
        // Secondary display at 150%: physical 1920x1080 next to the primary,
        // bottom taskbar 60 physical px.
        let monitors = [
            geometry((0., 0., 1920., 1080.), (0., 0., 1920., 1040.), 1.),
            geometry((1920., 0., 1920., 1080.), (1920., 0., 1920., 1020.), 1.5),
        ];
        let (rect, scale) =
            tray_flyout_bounds(&anchor(2388., 988.), size(px(360.), px(352.)), &monitors)
                .expect("anchor on secondary monitor");
        assert_eq!(scale, 1.5);
        // 360 logical px = 540 physical; centered on anchor x=2400.
        assert_eq!(rect.x, 2400. - 270.);
        assert_eq!(rect.y, 1020. - 528.);
        assert_eq!(rect.width, 540.);
        assert_eq!(rect.height, 528.);
    }

    #[test]
    fn flyout_clamps_against_right_work_area_edge() {
        let monitors = [geometry((0., 0., 1920., 1080.), (0., 0., 1920., 1040.), 1.)];
        let (rect, _) =
            tray_flyout_bounds(&anchor(1900., 1044.), size(px(360.), px(352.)), &monitors)
                .expect("anchor on monitor");
        assert_eq!(rect.x, 1920. - 360.);
    }

    #[test]
    fn flyout_rejects_anchor_outside_every_monitor() {
        let monitors = [geometry((0., 0., 1920., 1080.), (0., 0., 1920., 1040.), 1.)];
        assert!(
            tray_flyout_bounds(&anchor(5000., 5000.), size(px(360.), px(352.)), &monitors)
                .is_none()
        );
    }

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
