//! Disposable main window views over the application-owned service projection.

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
    let preferences = crate::project::load_preferences().unwrap_or_default();
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("register embedded Inter font");

            let tray = match super::tray::AppTray::new() {
                Ok(tray) => Some(tray),
                Err(error) => {
                    eprintln!("Moiren tray unavailable: {error}");
                    None
                }
            };

            let state = cx.new(|_| UiState::new(handle.clone(), initial));
            state.update(cx, |s, _| {
                s.restore_preferences(&preferences);
                s.dispatch(AppRequest::RefreshCatalog);
            });

            let desktop = cx.new(|cx| desktop::Desktop::new(state, tray, cx));
            cx.set_global(desktop::DesktopOwner {
                _desktop: desktop.clone(),
            });
            desktop.update(cx, |desktop, cx| {
                desktop.show_window(cx).expect("failed to open main window");
                desktop.spawn_poll(cx);
            });
            cx.on_action::<app_view::Quit>(|_, cx| cx.quit());
        });
    Ok(())
}

/// Root view: toolbar, active page, status bar.
pub struct StudioShell {
    state: Entity<UiState>,
    settings_open: bool,
    settings: settings::SettingsEditor,
    project_menu: bool,
    sidebar_open: bool,
    inspector_open: bool,
    compact: bool,
    density: Density,
    preferences: crate::project::AppPreferences,
    preferences_path: Option<std::path::PathBuf>,
    preference_save: Option<Task<()>>,
    preference_save_revision: u64,
    preferences_quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    device_tab: devices::DeviceTab,
    selected_device: Option<String>,
    export_controls: devices::ExportControls,
    selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    meters: MeterSubscriptions,
    node_meters: super::monitoring::NodeMeters,
    node_meter_points:
        std::rc::Rc<std::cell::RefCell<std::collections::BTreeSet<moiren_core::graph::NodeId>>>,
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
    node_positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    node_drag: Option<(moiren_core::graph::NodeId, f32, f32)>,
    graph_focus: FocusHandle,
    connection_drag: Option<graph::ConnectionDrag>,
    pending_node_spawn: Option<graph::PendingNodeSpawn>,
    node_search: Entity<component::input::InputState>,
    node_name: Entity<component::input::InputState>,
    node_name_selection: Option<moiren_core::graph::NodeId>,
    hand_tool: bool,
    matrix_scroll: ScrollHandle,
    mixer_scroll: ScrollHandle,
    mixer: mixer::MixerPresentation,
    compressor: graph::CompressorControls,
    meter_points: std::rc::Rc<std::cell::Cell<(bool, bool)>>,
    observation_nodes: Option<(
        crate::service::SessionGeneration,
        moiren_core::graph::NodeId,
        moiren_core::graph::NodeId,
    )>,
    canvas_size: Option<(f32, f32)>,
    selected_node: Option<moiren_core::graph::NodeId>,
    /// Screen-space anchor of an active pan drag (window coordinates).
    pan_grab: Option<(f32, f32)>,
    /// Left background clicks deselect only on release; a pan keeps selection.
    background_click: Option<(f32, f32)>,
    desktop: Option<WeakEntity<desktop::Desktop>>,
    window_state: crate::project::WindowState,
    /// Precision-touchpad subclass state (Windows pointer input). Held for
    /// its Drop: unregisters the Win32 subclass when the shell goes away.
    #[cfg(windows)]
    #[allow(dead_code)]
    touchpad: super::touchpad::TouchpadRegistration,
}

impl StudioShell {
    fn new(
        state: Entity<UiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
        desktop: Option<WeakEntity<desktop::Desktop>>,
    ) -> Self {
        component::Theme::change(component::ThemeMode::Dark, Some(window), cx);
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
        let node_search =
            cx.new(|cx| component::input::InputState::new(window, cx).placeholder("Search nodes"));
        cx.subscribe(
            &node_search,
            |_, _, _: &component::input::InputEvent, cx| cx.notify(),
        )
        .detach();
        let node_name = cx.new(|cx| component::input::InputState::new(window, cx));
        let meters = MeterSubscriptions::new(state.read(cx).handle.clone());
        let node_meters = super::monitoring::NodeMeters::new(state.read(cx).handle.clone());
        let preferences = crate::project::load_preferences().unwrap_or_default();
        let density = match preferences.density {
            crate::project::UiDensity::Compact => Density::Compact,
            _ => Density::Comfortable,
        };
        let settings = settings::SettingsEditor::new(&preferences, cx);
        let export_controls = devices::ExportControls::new(window, cx);
        let mut shell = Self {
            state,
            settings_open: false,
            settings,
            project_menu: false,
            sidebar_open: true,
            inspector_open: true,
            compact: false,
            density,
            preferences,
            preferences_path: crate::project::preferences_path().ok(),
            preference_save: None,
            preference_save_revision: 0,
            preferences_quitting: Default::default(),
            device_tab: devices::DeviceTab::Outputs,
            selected_device: None,
            export_controls,
            selected_route: None,
            meters,
            node_meters,
            node_meter_points: Default::default(),
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
            graph_focus: cx.focus_handle(),
            connection_drag: None,
            pending_node_spawn: None,
            node_search,
            node_name,
            node_name_selection: None,
            hand_tool: false,
            matrix_scroll: ScrollHandle::new(),
            mixer_scroll: ScrollHandle::new(),
            mixer: mixer::MixerPresentation::default(),
            compressor: graph::CompressorControls::default(),
            meter_points: Default::default(),
            observation_nodes: None,
            canvas_size: None,
            selected_node: None,
            pan_grab: None,
            background_click: None,
            desktop,
            window_state: desktop::capture_window_state(window, Page::Graph),
            #[cfg(windows)]
            touchpad,
        };
        shell.presentation.show();
        let restored = shell
            .state
            .update(cx, |state, _| state.startup_project.take());
        if let Some(document) = restored {
            if let Err(error) = shell.apply_project_presentation(&document, cx) {
                shell.project_message = Some(error.to_string());
            }
            shell.project_path = shell.preferences.last_project_path.clone();
        }
        cx.observe_window_bounds(window, |shell, window, _| {
            if shell.presentation.visible {
                shell.window_state = desktop::capture_window_state(window, shell.presentation.page);
            }
        })
        .detach();
        cx.on_release(|shell, cx| {
            if let Some(desktop) = shell.desktop.clone() {
                let _ = desktop.update(cx, |desktop, cx| desktop.window_released(shell, cx));
            }
        })
        .detach();
        cx.observe_window_visibility(window, |shell, visibility, _, cx| {
            shell.presentation.visible = visibility.is_visible();
            shell.presentation.minimized = !visibility.is_visible();
            if !visibility.is_visible() {
                shell.meters.update(&shell.state.read(cx).snapshot, false);
            }
            cx.notify();
        })
        .detach();
        // Quit observers run before GPUI drops the windows. Move the latest
        // write into shutdown ownership: it also owns every earlier queued
        // write, and must finish after the shell entity is gone.
        cx.on_app_quit(|shell, _| {
            shell
                .preferences_quitting
                .store(true, std::sync::atomic::Ordering::Release);
            let pending = shell.preference_save.take();
            async move {
                if let Some(pending) = pending {
                    pending.await;
                }
            }
        })
        .detach();
        shell
    }

    #[cfg(feature = "gpui-test")]
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
                        shell.finish_project_load(cx);
                        let meters_changed =
                            !shell.meters_frame_active(cx) && shell.pump_meters(cx);
                        if changed || meters_changed {
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

    /// Whether any meter observation is on screen right now. While true the
    /// render loop requests animation frames so meter motion runs at the
    /// display refresh rate.
    fn meters_frame_active(&self, cx: &App) -> bool {
        let snapshot = &self.state.read(cx).snapshot;
        self.presentation.visible
            && !self.presentation.minimized
            && !self.settings_open
            && matches!(self.presentation.page, Page::Graph | Page::Mixer)
            && snapshot.phase == SessionPhase::Running
            && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
    }

    /// Refresh every meter observation once: reads the latest windows,
    /// advances display ballistics, and reconciles subscriptions with the
    /// current visibility. Called per display frame while
    /// [`Self::meters_frame_active`] holds, and from the slower control-side
    /// loops otherwise. Returns whether a raw reading or status changed;
    /// ballistics motion alone does not, because animation frames (not
    /// notify) carry it.
    pub(super) fn pump_meters(&mut self, cx: &App) -> bool {
        self.meters.set_preferences(self.preferences.meters);
        self.node_meters.set_preferences(self.preferences.meters);
        let visible = self.meters_frame_active(cx);
        let points = self.meter_points.get();
        let meter_changed = self.meters.update_points(
            &self.state.read(cx).snapshot,
            visible && points.0,
            visible && points.1,
        );
        let node_meter_changed = self.update_node_meters(visible, cx);
        meter_changed || node_meter_changed
    }

    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.presentation.page = page;
        self.window_state.workspace = desktop::workspace(page);
        self.settings_open = false;
        self.project_menu = false;
        #[cfg(feature = "gpui-test")]
        if let Some(cell) = &self.test_page {
            cell.update(cx, |cell, _| cell.page = page);
        }
        cx.notify();
    }

    fn prepare_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presentation.visible {
            self.window_state = desktop::capture_window_state(window, self.presentation.page);
        }
        self.presentation.hide_to_tray();
        self.meters.update(&self.state.read(cx).snapshot, false);
        self.update_node_meters(false, cx);
        if let Some(desktop) = self.desktop.clone() {
            let _ = desktop.update(cx, |desktop, cx| desktop.prepare_close(self, cx));
        }
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_close(window, cx);
        window.remove_window();
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
mod desktop;
#[cfg(feature = "gpui-test")]
pub use capture::run_ui_capture;
mod controls;
mod devices;
mod graph;
mod matrix;
mod mixer;
mod native;
mod project;
mod settings;
#[cfg(any(test, feature = "gpui-test"))]
mod test_harness;
#[cfg(any(test, feature = "gpui-test"))]
pub use test_harness::{finish_service, initial_projection, spawn_service, wait_for, wait_until};
#[cfg(feature = "gpui-test")]
pub use test_harness::{shell_with_state, test_shell};
#[cfg(all(test, feature = "gpui-test"))]
mod tests;
mod tray_mixer;

impl Render for StudioShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "gpui-test")]
        self.mixer
            .trace
            .borrow_mut()
            .begin(self.presentation.page == Page::Mixer);
        #[cfg(feature = "gpui-test")]
        let visible_meter_count = self.node_meter_points.borrow().len();
        self.finish_settings_save(cx);
        self.keep_settings_focus(window, cx);
        self.finish_node_spawn(cx);
        self.sync_node_name(window, cx);
        self.sync_export_controls(window, cx);
        self.sync_mixer_controls(window, cx);
        self.sync_compressor_controls(window, cx);
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
            window.is_visible() && matches!(self.presentation.page, Page::Graph | Page::Mixer);
        if self.meters_frame_active(cx) {
            #[cfg(feature = "gpui-test")]
            let meter_began = std::time::Instant::now();
            // Frame-driven meters: advance observations every display frame
            // (this frame builds from the pumped state) and keep the
            // vsync loop running. Must run before the per-frame reset of
            // `meter_points` below so visibility comes from the last frame.
            self.pump_meters(cx);
            #[cfg(feature = "gpui-test")]
            self.mixer.trace.borrow_mut().add(meter_began.elapsed());
            window.request_animation_frame();
        }
        self.refresh_observation_nodes(cx);
        self.meter_points.set((false, false));
        self.node_meter_points.borrow_mut().clear();
        if !visible {
            self.meters.update(&self.state.read(cx).snapshot, false);
            self.update_node_meters(false, cx);
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
        let error_strip = error.map(|error| {
            let log_path = crate::logging::log_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Application file logging is unavailable".into());
            div()
                .id("error-strip")
                .flex_none()
                .px(px(theme::SPACE_4))
                .py(px(theme::SPACE_2))
                .bg(theme::soft(theme::DANGER))
                .text_color(theme::DANGER)
                .text_size(px(theme::TEXT_SM))
                .child(error)
                .child(
                    div()
                        .pt(px(2.))
                        .text_color(theme::TEXT)
                        .child(format!("Diagnostics: {log_path}")),
                )
                .gpui_test_observe()
        });
        let view = div()
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
            .when_some(error_strip, |view, error_strip| view.child(error_strip))
            .child(match self.presentation.page {
                Page::Graph => graph::render_graph_page(self, cx),
                Page::Mixer => mixer::page(self, cx),
                Page::Matrix => matrix::page(self, cx),
                Page::Devices => devices::page(self, cx),
            })
            .child(chrome::statusbar(self.state.read(cx)).gpui_test_observe())
            .when(self.project_menu, |v| v.child(project::menu(self, cx)))
            .when(self.settings_open, |v| {
                v.child(settings::overlay(self, window, cx))
            });
        #[cfg(feature = "gpui-test")]
        self.mixer.trace.borrow_mut().finish(visible_meter_count);
        view
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
