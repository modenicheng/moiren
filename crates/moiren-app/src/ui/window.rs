//! Main window: owns the service guard, minimizes instead of quitting,
//! and polls service snapshots on the UI clock.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::app_view;
use super::monitoring::MeterSubscriptions;
use super::pages::{self, Page};
use super::state::UiState;
use super::theme;
use super::widgets::action_assets::ActionIcon;
use super::widgets::action_assets::{ActionAssets, register_fonts};
use super::widgets::actions::{ActionButton, ButtonVariant, ControlSize, Density, IconButton};
use super::widgets::navigation::{WindowAction, WindowControlButton, WorkspaceTab};
use crate::service::{AppHandle, AppRequest, ServiceRuntime, SessionPhase, start_service};
use pages::graph_canvas::Camera;

/// Presentation survives hiding independently of the service/audio lifetime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowPresentation {
    pub visible: bool,
    pub minimized: bool,
    pub page: Page,
}

impl WindowPresentation {
    pub fn show(&mut self) {
        self.visible = true;
        self.minimized = false;
    }
    pub fn hide_to_tray(&mut self) {
        self.visible = false;
    }
}

/// Established immediately after spawn so every construction/event-loop return
/// requests independent exit and waits for worker-owned retirement. No error
/// path can detach a service that still owns audio.
struct OwnedService(Option<ServiceRuntime>);
impl OwnedService {
    fn finish(&mut self) -> anyhow::Result<()> {
        if let Some(service) = self.0.take() {
            service.handle().request_exit();
            service
                .join()
                .map_err(|error| anyhow::anyhow!("service cleanup: {error:?}"))?;
        }
        Ok(())
    }
}
impl Drop for OwnedService {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub fn run() -> anyhow::Result<()> {
    let service =
        start_service().map_err(|e| anyhow::anyhow!("starting application service: {e:?}"))?;
    let handle = service.handle();
    let mut owned = OwnedService(Some(service));
    let result = run_app(handle.clone());
    let cleanup = owned.finish();
    result.and(cleanup)
}

fn run_app(handle: AppHandle) -> anyhow::Result<()> {
    let initial = super::state::initial_snapshot(&handle)?;
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("register embedded Inter font");

            let tray = super::tray::AppTray::new().ok();

            let state = cx.new(|_| UiState::new(handle.clone(), initial));
            state.update(cx, |s, _| s.dispatch(AppRequest::RefreshCatalog));

            let (window_handle, window_entity) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(SharedString::from("Moiren")),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    window_min_size: Some(size(px(940.), px(600.))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| StudioShell::new(state, window, cx, tray)),
            )
            .expect("failed to open main window");

            // Closing preserves the window entity and the independent audio owner.
            window_handle
                .update(cx, |_, window, cx| {
                    window.on_window_should_close(cx, |window, _cx| {
                        // The tray can reactivate this same entity; removing it would
                        // discard the service poller and its presentation state.
                        window.minimize_window();
                        false
                    });
                })
                .ok();

            window_entity.update(cx, |shell, cx| shell.spawn_poll(cx));
            cx.on_action::<app_view::Quit>(|_, cx| cx.quit());
        });
    Ok(())
}

/// Root view: toolbar, active page, status bar.
pub struct StudioShell {
    state: Entity<UiState>,
    settings_open: bool,
    project_menu: bool,
    sidebar_open: bool,
    inspector_open: bool,
    compact: bool,
    density: Density,
    device_tab: devices::DeviceTab,
    selected_device: Option<String>,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    meters: MeterSubscriptions,
    gain_slider: Entity<base::slider::SliderState>,
    pan_slider: Entity<base::slider::SliderState>,
    project_path: Option<std::path::PathBuf>,
    project_name: String,
    project_message: Option<String>,
    project_busy: bool,
    pending_project: Option<(
        crate::service::RequestId,
        crate::project::ProjectDocument,
        std::path::PathBuf,
    )>,
    #[cfg(feature = "gpui-test")]
    test_page: Option<Entity<PageCell>>,
    presentation: WindowPresentation,
    camera: Camera,
    node_positions: std::collections::HashMap<String, (f32, f32)>,
    node_drag: Option<(String, f32, f32)>,
    hand_tool: bool,
    matrix_scroll: ScrollHandle,
    mixer_scroll: ScrollHandle,
    meter_points: std::rc::Rc<std::cell::Cell<(bool, bool)>>,
    observation_nodes: Option<(
        crate::service::SessionGeneration,
        moiren_core::graph::NodeId,
        moiren_core::graph::NodeId,
    )>,
    canvas_size: Option<(f32, f32)>,
    selected_node: Option<String>,
    /// Screen-space anchor of an active pan drag (window coordinates).
    pan_grab: Option<(f32, f32)>,
    /// Tray bridge; None when the platform tray failed to build.
    tray: Option<super::tray::AppTray>,
    /// Precision-touchpad subclass state (Windows pointer input). Held for
    /// its Drop: unregisters the Win32 subclass when the shell goes away.
    #[cfg(windows)]
    #[allow(dead_code)]
    touchpad: super::touchpad::TouchpadRegistration,
    /// Open window handle for tray "show" re-opens.
    window_handle: Option<AnyWindowHandle>,
}

impl StudioShell {
    fn new(
        state: Entity<UiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
        tray: Option<super::tray::AppTray>,
    ) -> Self {
        #[cfg(windows)]
        let touchpad = {
            let mut registration = super::touchpad::TouchpadRegistration::new();
            // Subclass the Win32 window so touchpad pointer messages reach
            // default processing and Windows keeps recognizing gestures.
            if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window)
                && let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                registration.attach_raw(win32.hwnd.get());
            }
            registration
        };
        let gain = state.read(cx).gain as f32;
        let pan = state.read(cx).pan as f32;
        let gain_slider = cx.new(|_| {
            base::slider::SliderState::new()
                .min(-60.)
                .max(0.)
                .step(0.1)
                .default_value(if gain <= 0. { -60. } else { 20. * gain.log10() })
        });
        let pan_slider = cx.new(|_| {
            base::slider::SliderState::new()
                .min(-1.)
                .max(1.)
                .step(0.01)
                .default_value(pan)
        });
        cx.subscribe(
            &gain_slider,
            |shell, _, event: &base::slider::SliderEvent, cx| {
                if let base::slider::SliderEvent::Change(value) = event {
                    shell.state.update(cx, |state, _| {
                        state.gain = if value.end() <= -60. {
                            0.
                        } else {
                            10f64.powf(f64::from(value.end()) / 20.)
                        };
                        state.request_parameters();
                    });
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe(
            &pan_slider,
            |shell, _, event: &base::slider::SliderEvent, cx| {
                if let base::slider::SliderEvent::Change(value) = event {
                    shell.state.update(cx, |state, _| {
                        state.pan = f64::from(value.end());
                        state.request_parameters();
                    });
                    cx.notify();
                }
            },
        )
        .detach();
        let meters = MeterSubscriptions::new(state.read(cx).handle.clone());
        let density = crate::project::load_preferences()
            .map(|p| match p.density {
                crate::project::UiDensity::Compact => Density::Compact,
                _ => Density::Comfortable,
            })
            .unwrap_or_default();
        let mut shell = Self {
            state,
            settings_open: false,
            project_menu: false,
            sidebar_open: true,
            inspector_open: true,
            compact: false,
            density,
            device_tab: devices::DeviceTab::Outputs,
            selected_device: None,
            selected_route: None,
            meters,
            gain_slider,
            pan_slider,
            project_path: None,
            project_name: "Untitled session".into(),
            project_message: None,
            project_busy: false,
            pending_project: None,
            #[cfg(feature = "gpui-test")]
            test_page: None,
            presentation: WindowPresentation::default(),
            camera: Camera::default(),
            node_positions: Default::default(),
            node_drag: None,
            hand_tool: false,
            matrix_scroll: ScrollHandle::new(),
            mixer_scroll: ScrollHandle::new(),
            meter_points: Default::default(),
            observation_nodes: None,
            canvas_size: None,
            selected_node: None,
            pan_grab: None,
            tray,
            window_handle: None,
            #[cfg(windows)]
            touchpad,
        };
        shell.window_handle = Some(window.window_handle());
        shell.presentation.show();
        cx.observe_window_visibility(window, |shell, visibility, _, cx| {
            shell.presentation.visible = visibility.is_visible();
            shell.presentation.minimized = !visibility.is_visible();
            if !visibility.is_visible() {
                shell.meters.update(&shell.state.read(cx).snapshot, false);
            }
            cx.notify();
        })
        .detach();
        shell
    }

    fn spawn_poll(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                // Stop polling once the shell view is gone (window closed).
                if this
                    .update(cx, |shell, cx| {
                        let changed = shell.state.update(cx, |state, cx| {
                            let changed = state.poll();
                            if state.snapshot.phase == SessionPhase::Exited {
                                cx.quit();
                            }
                            changed
                        });
                        shell.drain_tray(cx);
                        shell.finish_project_load(cx);
                        let visible = shell.presentation.visible
                            && !shell.presentation.minimized
                            && !shell.settings_open
                            && shell.presentation.page == Page::Mixer;
                        let points = shell.meter_points.get();
                        let meter_changed = shell.meters.update_points(
                            &shell.state.read(cx).snapshot,
                            visible && points.0,
                            visible && points.1,
                        );
                        if changed || meter_changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.presentation.page = page;
        self.settings_open = false;
        self.project_menu = false;
        #[cfg(feature = "gpui-test")]
        if let Some(cell) = &self.test_page {
            cell.update(cx, |cell, _| cell.page = page);
        }
        cx.notify();
    }

    /// Apply drained tray actions on the UI thread.
    fn drain_tray(&mut self, cx: &mut Context<Self>) {
        let Some(tray) = self.tray.as_mut() else {
            return;
        };
        for action in tray.poll_actions() {
            match action {
                super::tray::TrayAction::ShowWindow => {
                    self.presentation.show();
                    if let Some(handle) = self.window_handle {
                        handle
                            .update(cx, |_, window, _| {
                                window.activate_window();
                            })
                            .ok();
                    }
                    cx.notify();
                }
                super::tray::TrayAction::StopSession => {
                    self.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::Stop);
                    });
                }
                super::tray::TrayAction::ExitApp => {
                    cx.quit();
                }
            }
        }
        // Reflect session state on the tray menu.
        let running = self.state.read(cx).snapshot.phase == SessionPhase::Running;
        if let Some(tray) = self.tray.as_ref() {
            tray.set_session_active(running);
        }
    }
}

/// Headless page holder: lets tests switch pages without the full shell.
#[derive(Default)]
pub struct PageCell {
    pub page: Page,
}

impl Render for PageCell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

/// Tests construct the production shell, including navigation and service callbacks.
#[cfg(feature = "gpui-test")]
pub fn shell_for_tests(
    state: Entity<UiState>,
    page: Entity<PageCell>,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) -> StudioShell {
    let mut shell = StudioShell::new(state, window, cx, None);
    shell.test_page = Some(page);
    shell
}

#[cfg(feature = "gpui-test")]
pub(crate) trait TestObserve {
    type Out;
    fn gpui_test_observe(self) -> Self::Out;
}
#[cfg(feature = "gpui-test")]
impl<E: gpui_kit::base::TestSupportExt> TestObserve for E {
    type Out = gpui_kit::base::ObservedElement<E>;
    fn gpui_test_observe(self) -> Self::Out {
        gpui_kit::base::TestSupportExt::test_support(self)
    }
}
#[cfg(not(feature = "gpui-test"))]
pub(crate) trait TestObserve {
    fn gpui_test_observe(self) -> Self;
}
#[cfg(not(feature = "gpui-test"))]
impl<E> TestObserve for E {
    fn gpui_test_observe(self) -> Self {
        self
    }
}

#[cfg(feature = "gpui-test")]
mod capture;
mod chrome;
#[cfg(feature = "gpui-test")]
pub use capture::run_ui_capture;
mod controls;
mod devices;
mod graph;
mod matrix;
mod mixer;
mod project;
mod settings;
#[cfg(all(test, feature = "gpui-test"))]
mod tests;

impl Render for StudioShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "gpui-test")]
        if let Some(page) = &self.test_page {
            self.presentation.page = page.read(cx).page;
        }
        let compact = f32::from(window.viewport_size().width) < 1200.;
        if compact != self.compact {
            self.sidebar_open = !compact;
            self.inspector_open = !compact;
        }
        self.compact = compact;
        let visible =
            window.is_visible() && !self.settings_open && self.presentation.page == Page::Mixer;
        self.refresh_observation_nodes(cx);
        self.meter_points.set((false, false));
        if !visible {
            self.meters.update(&self.state.read(cx).snapshot, false);
        }
        let (gain, pan) = {
            let state = self.state.read(cx);
            (
                if state.gain <= 0. {
                    -60.
                } else {
                    (20. * state.gain.log10()).clamp(-60., 0.) as f32
                },
                state.pan as f32,
            )
        };
        if self.gain_slider.read(cx).value().end() != gain {
            self.gain_slider
                .update(cx, |s, cx| s.set_value(gain, window, cx));
        }
        if self.pan_slider.read(cx).value().end() != pan {
            self.pan_slider
                .update(cx, |s, cx| s.set_value(pan, window, cx));
        }
        let error = {
            let state = self.state.read(cx);
            state
                .dispatch_error
                .as_ref()
                .or(state.snapshot.graph.last_error.as_ref())
                .or(state.snapshot.error.as_ref())
                .or(state.snapshot.catalog_error.as_ref())
                .cloned()
        };
        div()
            .id("studio-shell")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::WINDOW_BG)
            .text_color(theme::TEXT)
            .font_family(theme::FONT_UI)
            .text_size(px(theme::TEXT_MD))
            .child(chrome::toolbar(self, cx))
            .when_some(error, |view, error| {
                view.child(
                    div()
                        .id("error-strip")
                        .flex_none()
                        .px(px(theme::SPACE_4))
                        .py(px(theme::SPACE_2))
                        .bg(theme::soft(theme::DANGER))
                        .text_color(theme::DANGER)
                        .text_size(px(theme::TEXT_SM))
                        .child(error)
                        .gpui_test_observe(),
                )
            })
            .child(if self.settings_open {
                settings::page(self, cx)
            } else {
                match self.presentation.page {
                    Page::Graph => graph::render_graph_page(self, cx),
                    Page::Mixer => mixer::page(self, cx),
                    Page::Matrix => matrix::page(self, cx),
                    Page::Devices => devices::page(self, cx),
                }
            })
            .child(chrome::statusbar(self.state.read(cx)).gpui_test_observe())
            .when(self.project_menu, |v| v.child(project::menu(self, cx)))
    }
}

impl StudioShell {
    fn refresh_observation_nodes(&mut self, cx: &App) {
        use moiren_core::graph::NodeKind;
        let snapshot = &self.state.read(cx).snapshot;
        if snapshot.phase != SessionPhase::Running
            || self
                .observation_nodes
                .is_some_and(|(generation, _, _)| generation != snapshot.generation)
        {
            self.observation_nodes = None;
        }
        let graph = &snapshot.graph;
        if snapshot.phase == SessionPhase::Running
            && graph.active_revision == Some(graph.candidate_revision)
        {
            let source = graph
                .graph
                .nodes()
                .iter()
                .find(|n| n.kind() == NodeKind::Source)
                .map(|n| n.id());
            let sink = graph
                .graph
                .nodes()
                .iter()
                .find(|n| n.kind() == NodeKind::Sink)
                .map(|n| n.id());
            self.observation_nodes = source
                .zip(sink)
                .map(|(source, sink)| (snapshot.generation, source, sink));
        }
    }
}
