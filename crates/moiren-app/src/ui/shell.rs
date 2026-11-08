//! Disposable main window views over the application-owned service projection.

use std::time::{Duration, Instant};

pub(crate) use gpui_kit::prelude::*;
pub(crate) use gpui_kit::*;
pub(crate) use rust_i18n::t;

pub(crate) use super::actions;
pub(crate) use super::monitoring::MeterSubscriptions;
use super::pages::{self, Page};
pub(crate) use super::pages::{devices, graph, matrix, mixer, settings};
pub(crate) use super::state::UiState;
pub(crate) use super::theme;
pub(crate) use super::widgets::action_assets::ActionIcon;
pub(crate) use super::widgets::action_assets::{ActionAssets, register_fonts};
pub(crate) use super::widgets::actions::{
    ActionButton, ButtonVariant, ControlSize, Density, IconButton,
};
pub(crate) use super::widgets::navigation::{WindowAction, WindowControlButton, WorkspaceTab};
pub(crate) use crate::service::{
    AppHandle, AppRequest, ServiceRuntime, SessionPhase, start_service,
};
pub(crate) use pages::graph::graph_canvas::Camera;

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
                KeyBinding::new("ctrl-z", actions::Undo, None),
                KeyBinding::new("ctrl-y", actions::Redo, None),
                KeyBinding::new("ctrl-shift-z", actions::Redo, None),
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
            cx.on_action::<actions::Quit>(|_, cx| cx.quit());
            // A terminal Ctrl+C must land on the same graceful path as the
            // in-app quit: the default handler would kill the process before
            // the quit observers flush autosave and the service retires.
            #[cfg(windows)]
            {
                crate::console::install_quit_handler();
                cx.spawn(async move |cx| {
                    crate::console::quit_requested().await;
                    log::info!(target: "moiren::app", "console interrupt: quitting gracefully");
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
    Ok(())
}

/// Root view: toolbar, active page, status bar.
pub struct StudioShell {
    pub(crate) state: Entity<UiState>,
    pub(crate) settings_open: bool,
    pub(crate) settings: settings::SettingsPageState,
    pub(crate) project_menu: bool,
    pub(crate) sidebar_open: bool,
    pub(crate) inspector_open: bool,
    pub(crate) compact: bool,
    pub(crate) density: Density,
    /// Live preference values plus the quiet-save machinery (design 20 §4).
    pub(crate) preferences: preferences::PreferenceStore,
    /// Recovery autosave: where the working document is written, the state it
    /// was last saved from, and the in-flight debounce/write machinery.
    pub(crate) autosave: autosave::Autosave,
    pub(crate) devices: devices::DevicesPageState,
    pub(crate) selected_route: Option<(moiren_core::graph::PortId, moiren_core::graph::PortId)>,
    pub(crate) meters: MeterSubscriptions,
    pub(crate) node_meter_points:
        std::rc::Rc<std::cell::RefCell<std::collections::BTreeSet<moiren_core::graph::NodeId>>>,
    pub(crate) project_io: project::ProjectIo,
    #[cfg(feature = "gpui-test")]
    pub(crate) test_page: Option<Entity<PageCell>>,
    pub(crate) presentation: WindowPresentation,
    pub(crate) camera: Camera,
    pub(crate) node_positions: std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    pub(crate) node_drag: Option<(moiren_core::graph::NodeId, f32, f32)>,
    /// Multi-selection on the graph canvas; `selected_node` is the primary
    /// (last-clicked) member that drives the inspector.
    pub(crate) selected_nodes: std::collections::BTreeSet<moiren_core::graph::NodeId>,
    /// Active rubber-band drag; see [`graph::SelectionDrag`].
    pub(crate) selection_drag: Option<graph::SelectionDrag>,
    pub(crate) graph_focus: FocusHandle,
    pub(crate) connection_drag: Option<graph::ConnectionDrag>,
    /// The open canvas context menu, over a node or the background.
    pub(crate) context_menu: Option<graph::ContextMenu>,
    /// A right press on the canvas awaiting its release: within the drag
    /// threshold the release opens the context menu, beyond it the stroke
    /// was a pan.
    pub(crate) right_press: Option<(f32, f32)>,
    /// Snap alignment guides for the in-flight card drag; painted by the
    /// canvas layer, cleared when the gesture ends.
    pub(crate) drag_guides: Option<pages::graph::graph_canvas::DragGuides>,
    /// The snap offset the in-flight card drag has baked into
    /// `node_positions` so far; the threshold measures against the drag's
    /// free path by undoing it, so a snapped card lets go once the cursor
    /// leaves the threshold. Zero outside a drag.
    pub(crate) node_drag_snap: (f32, f32),
    /// Active card-knob value drag: node, control, press y, press value.
    pub(crate) knob_drag: Option<(
        moiren_core::graph::NodeId,
        pages::graph::graph_canvas::KnobControl,
        f32,
        f64,
    )>,
    pub(crate) pending_node_spawn: Option<graph::PendingNodeSpawn>,
    pub(crate) node_search: Entity<component::input::InputState>,
    pub(crate) node_name: Entity<component::input::InputState>,
    /// Node whose label is being edited inline in the inspector.
    pub(crate) node_name_editing: Option<moiren_core::graph::NodeId>,
    pub(crate) hand_tool: bool,
    pub(crate) matrix: matrix::MatrixPageState,
    pub(crate) mixer_scroll: ScrollHandle,
    /// The shared inspector panel's scroll (graph/matrix/device inspectors
    /// are mutually exclusive), plus the other cached panels' scroll
    /// containers: their offsets ride the panel keys, since a cached panel
    /// replays baked paint coordinates until its key changes.
    pub(crate) inspector_scroll: ScrollHandle,
    pub(crate) library_scroll: ScrollHandle,
    pub(crate) library_rail_scroll: ScrollHandle,
    pub(crate) binding_menu_scroll: ScrollHandle,
    /// The dock pane's own scroll: docked strips key their cached paint on
    /// this offset, so a dock scroll rebuilds them instead of leaving the
    /// main row's stale coordinates on screen.
    pub(crate) dock_scroll: ScrollHandle,
    pub(crate) mixer: mixer::MixerPresentation,
    /// Render/frame instrumentation for the whole window (shell, chrome,
    /// every page), written by ui_capture and the gpui-test assertions.
    /// Lives on the shell because its scope is the window, not the mixer.
    #[cfg(feature = "gpui-test")]
    pub(crate) trace: std::cell::RefCell<performance::MeterTrace>,
    pub(crate) mixer_geometry: mixer::page::Geometry,
    pub(crate) toolbar_chrome: Entity<chrome::ToolbarChrome>,
    pub(crate) status_chrome: Entity<chrome::StatusbarChrome>,
    pub(crate) mixer_frame: Entity<mixer::page::MixerMeterLayer>,
    pub(crate) graph_meter_frame: Entity<graph::page::GraphMeterLayer>,
    pub(crate) graph_controls: Entity<graph::page::GraphPanelHost>,
    /// The graph canvas's most recent projected frame, shared with the meter
    /// frame's overlay paint. Refreshed by the canvas prepaint on cache
    /// rebuilds; between them the geometry the cached canvas painted stands.
    pub(crate) graph_canvas_frame: pages::graph::graph_canvas_view::PreparedFrame,
    /// Window rects of the graph page's HUD overlays (editing toolbar,
    /// overview, library handle), registered by the cached panel's probes
    /// and carved out of the meter frame's paint so live bars stay under
    /// them.
    pub(crate) graph_overlays: pages::graph::graph_canvas::OverlayRects,
    /// The meter frame's copy of the spectrum curve it is painting: the
    /// subscription table cannot stay borrowed across the paint, and a
    /// per-frame allocation would be a wakeup cost of its own.
    pub(crate) spectrum_scratch: Vec<f32>,
    /// The canvas card under the pointer, tracked on idle moves for the
    /// fault tooltip the meter frame paints; never part of a panel key.
    pub(crate) hovered_node: Option<moiren_core::graph::NodeId>,
    /// The mixer strip under the pointer, tracked by the strip's hover
    /// listener for the fault banner the meter frame paints; the graph
    /// field's twin, likewise never part of a panel key.
    pub(crate) hovered_strip: Option<moiren_core::graph::NodeId>,
    /// The endpoint menu hanging off a source/sink card's binding chip.
    pub(crate) binding_menu: Option<graph::BindingMenu>,
    /// The fault detail popover hanging off a faulted object's "!" marker
    /// (graph badge, mixer strip badge, status bar summary).
    pub(crate) fault_popover: Option<fault_popover::FaultPopover>,
    pub(crate) compressor: graph::CompressorControls,
    pub(crate) meter_points: std::rc::Rc<std::cell::Cell<(bool, bool)>>,
    /// Why this window currently needs display-rate frames (design 17 Tier
    /// H): sources register while active, and the control-side poll is the
    /// complement.
    pub(crate) frame_demand: hot::FrameDemand,
    /// Where the inspector's meter tracks sit; registered by the inspector's
    /// canvases each frame and read by the sidebar hot layer.
    pub(crate) inspector_meters: sidebar::InspectorMeterFrame,
    pub(crate) inspector_meter_layer: Entity<sidebar::InspectorMeterLayer>,
    pub(crate) inspector_panel: Entity<sidebar::InspectorPanelHost>,
    pub(crate) library_panel: Entity<sidebar::LibraryPanelHost>,
    pub(crate) library_rail_panel: Entity<sidebar::LibraryRailPanelHost>,
    pub(crate) observation_nodes: Option<(
        crate::service::SessionGeneration,
        moiren_core::graph::NodeId,
        moiren_core::graph::NodeId,
    )>,
    pub(crate) canvas_size: Option<(f32, f32)>,
    pub(crate) selected_node: Option<moiren_core::graph::NodeId>,
    /// Screen-space anchor of an active pan drag (window coordinates).
    pub(crate) pan_grab: Option<(f32, f32)>,
    /// Left background clicks deselect only on release; a pan keeps selection.
    pub(crate) background_click: Option<(f32, f32)>,
    /// Last selectable-element press (target hash + time) for double-click
    /// detection: single clicks select, double clicks also open the inspector.
    pub(crate) last_element_press: Option<(u64, Instant)>,
    pub(crate) desktop: Option<WeakEntity<desktop::Desktop>>,
    pub(crate) window_state: crate::project::WindowState,
    /// Precision-touchpad subclass state (Windows pointer input). Held for
    /// its Drop: unregisters the Win32 subclass when the shell goes away.
    #[cfg(windows)]
    #[allow(dead_code)]
    pub(crate) touchpad: super::touchpad::TouchpadRegistration,
}

impl StudioShell {
    pub(crate) fn new(
        state: Entity<UiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
        desktop: Option<WeakEntity<desktop::Desktop>>,
    ) -> Self {
        component::Theme::change(component::ThemeMode::Dark, Some(window), cx);
        crate::ui::theme::apply_component_tokens(cx);
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
        let preferences = crate::project::load_preferences().unwrap_or_default();
        let density = match preferences.density {
            crate::project::UiDensity::Compact => Density::Compact,
            _ => Density::Comfortable,
        };
        let settings = settings::SettingsEditor::new(&preferences, cx);
        let export_controls = devices::ExportControls::new(window, cx);
        let mixer_geometry = mixer::page::Geometry::default();
        let graph_canvas_frame = pages::graph::graph_canvas_view::PreparedFrame::default();
        let shell_entity = cx.entity();
        let mixer_meters = std::rc::Rc::new(std::cell::RefCell::new(
            mixer::page::MixerMeters::new(shell_entity.downgrade(), mixer_geometry.clone()),
        ));
        let mixer_frame = cx.new(|_| mixer::page::MixerMeterLayer::new(mixer_meters));
        let graph_meters = std::rc::Rc::new(std::cell::RefCell::new(
            graph::page::GraphMeters::new(shell_entity.downgrade(), graph_canvas_frame.clone()),
        ));
        let graph_meter_frame = cx.new(|_| graph::page::GraphMeterLayer::new(graph_meters));
        let inspector_meters = sidebar::InspectorMeterFrame::default();
        let inspector_meter_layer = cx.new(|_| {
            sidebar::InspectorMeterLayer::new(std::rc::Rc::new(std::cell::RefCell::new(
                sidebar::InspectorMeters::new(shell_entity.downgrade(), inspector_meters.clone()),
            )))
        });
        let devices_panel = cx.new(|cx| {
            devices::DevicesPanelHost::new(devices::DevicesPanel, shell_entity.clone(), cx)
        });
        let settings_panel = cx.new(|cx| {
            settings::SettingsPanelHost::new(settings::SettingsPanel, shell_entity.clone(), cx)
        });
        let inspector_panel = cx.new(|cx| {
            sidebar::InspectorPanelHost::new(sidebar::InspectorPanel, shell_entity.clone(), cx)
        });
        let library_panel = cx.new(|cx| {
            sidebar::LibraryPanelHost::new(sidebar::LibraryPanel, shell_entity.clone(), cx)
        });
        let library_rail_panel = cx.new(|cx| {
            sidebar::LibraryRailPanelHost::new(sidebar::LibraryRailPanel, shell_entity.clone(), cx)
        });
        let graph_controls = cx.new(|child_cx| {
            graph::page::GraphPanelHost::new(
                graph::page::GraphPanel,
                shell_entity.clone(),
                child_cx,
            )
        });
        let toolbar_chrome =
            cx.new(|cx| chrome::ToolbarChrome::new(chrome::ToolbarPanel, shell_entity.clone(), cx));
        let status_chrome = cx.new(|cx| {
            chrome::StatusbarChrome::new(chrome::StatusbarPanel, shell_entity.clone(), cx)
        });
        let mut shell = Self {
            state,
            settings_open: false,
            settings: settings::SettingsPageState {
                editor: settings,
                menu: None,
                scroll: ScrollHandle::new(),
                panel: settings_panel,
            },
            project_menu: false,
            sidebar_open: true,
            inspector_open: true,
            compact: false,
            density,
            preferences: preferences::PreferenceStore {
                values: preferences,
                path: crate::project::preferences_path().ok(),
                save: None,
                save_revision: 0,
                quitting: Default::default(),
            },
            autosave: autosave::Autosave {
                path: crate::project::autosave_path().ok(),
                ..Default::default()
            },
            devices: devices::DevicesPageState {
                device_tab: devices::DeviceTab::Outputs,
                selected_device: None,
                export_controls,
                device_list_scroll: ScrollHandle::new(),
                device_override_scroll: ScrollHandle::new(),
                exports_scroll: ScrollHandle::new(),
                panel: devices_panel,
            },
            selected_route: None,
            meters,
            node_meter_points: Default::default(),
            project_io: project::ProjectIo {
                path: None,
                name: t!("Untitled session").into(),
                message: None,
                busy: false,
                pending: None,
            },
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
            context_menu: None,
            right_press: None,
            drag_guides: None,
            node_drag_snap: (0., 0.),
            knob_drag: None,
            pending_node_spawn: None,
            node_search,
            node_name,
            node_name_editing: None,
            hand_tool: false,
            matrix: matrix::MatrixPageState::default(),
            mixer_scroll: ScrollHandle::new(),
            inspector_scroll: ScrollHandle::new(),
            library_scroll: ScrollHandle::new(),
            library_rail_scroll: ScrollHandle::new(),
            binding_menu_scroll: ScrollHandle::new(),
            dock_scroll: ScrollHandle::new(),
            mixer: mixer::MixerPresentation::default(),
            #[cfg(feature = "gpui-test")]
            trace: Default::default(),
            mixer_geometry,
            toolbar_chrome,
            status_chrome,
            mixer_frame,
            graph_meter_frame,
            graph_controls,
            graph_canvas_frame,
            graph_overlays: Default::default(),
            spectrum_scratch: Vec::new(),
            hovered_node: None,
            hovered_strip: None,
            binding_menu: None,
            fault_popover: None,
            compressor: graph::CompressorControls::default(),
            meter_points: Default::default(),
            frame_demand: Default::default(),
            inspector_meters,
            inspector_meter_layer,
            inspector_panel,
            library_panel,
            library_rail_panel,
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
                shell.project_io.message = Some(error.to_string());
            }
            shell.project_io.path = shell.preferences.values.last_project_path.clone();
        }
        // The restored startup state is already on disk (autosave or project);
        // the first edit, not the launch, owns the first autosave write.
        shell.autosave.stamp = autosave::AutosaveStamp::capture(&shell, cx);
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
        // Alt+Tab (or any focus steal) mid-gesture loses the mouse-up, and a
        // grab armed on an inactive window stays armed: cancel on
        // deactivation, since the platform will never complete the stroke.
        // Registration fires this once immediately; with no gesture in
        // flight the cancel is a no-op.
        cx.observe_window_activation(window, |shell, window, cx| {
            if !window.is_window_active() {
                shell.cancel_active_gesture(cx);
            }
        })
        .detach();
        // Quit observers run before GPUI drops the windows. Move the latest
        // write into shutdown ownership: it also owns every earlier queued
        // write, and must finish after the shell entity is gone.
        cx.on_app_quit(|shell, cx| {
            shell
                .preferences
                .quitting
                .store(true, std::sync::atomic::Ordering::Release);
            let pending = shell.preferences.save.take();
            let autosave = shell.autosave_flush(cx);
            async move {
                if let Some(pending) = pending {
                    pending.await;
                }
                if let Some(autosave) = autosave {
                    autosave.await;
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
                        shell.poll_autosave(cx);
                        let meters_changed = !shell.frames_wanted(cx) && shell.pump_meters(cx);
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

    /// Whether the window may animate right now: visible, presented, settings
    /// closed, session running, and on a page that mounts meter hot layers.
    /// Page membership stands in for mounting — the Graph and Mixer pages are
    /// where meter layers exist — because leftover meter demand on a page
    /// with no hot layer must not starve the control-side poll.
    pub(crate) fn frames_eligible(&self, cx: &App) -> bool {
        let snapshot = &self.state.read(cx).snapshot;
        self.presentation.visible
            && !self.presentation.minimized
            && !self.settings_open
            && matches!(self.presentation.page, Page::Graph | Page::Mixer)
            && snapshot.phase == SessionPhase::Running
    }

    /// Whether the window should keep taking display-rate frames: something
    /// registered demand with the arbiter and the window may animate. While
    /// false the vsync loop stands down and the control-side poll alone keeps
    /// the observations current, so a fully silent session costs no animation
    /// frames at all.
    pub(crate) fn frames_wanted(&self, cx: &App) -> bool {
        // A graph gesture bypasses the meter eligibility gate: the canvas is
        // interactive whether or not a session runs and every meter rests.
        // Only the page that mounts the canvas may speak for a gesture,
        // though: a grab that outlived the page (some lifecycle path missed
        // the mouse-up) must not starve the control-side poll, which is the
        // only meter pump the pages without hot layers have.
        (self.drag_active() && self.presentation.page == Page::Graph)
            || (self.frames_eligible(cx) && self.frame_demand.any())
    }

    /// Whether a graph canvas gesture is in flight (rubber band, pan, node
    /// or connection drag).
    pub(crate) fn drag_active(&self) -> bool {
        self.selection_drag.is_some()
            || self.pan_grab.is_some()
            || self.node_drag.is_some()
            || self.connection_drag.is_some()
            || self.knob_drag.is_some()
    }

    /// Whether any observed meter still moves: decaying ballistics, a held
    /// peak marker, fresh readings above the scale bottom, or a spectrum
    /// curve still falling toward its floor all keep the display short of
    /// rest.
    pub(crate) fn meters_in_motion(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        !self.meters.at_rest() || !state.node_meters.at_rest() || !state.node_spectra.at_rest()
    }

    /// Refresh every meter observation once: reads the latest windows,
    /// advances display ballistics, and reconciles subscriptions with the
    /// current visibility. Called per display frame while
    /// [`Self::frames_wanted`] holds, and from the slower
    /// control-side loops otherwise. Returns whether a display-bearing
    /// reading changed — raw levels, status, or a freshly latched clip;
    /// ballistics motion alone does not, because animation frames (not
    /// notify) carry it.
    pub(super) fn pump_meters(&mut self, cx: &mut Context<Self>) -> bool {
        // A gesture in flight proves the window is interactive even when the
        // platform's visibility flag is stale (a full occlusion followed by
        // a fast alt-tab can leave it false): the meter updates must not
        // blank out mid-drag, or the readings freeze at the publication
        // cadence.
        let visible = self.frames_eligible(cx) || self.drag_active();
        let points = self.meter_points.get();
        // The legacy monitor meters only display where the strip has no node
        // level topic: a managed strip's meter comes from NodeMeters, and
        // the pre-fader capture tap carries signal even when the fader is
        // muted. Subscribing it for a display nobody sees kept the whole
        // page's frame loop armed forever — the Graph page mounts no such
        // probes and stands down, the Mixer page burned a core.
        let (input_shown, output_shown) = self.legacy_meters_shown(cx, points);
        let meter_changed = self.meters.update_points(
            &self.state.read(cx).snapshot,
            visible && input_shown,
            visible && output_shown,
        );
        let node_meter_changed = self.update_node_meters(visible, cx);
        // Re-derive the arbiter's meter demand from the fresh ballistics: the
        // loop continues while anything moves, and the control-side poll
        // takes over the moment everything rests.
        self.frame_demand
            .set(hot::METER_MOTION, self.meters_in_motion(cx));
        self.sync_drag_demand();
        meter_changed || node_meter_changed
    }

    /// Re-derive the gesture demand source from the in-flight graph
    /// gestures. Called from the pump and at gesture boundaries: the
    /// mouse-down handlers must arm the loop immediately, not at the next
    /// control poll.
    pub(crate) fn sync_drag_demand(&mut self) {
        self.frame_demand.set(hot::DRAG, self.drag_active());
    }

    /// Whether the legacy input/output monitor meters actually display: their
    /// probe is mounted on screen AND the strip's node level topic is
    /// missing, so the meter paint falls back to the legacy reading. A
    /// managed strip renders from NodeMeters alone.
    pub(crate) fn legacy_meters_shown(&self, cx: &App, points: (bool, bool)) -> (bool, bool) {
        let managed = |id: moiren_core::graph::NodeId| {
            self.state
                .read(cx)
                .snapshot
                .signal_topics
                .node_levels
                .contains_key(&id)
        };
        match self.observation_nodes {
            Some((_, source, sink)) => (points.0 && !managed(source), points.1 && !managed(sink)),
            None => (false, false),
        }
    }

    pub(crate) fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        // Switching unmounts the canvas, so an in-flight gesture never sees
        // its closing mouse-up (the new page's background has no canvas
        // handlers); cancel it here or it stays armed across pages.
        self.cancel_active_gesture(cx);
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
    shell.preferences.path = None;
    shell.autosave.path = None;
    shell
}

// Re-exported so the window submodules keep picking it up through `use super::*`.
pub(crate) use crate::ui::observe::TestObserve;

pub(crate) mod autosave;
#[cfg(feature = "gpui-test")]
pub(crate) mod capture;
pub(crate) mod chrome;
pub(crate) mod desktop;
#[cfg(feature = "gpui-test")]
pub use capture::run_ui_capture;
pub(crate) mod controls;
pub(crate) mod fault_popover;
pub(crate) mod hot;
pub(crate) mod native;
pub(crate) mod panel;
#[cfg(feature = "gpui-test")]
pub(crate) mod performance;
pub(crate) mod preferences;
pub(crate) mod project;
pub(crate) mod sidebar;
#[cfg(any(test, feature = "gpui-test"))]
pub(crate) mod test_harness;
#[cfg(any(test, feature = "gpui-test"))]
pub use test_harness::{finish_service, initial_projection, spawn_service, wait_for, wait_until};
#[cfg(feature = "gpui-test")]
pub use test_harness::{settle_panels, shell_with_state, test_shell};
#[cfg(all(test, feature = "gpui-test"))]
pub(crate) mod tests;
pub(crate) mod tray_mixer;

impl Render for StudioShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "gpui-test")]
        {
            self.trace.borrow_mut().shell_renders += 1;
        }
        self.finish_settings_save(cx);
        self.keep_settings_focus(window, cx);
        self.finish_node_spawn(cx);
        graph::sync_binding_menu(self, cx);
        fault_popover::sync(self, cx);
        self.sync_node_name(window, cx);
        self.keep_graph_focus(window, cx);
        self.sync_export_controls(window, cx);
        self.sync_mixer_controls(window, cx);
        self.sync_compressor_controls(window, cx);
        // An iconic window never presents, but the platform vsync thread
        // still re-invalidates it every tick and such frames render
        // unconditionally, so a minimized window would otherwise keep paying
        // full tree construction forever (#38). The first render after
        // restore rebuilds the real tree.
        if native::window_is_iconic(window) {
            return div().into_any_element();
        }
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
        // Meter pumping lives in the pages' dedicated meter-frame views
        // (Graph and Mixer each own one), so animation frames no longer
        // notify this root. A drawn frame still rebuilds the root element;
        // the cached pages splice instead of re-rendering their contents.
        self.refresh_observation_nodes(cx);
        // Graph and Mixer demand is retained with their cached controls,
        // which re-register on a cache rebuild. Clearing on an ordinary
        // frame would unsubscribe meters that remain visible; other pages
        // hold no meter demand of their own.
        if !matches!(self.presentation.page, Page::Graph | Page::Mixer) {
            self.meter_points.set((false, false));
            self.node_meter_points.borrow_mut().clear();
        }
        if !visible {
            self.meters.update(&self.state.read(cx).snapshot, false);
            self.update_node_meters(false, cx);
        }
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
            .child(chrome::toolbar_for_page(self, window))
            .child(
                div()
                    .id("workspace-row")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    // Settings is a workspace page, not an overlay: it
                    // replaces the page + inspector column entirely, so a
                    // canvas gesture can never reach a hidden workspace.
                    .when(self.settings_open, |row| {
                        row.child(settings::mount(self, window))
                    })
                    .when(!self.settings_open, |row| {
                        row.child(match self.presentation.page {
                            Page::Graph => graph::page::mount(self, window),
                            Page::Mixer => mixer::page::page(self, window, cx),
                            Page::Matrix => matrix::page(self, cx),
                            Page::Devices => devices::mount(self, window, cx),
                        })
                        .child(sidebar::inspector_host(self, window, cx))
                    }),
            )
            .child(chrome::status_for_page(self, window))
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
            .when(self.context_menu.is_some(), |v| {
                v.child(graph::context_menu_overlay(self, window, cx))
            })
            .when(self.binding_menu.is_some(), |v| {
                v.child(graph::binding_menu_overlay(self, window, cx))
            })
            .when(self.settings.menu.is_some(), |v| {
                v.child(settings::menu_overlay(self, window, cx))
            })
            .when(self.fault_popover.is_some(), |v| {
                v.child(fault_popover::overlay(self, window, cx))
            })
            .on_action(cx.listener(|shell, _: &actions::Undo, window, cx| {
                shell.undo_redo_graph(false, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &actions::Redo, window, cx| {
                shell.undo_redo_graph(true, window, cx);
            }))
            .into_any_element()
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

    /// True when a text field anywhere in the shell owns focus: the graph
    /// search plus every family of [`Self::text_input_entities`] keep typing
    /// shortcuts (and their undo) for themselves.
    fn typing_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.node_search.read(cx).focus_handle(cx) == *focused
            || self
                .text_input_entities()
                .iter()
                .any(|input| input.read(cx).focus_handle(cx) == *focused)
    }

    /// Every text-input entity the shell renders nested in a cached panel:
    /// the inline node rename, the mixer's click-to-type families, the
    /// compressor parameter fields, and the export drafts. [`Self::typing_focus`]
    /// and the inspector panel's nested observers consume this single walk,
    /// so a new editor family cannot land in one enumeration site and miss
    /// the other (the shared root cause of audit SD-1/SD-2).
    pub(crate) fn text_input_entities(&self) -> Vec<Entity<component::input::InputState>> {
        let mut inputs = vec![self.node_name.clone()];
        inputs.extend(self.mixer.text_input_entities().cloned());
        inputs.extend(self.compressor.inputs.values().cloned());
        inputs.extend(self.devices.export_controls.input_entities());
        inputs
    }

    /// Every mixer-style slider slot the shell owns in one walk: the strip
    /// faders, the sends, the inspector's fader rows, and the compressor
    /// parameters (the tray flyout keeps its own slots in its own window,
    /// whose shortcuts never reach this shell). [`Self::cancel_active_drags`]
    /// consumes this single walk so a new slot family cannot land in one
    /// enumeration site and keep its drags un-cancellable — the same
    /// single-walk rule as [`Self::text_input_entities`].
    fn slider_slots(&self) -> impl Iterator<Item = &mixer::SliderSlot> {
        self.mixer
            .faders
            .values()
            .chain(self.mixer.sends.values())
            .chain(self.mixer.inspector_faders.values())
            .chain(self.compressor.sliders.values())
    }

    /// Cancels every in-flight slider drag so a journal step cannot be
    /// overwritten by its gesture (audit SD-7).
    fn cancel_active_drags(&self) {
        for slot in self.slider_slots() {
            if slot.dragging.get() {
                slot.cancel_drag();
            }
        }
    }

    /// Ctrl+Z / Ctrl+Y: step the service graph journal. Typing focus and
    /// modal settings keep the shortcut for themselves.
    pub(crate) fn undo_redo_graph(&mut self, redo: bool, window: &Window, cx: &mut Context<Self>) {
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
        // Undo outranks an in-flight fader drag (audit SD-7): the drag holds
        // the write-back down, so the undone value would stay invisible until
        // release and the release would then land the stale pointer value
        // over it. Cancelling first reopens the write-back and drops the
        // gesture's tail; moving the pointer again starts a fresh drag.
        self.cancel_active_drags();
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

    pub(crate) fn refresh_observation_nodes(&mut self, cx: &App) {
        use moiren_core::graph::NodeKind;
        let before = self.observation_nodes;
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
        if before != self.observation_nodes {
            // The probe flags belonged to the previous owners' canvases,
            // which mount only while eligible. The new owners re-register on
            // their rebuild in this same layout pass — eligibility rides in
            // their strip keys — so clearing here leaves no stale window.
            self.meter_points.set((false, false));
        }
    }
}
