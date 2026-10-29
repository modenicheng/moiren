//! Disposable main window views over the application-owned service projection.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

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
    crate::i18n::apply(preferences.language);
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("register embedded Inter font");
            cx.bind_keys([
                KeyBinding::new("ctrl-z", app_view::Undo, None),
                KeyBinding::new("ctrl-y", app_view::Redo, None),
                KeyBinding::new("ctrl-shift-z", app_view::Redo, None),
            ]);

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
    /// Multi-selection on the graph canvas; `selected_node` is the primary
    /// (last-clicked) member that drives the inspector.
    selected_nodes: std::collections::BTreeSet<moiren_core::graph::NodeId>,
    /// Active rubber-band drag; see [`graph::SelectionDrag`].
    selection_drag: Option<graph::SelectionDrag>,
    graph_focus: FocusHandle,
    connection_drag: Option<graph::ConnectionDrag>,
    pending_node_spawn: Option<graph::PendingNodeSpawn>,
    node_search: Entity<component::input::InputState>,
    node_name: Entity<component::input::InputState>,
    /// Node whose label is being edited inline in the inspector.
    node_name_editing: Option<moiren_core::graph::NodeId>,
    hand_tool: bool,
    matrix_scroll: ScrollHandle,
    mixer_scroll: ScrollHandle,
    mixer: mixer::MixerPresentation,
    mixer_geometry: mixer::meter_frame::Geometry,
    mixer_controls: Entity<mixer::meter_frame::MixerControls>,
    mixer_toolbar: Entity<chrome::CachedChrome>,
    mixer_status: Entity<chrome::CachedChrome>,
    mixer_frame: Entity<mixer::meter_frame::MixerMeterFrame>,
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
    /// Last selectable-element press (target hash + time) for double-click
    /// detection: single clicks select, double clicks also open the inspector.
    last_element_press: Option<(u64, Instant)>,
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
        let node_search = cx.new(|cx| {
            component::input::InputState::new(window, cx).placeholder(t!("Search nodes"))
        });
        cx.subscribe(
            &node_search,
            |_, _, _: &component::input::InputEvent, cx| cx.notify(),
        )
        .detach();
        let node_name = cx.new(|cx| component::input::InputState::new(window, cx));
        cx.subscribe(
            &node_name,
            |shell, _, event: &component::input::InputEvent, cx| {
                match event {
                    component::input::InputEvent::PressEnter { .. } => {
                        shell.confirm_node_rename(cx);
                    }
                    // Clicking elsewhere commits the edit, like the compressor
                    // fields; explicit cancel is the ✗ button.
                    component::input::InputEvent::Blur => shell.confirm_node_rename(cx),
                    _ => {}
                }
            },
        )
        .detach();
        let meters = MeterSubscriptions::new(state.read(cx).handle.clone());
        let node_meters = super::monitoring::NodeMeters::new(state.read(cx).handle.clone());
        let preferences = crate::project::load_preferences().unwrap_or_default();
        let density = match preferences.density {
            crate::project::UiDensity::Compact => Density::Compact,
            _ => Density::Comfortable,
        };
        let settings = settings::SettingsEditor::new(&preferences, cx);
        let export_controls = devices::ExportControls::new(window, cx);
        let mixer_geometry = mixer::meter_frame::Geometry::default();
        let shell_entity = cx.entity();
        let mixer_controls = cx.new(|child_cx| {
            mixer::meter_frame::MixerControls::new(
                shell_entity.clone(),
                mixer_geometry.clone(),
                child_cx,
            )
        });
        let mixer_frame = cx.new(|_| {
            mixer::meter_frame::MixerMeterFrame::new(
                shell_entity.downgrade(),
                mixer_geometry.clone(),
            )
        });
        let mixer_toolbar = cx.new(|cx| chrome::CachedChrome::new(shell_entity.clone(), true, cx));
        let mixer_status = cx.new(|cx| chrome::CachedChrome::new(shell_entity.clone(), false, cx));
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
            project_name: t!("Untitled session").into(),
            project_message: None,
            project_busy: false,
            pending_project: None,
            #[cfg(feature = "gpui-test")]
            test_page: None,
            presentation: WindowPresentation::default(),
            camera: Camera::default(),
            node_positions: Default::default(),
            node_drag: None,
            selected_nodes: Default::default(),
            selection_drag: None,
            graph_focus: cx.focus_handle(),
            connection_drag: None,
            pending_node_spawn: None,
            node_search,
            node_name,
            node_name_editing: None,
            hand_tool: false,
            matrix_scroll: ScrollHandle::new(),
            mixer_scroll: ScrollHandle::new(),
            mixer: mixer::MixerPresentation::default(),
            mixer_geometry,
            mixer_controls,
            mixer_toolbar,
            mixer_status,
            mixer_frame,
            compressor: graph::CompressorControls::default(),
            meter_points: Default::default(),
            observation_nodes: None,
            canvas_size: None,
            selected_node: None,
            pan_grab: None,
            background_click: None,
            last_element_press: None,
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
                            !shell.meter_animation_active(cx) && shell.pump_meters(cx);
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
    }

    /// Whether any observed meter still moves: decaying ballistics, a held
    /// peak marker, or fresh readings above the scale bottom all keep the
    /// display short of rest. While false the vsync loop stands down and the
    /// control-side poll alone keeps the observations current, so a fully
    /// silent session costs no animation frames at all.
    fn meters_in_motion(&self) -> bool {
        !self.meters.at_rest() || !self.node_meters.at_rest()
    }

    /// Whether the animation-frame loop is currently driving meter pumps;
    /// the control-side poll may skip pumping only while this holds.
    fn meter_animation_active(&self, cx: &App) -> bool {
        self.meters_frame_active(cx) && self.meters_in_motion()
    }

    /// Refresh every meter observation once: reads the latest windows,
    /// advances display ballistics, and reconciles subscriptions with the
    /// current visibility. Called per display frame while
    /// [`Self::meter_animation_active`] holds, and from the slower
    /// control-side loops otherwise. Returns whether a display-bearing
    /// reading changed — raw levels, status, or a freshly latched clip;
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
    // Headless tests must never write the developer's real configuration.
    shell.preferences_path = None;
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
mod sidebar;
#[cfg(any(test, feature = "gpui-test"))]
mod test_harness;
#[cfg(any(test, feature = "gpui-test"))]
pub use test_harness::{finish_service, initial_projection, spawn_service, wait_for, wait_until};
#[cfg(feature = "gpui-test")]
pub use test_harness::{settle_panels, shell_with_state, test_shell};
#[cfg(all(test, feature = "gpui-test"))]
mod tests;
mod tray_mixer;

impl Render for StudioShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.finish_settings_save(cx);
        self.keep_settings_focus(window, cx);
        self.finish_node_spawn(cx);
        self.sync_node_name(window, cx);
        self.keep_graph_focus(window, cx);
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
        if self.meters_frame_active(cx) && self.presentation.page == Page::Graph {
            // Mixer owns a separate animation canvas; Graph keeps its existing
            // frame-driven path. Never pump the same subscriptions twice.
            self.pump_meters(cx);
            // Motion, not visibility, sustains the loop: once every observed
            // layer rests at the scale bottom the frames stop and the
            // control-side poll takes over pumping until fresh data moves.
            if self.meters_in_motion() {
                window.request_animation_frame();
            }
        }
        self.refresh_observation_nodes(cx);
        // Mixer demand is retained with its cached controls. Clearing it on
        // an animation-only frame would unsubscribe meters that remain visible.
        if self.presentation.page != Page::Mixer {
            self.meter_points.set((false, false));
            self.node_meter_points.borrow_mut().clear();
        }
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
        let error_strip = {
            let state = self.state.read(cx);
            let faults = state.snapshot.graph.faults.clone();
            let other = state
                .dispatch_error
                .as_ref()
                .or(state.snapshot.error.as_ref())
                .or(state.snapshot.catalog_error.as_ref())
                .cloned();
            (!faults.is_empty() || other.is_some()).then(|| {
                let log_path = crate::logging::log_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| t!("Application file logging is unavailable").into());
                let this = cx.entity();
                let graph = state.snapshot.graph.clone();
                let mut list = div()
                    .id("error-strip")
                    .flex_none()
                    .flex()
                    .flex_col()
                    .px(px(theme::SPACE_4))
                    .py(px(theme::SPACE_2))
                    .bg(theme::soft(theme::DANGER))
                    .text_size(px(theme::TEXT_SM));
                for (index, fault) in faults.iter().enumerate() {
                    let subject = fault.subject;
                    let location = match subject {
                        crate::service::GraphSubject::Node(node) => {
                            graph.node(node).map(|node| node.label.clone())
                        }
                        crate::service::GraphSubject::Route(source, destination) => {
                            let port_label = |port: &moiren_core::graph::PortId| {
                                graph
                                    .graph
                                    .nodes()
                                    .iter()
                                    .find(|node| {
                                        node.inputs()
                                            .iter()
                                            .chain(node.outputs())
                                            .any(|candidate| candidate.id() == *port)
                                    })
                                    .map(|node| node.id())
                                    .and_then(|owner| {
                                        graph.node(owner).map(|node| node.label.clone())
                                    })
                            };
                            let source = port_label(&source);
                            let destination = port_label(&destination);
                            match (source, destination) {
                                (Some(source), Some(destination)) => {
                                    Some(format!("{source} → {destination}"))
                                }
                                _ => None,
                            }
                        }
                        crate::service::GraphSubject::Graph => None,
                    };
                    let mut row = div()
                        .id(SharedString::from(format!("fault-{index}")))
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .when(subject != crate::service::GraphSubject::Graph, |row| {
                            row.cursor_pointer().on_click({
                                let this = this.clone();
                                move |_, _, cx| {
                                    this.update(cx, |shell, cx| {
                                        shell.inspector_open = true;
                                        match subject {
                                            crate::service::GraphSubject::Node(node) => {
                                                shell.selected_route = None;
                                                shell.selected_node = Some(node);
                                            }
                                            crate::service::GraphSubject::Route(
                                                source,
                                                destination,
                                            ) => {
                                                shell.selected_node = None;
                                                shell.selected_route = Some((source, destination));
                                            }
                                            crate::service::GraphSubject::Graph => {}
                                        }
                                        cx.notify();
                                    });
                                }
                            })
                        });
                    row = row
                        .child(
                            div()
                                .text_color(theme::DANGER)
                                .font_weight(FontWeight::SEMIBOLD)
                                // Causes and hints are English catalog keys
                                // produced by the service; translation happens
                                // here so a locale switch re-renders stored
                                // faults, and unknown prose passes through.
                                .child(t!("Error: %{cause}", cause = t!(fault.cause.as_str()))),
                        )
                        .when_some(location, |row, location| {
                            row.child(
                                div().text_color(theme::TEXT).child(t!(
                                    "Channel: %{location}",
                                    location = location.as_str()
                                )),
                            )
                        })
                        .when_some(fault.hint.as_deref(), |row, hint: &str| {
                            row.child(
                                div()
                                    .text_color(theme::TEXT)
                                    .child(t!("Hint: %{hint}", hint = t!(hint))),
                            )
                        });
                    list = list.child(row.gpui_test_observe());
                }
                if let Some(error) = other {
                    // Dispatch/persistence diagnostics may be plain prose; a
                    // catalog miss renders the source text unchanged.
                    let message = t!(error.as_str()).into_owned();
                    list = list.child(
                        div()
                            .id("error-strip-message")
                            .text_color(theme::DANGER)
                            .child(message),
                    );
                }
                list.child(
                    div()
                        .pt(px(2.))
                        .text_color(theme::TEXT)
                        .child(t!("Diagnostics: %{log_path}", log_path = log_path)),
                )
                .gpui_test_observe()
            })
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
            .child(chrome::toolbar_for_page(self, cx))
            .when_some(error_strip, |view, error_strip| view.child(error_strip))
            .child(
                div()
                    .id("workspace-row")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(match self.presentation.page {
                        Page::Graph => graph::render_graph_page(self, cx),
                        Page::Mixer => mixer::meter_frame::page(self),
                        Page::Matrix => matrix::page(self, cx),
                        Page::Devices => devices::page(self, cx),
                    })
                    .child(sidebar::inspector_host(self, cx)),
            )
            .child(chrome::status_for_page(self, cx))
            // Clicking anywhere outside an open inline editor commits it, so
            // saving never requires Enter. Editors stop presses inside their
            // own bounds from propagating; a press that reaches this root
            // listener therefore landed outside the active editor, and the
            // bubble phase runs it after whatever deeper handler the press
            // targeted.
            .on_mouse_down(MouseButton::Left, {
                let shell = cx.entity();
                move |_, _, cx| {
                    shell.update(cx, |shell, cx| {
                        if mixer::commit_active_editor(shell, cx) {
                            cx.notify();
                        }
                    });
                }
            })
            .when(self.project_menu, |v| v.child(project::menu(self, cx)))
            .when(self.settings_open, |v| {
                v.child(settings::overlay(self, window, cx))
            })
            .on_action(cx.listener(|shell, _: &app_view::Undo, window, cx| {
                shell.undo_redo_graph(false, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &app_view::Redo, window, cx| {
                shell.undo_redo_graph(true, window, cx);
            }))
    }
}

impl StudioShell {
    /// Stable identity for a selectable element (canvas node, wire, mixer
    /// strip, matrix cell) used by double-click detection.
    pub(super) fn element_key(kind: &str, id: impl std::hash::Hash) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        kind.hash(&mut hasher);
        id.hash(&mut hasher);
        hasher.finish()
    }

    /// Records a press on a selectable element and reports whether it closes
    /// a double-click pair with the previous press of the same element.
    pub(super) fn element_double_clicked(&mut self, key: u64) -> bool {
        let now = Instant::now();
        let double = self.last_element_press.is_some_and(|(previous, at)| {
            previous == key && now.duration_since(at) <= Duration::from_millis(400)
        });
        self.last_element_press = Some((key, now));
        double
    }

    /// Expands the inspector from a double click or an explicit open action;
    /// compact widths give its space back by closing the library.
    pub(super) fn open_inspector(&mut self) {
        self.inspector_open = true;
        if self.compact {
            self.sidebar_open = false;
        }
    }

    /// True when a text field anywhere in the shell owns focus: graph
    /// search, node rename, inspector editors, mixer send gains, and export
    /// fields all keep typing shortcuts (and their undo) for themselves.
    fn typing_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.node_search.read(cx).focus_handle(cx) == *focused
            || self.node_name.read(cx).focus_handle(cx) == *focused
            || self
                .compressor
                .inputs
                .values()
                .any(|input| input.read(cx).focus_handle(cx) == *focused)
            || self
                .mixer
                .send_inputs
                .values()
                .any(|input| input.read(cx).focus_handle(cx) == *focused)
            || self.export_controls.owns_focus(focused, cx)
    }

    /// Ctrl+Z / Ctrl+Y: step the service graph journal. Typing focus and
    /// modal settings keep the shortcut for themselves.
    fn undo_redo_graph(&mut self, redo: bool, window: &Window, cx: &mut Context<Self>) {
        let can_step = {
            let state = self.state.read(cx);
            if redo {
                state.snapshot.graph.can_redo
            } else {
                state.snapshot.graph.can_undo
            }
        };
        if !can_step || self.settings_open {
            return;
        }
        // A focused text field owns ctrl-z; undoing the graph out from under
        // a keystroke the user is typing would be a surprise.
        if let Some(focused) = window.focused(cx)
            && self.typing_focus(&focused, cx)
        {
            return;
        }
        self.state.update(cx, |state, _| {
            state.dispatch(if redo {
                AppRequest::RedoGraph
            } else {
                AppRequest::UndoGraph
            });
        });
        cx.notify();
    }

    /// Keeps Delete/Escape alive on the graph page: whenever no text field of
    /// the page (and no modal) owns focus, the canvas key handler gets it back.
    /// Without this, clicking any button or panel leaves keystrokes routed to
    /// a stale focus target and Delete appears dead.
    fn keep_graph_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open
            || self.presentation.page != Page::Graph
            || self.graph_focus.contains_focused(window, cx)
        {
            return;
        }
        let focused = window.focused(cx);
        let text_input_focused = focused.is_some_and(|focused| self.typing_focus(&focused, cx));
        if !text_input_focused {
            window.focus(&self.graph_focus, cx);
        }
    }

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
        if snapshot.phase == SessionPhase::Running {
            let source = graph
                .node_of_kind(NodeKind::Source)
                .map(|(node, _)| node.id());
            let sink = graph
                .node_of_kind(NodeKind::Sink)
                .map(|(node, _)| node.id());
            self.observation_nodes = source
                .zip(sink)
                .map(|(source, sink)| (snapshot.generation, source, sink));
        }
    }
}
