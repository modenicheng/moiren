//! Main window: owns the service guard, hides to tray instead of quitting,
//! and polls service snapshots on the UI clock.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::app_view;
use super::pages::{self, Page};
use super::state::UiState;
use super::theme;
use super::widgets::{WorkspaceTab, window_button};
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
    let initial = handle
        .try_snapshot()
        .expect("service publishes its initial snapshot at mailbox creation");
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        let state = cx.new(|_| UiState::new(handle.clone(), initial));
        state.update(cx, |s, _| s.dispatch(AppRequest::RefreshCatalog));

        let (_, window_entity) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Moiren")),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_min_size: Some(size(px(940.), px(600.))),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|cx| StudioShell::new(state, cx)),
        )
        .expect("failed to open main window");

        window_entity.update(cx, |shell, cx| shell.spawn_poll(cx));
        cx.on_action::<app_view::Quit>(|_, cx| cx.quit());
    });
    Ok(())
}

/// Root view: toolbar, active page, status bar.
pub struct StudioShell {
    state: Entity<UiState>,
    presentation: WindowPresentation,
    camera: Camera,
    selected_node: Option<String>,
    /// Screen-space anchor of an active pan drag (window coordinates).
    pan_grab: Option<(f32, f32)>,
}

impl StudioShell {
    fn new(state: Entity<UiState>, _cx: &mut Context<Self>) -> Self {
        let mut shell = Self {
            state,
            presentation: WindowPresentation::default(),
            camera: Camera::default(),
            selected_node: None,
            pan_grab: None,
        };
        shell.presentation.show();
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
                        if changed {
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
        cx.notify();
    }
}

impl Render for StudioShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = {
            let state = self.state.read(cx);
            let snapshot = &state.snapshot;
            snapshot
                .error
                .as_deref()
                .or(snapshot.catalog_error.as_deref())
                .unwrap_or("")
                .to_string()
        };

        div()
            .id("studio-shell")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::WINDOW_BG)
            .text_color(theme::TEXT)
            .font_family(theme::FONT_UI)
            // Toolbar band.
            .child(render_toolbar(self, cx))
            // Error strip.
            .when(!error.is_empty(), |shell| {
                shell.child(
                    div()
                        .id("error-strip")
                        .flex()
                        .flex_none()
                        .h(px(36.))
                        .px(px(theme::SPACE_3))
                        .items_center()
                        .bg(theme::SURFACE_3)
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::DANGER)
                        .child(error.to_string()),
                )
            })
            // Body: active page fills remaining space.
            .child(render_body(self, cx))
            // Status bar.
            .child(pages::engine_statusbar(self.state.read(cx)))
    }
}

fn render_toolbar(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let state = shell.state.read(cx);
    let active = state.snapshot.phase == SessionPhase::Running;
    let status_label = if active { "Running" } else { "Idle" };
    let status_color = if active {
        theme::PRIMARY
    } else {
        theme::TEXT_FAINT
    };
    let this = cx.entity();

    div()
        .id("app-toolbar")
        .flex()
        .flex_none()
        .h(px(theme::TOOLBAR_HEIGHT))
        .items_center()
        .justify_between()
        .px(px(theme::SPACE_2))
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            // Left cluster: brand + project selector.
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .text_size(px(theme::TEXT_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::PRIMARY)
                        .child("Moiren"),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::TEXT_MUTED)
                        .child("Main Session"),
                ),
        )
        // Center: workspace tabs.
        .child(
            div()
                .flex()
                .flex_none()
                .h_full()
                .items_center()
                .child(
                    WorkspaceTab::new("tab-graph", "Graph", "route")
                        .active(shell.presentation.page == Page::Graph)
                        .on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |shell, cx| shell.set_page(Page::Graph, cx));
                            }
                        }),
                )
                .child(
                    WorkspaceTab::new("tab-mixer", "Mixer", "wave")
                        .active(shell.presentation.page == Page::Mixer)
                        .on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |shell, cx| shell.set_page(Page::Mixer, cx));
                            }
                        }),
                )
                .child(WorkspaceTab::new("tab-matrix", "Matrix", "grid").enabled(false))
                .child(WorkspaceTab::new("tab-devices", "Devices", "devices").enabled(false)),
        )
        .child(
            // Right cluster: engine status + window buttons.
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::SPACE_1))
                        .px(px(theme::SPACE_2))
                        .h(px(24.))
                        .rounded(px(theme::RADIUS_SM))
                        .bg(theme::SURFACE_3)
                        .child(div().size(px(6.)).rounded_full().bg(status_color))
                        .child(
                            div()
                                .text_size(px(theme::TEXT_XS))
                                .text_color(status_color)
                                .child(status_label),
                        ),
                )
                .child(window_button(
                    "win-min",
                    "minimize",
                    false,
                    |_, window, _| {
                        window.minimize_window();
                    },
                ))
                .child(window_button("win-close", "close", true, |_, _, cx| {
                    cx.quit();
                })),
        )
}

fn render_body(shell: &StudioShell, cx: &mut Context<StudioShell>) -> impl IntoElement {
    match shell.presentation.page {
        Page::Graph => render_graph_page(shell, cx),
        Page::Mixer => pages::mixer_page(shell.state.read(cx)).into_any_element(),
    }
}

fn render_graph_page(shell: &StudioShell, cx: &mut Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let world = pages::world_from_snapshot(&state.snapshot);
    let camera = shell.camera;
    let selected = shell.selected_node.clone();
    let this = cx.entity();

    // Canvas view with interaction plumbed to the shell entity.
    let on_zoom = {
        let this = this.clone();
        move |(factor, _anchor, _bounds): (f32, Point<Pixels>, Bounds<Pixels>),
              _window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.camera.zoom_by(factor, (590., 380.));
                cx.notify();
            });
        }
    };
    let on_select_node = {
        let this = this.clone();
        move |id: &str, _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.selected_node = Some(id.to_string());
                cx.notify();
            });
        }
    };
    let on_bg_move = {
        let this = this.clone();
        move |(position, _bounds): (Point<Pixels>, Bounds<Pixels>),
              _window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                if let Some((gx, gy)) = shell.pan_grab {
                    let (dx, dy) = (f32::from(position.x) - gx, f32::from(position.y) - gy);
                    shell.camera.pan_by(dx, dy);
                    shell.pan_grab = Some((f32::from(position.x), f32::from(position.y)));
                    cx.notify();
                }
            });
        }
    };
    let on_bg_down = {
        let this = this.clone();
        move |(_event, _bounds): (MouseDownEvent, Bounds<Pixels>),
              window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.pan_grab = Some((
                    f32::from(window.mouse_position().x),
                    f32::from(window.mouse_position().y),
                ));
                shell.selected_node = None;
                cx.notify();
            });
        }
    };

    let on_camera_fit = {
        let this = this.clone();
        move |fitted: &Camera, _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                if !shell.camera.adjusted {
                    shell.camera = *fitted;
                    cx.notify();
                }
            });
        }
    };
    let canvas_view = pages::graph_canvas_view::graph_canvas_view(
        world,
        camera,
        selected.as_deref(),
        pages::graph_canvas_view::CanvasCallbacks {
            on_background_down: Box::new(on_bg_down),
            on_canvas_mouse_move: Box::new(on_bg_move),
            on_zoom: Box::new(on_zoom),
            on_select_node: std::rc::Rc::new(on_select_node),
            on_camera_fit: Box::new(on_camera_fit),
        },
    );

    div()
        .id("graph-page")
        .relative()
        .flex_1()
        .overflow_hidden()
        .child(canvas_view)
        .child(editing_toolbar(shell, &this))
        .into_any_element()
}

/// Floating Select/Pan toolbar + zoom controls, top-left of the canvas.
fn editing_toolbar(shell: &StudioShell, this: &Entity<StudioShell>) -> impl IntoElement {
    let zoom_label = format!("{}%", (shell.camera.zoom * 100.).round() as i32);
    div()
        .id("editing-toolbar")
        .absolute()
        .top(px(theme::SPACE_4))
        .left(px(theme::SPACE_4))
        .flex()
        .items_center()
        .gap(px(7.))
        .child(
            div()
                .id("zoom-out")
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::SURFACE_3)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT)
                .cursor_pointer()
                .hover(|s| s.bg(theme::SURFACE_4))
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.camera.zoom_by(1.0 / 1.05, (590., 380.));
                            cx.notify();
                        });
                    }
                })
                .child("−"),
        )
        .child(
            div()
                .id("zoom-label")
                .flex()
                .items_center()
                .justify_center()
                .w(px(64.))
                .h(px(30.))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::SURFACE_3)
                .border_1()
                .border_color(theme::BORDER)
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT)
                .child(zoom_label),
        )
        .child(
            div()
                .id("zoom-in")
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::SURFACE_3)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT)
                .cursor_pointer()
                .hover(|s| s.bg(theme::SURFACE_4))
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.camera.zoom_by(1.05, (590., 380.));
                            cx.notify();
                        });
                    }
                })
                .child("+"),
        )
        .child(
            div()
                .id("zoom-fit")
                .flex()
                .items_center()
                .justify_center()
                .px(px(theme::SPACE_2))
                .h(px(30.))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::SURFACE_3)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT)
                .cursor_pointer()
                .hover(|s| s.bg(theme::SURFACE_4))
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.camera.adjusted = false;
                            cx.notify();
                        });
                    }
                })
                .child("Fit"),
        )
}
