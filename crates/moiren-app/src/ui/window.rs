//! Main window: owns the service guard, hides to tray instead of quitting,
//! and polls service snapshots on the UI clock.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;

use super::app_view;
use super::state::UiState;
use crate::service::{
    AppHandle, AppRequest, AppSnapshot, ServiceRuntime, SessionPhase, start_service,
};

/// Presentation survives hiding independently of the service/audio lifetime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowPresentation {
    pub visible: bool,
    pub minimized: bool,
    pub page: Page,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    #[default]
    Session,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Session => "Session",
        }
    }
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
    let service = start_service()
        .map_err(|error| anyhow::anyhow!("starting application service: {error:?}"))?;
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
        state.update(cx, |_, cx| {
            cx.notify();
        });

        let (_, window_entity) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Moiren")),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|cx| StudioShell::new(state, cx)),
        )
        .expect("failed to open main window");

        // Poll the service on the UI clock. When it exits, quit the app.
        window_entity.update(cx, |shell, cx| {
            shell.spawn_poll(cx);
        });

        cx.on_action::<app_view::Quit>(|_, cx| cx.quit());
    });
    Ok(())
}

/// Root view: navigation chrome plus the active page.
pub struct StudioShell {
    state: gpui_kit::Entity<UiState>,
    presentation: WindowPresentation,
}

impl StudioShell {
    fn new(state: gpui_kit::Entity<UiState>, cx: &mut Context<Self>) -> Self {
        let mut shell = Self {
            state,
            presentation: WindowPresentation::default(),
        };
        shell.presentation.show();
        shell.request_initial_refresh(cx);
        shell
    }

    fn request_initial_refresh(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.dispatch(AppRequest::RefreshCatalog);
        });
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
}

impl Render for StudioShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = &self.state.read(cx).snapshot;
        div()
            .id("studio-shell")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(gpui_kit::base::ColorTokens::light().background)
            .text_color(gpui_kit::base::ColorTokens::light().foreground)
            .child(render_header(self, cx))
            .child(render_page(self, cx))
            .child(render_status_bar(snapshot))
    }
}

fn render_header(shell: &StudioShell, _cx: &Context<StudioShell>) -> impl IntoElement {
    div()
        .id("header")
        .flex()
        .items_center()
        .justify_between()
        .px_4()
        .h(px(40.))
        .border_b_1()
        .border_color(gpui_kit::base::ColorTokens::light().border)
        .child(
            div()
                .font_family("Segoe UI")
                .text_size(px(13.))
                .child(format!("Moiren — {}", shell.presentation.page.title())),
        )
        .child(
            div()
                .id("header-actions")
                .flex()
                .gap_2()
                .child(
                    gpui_kit::base::Button::new("minimize")
                        .child("—")
                        .on_click(|_, window, _cx| window.minimize_window()),
                )
                .child(
                    gpui_kit::base::Button::new("exit")
                        .child("Exit")
                        .on_click(|_, _, cx| cx.quit()),
                ),
        )
}

fn render_page(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    match shell.presentation.page {
        Page::Session => render_session_page(shell, cx),
    }
}

fn render_session_page(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    div()
        .id("session-page")
        .flex_1()
        .flex()
        .flex_col()
        .p_4()
        .gap_3()
        .child(
            div()
                .id("phase-row")
                .flex()
                .gap_4()
                .child(labelled("Phase", state.phase_text()))
                .child(labelled("Elapsed", &state.elapsed_text()))
                .child(labelled(
                    "Devices",
                    &snapshot.catalog.inputs.len().to_string(),
                )),
        )
        .child(
            gpui_kit::base::Button::new("start")
                .child(if state.pending_request {
                    "Working…"
                } else {
                    "Start Monitor"
                })
                .on_click({
                    let state = shell.state.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, _| {
                            state.dispatch(AppRequest::RefreshCatalog);
                        });
                    }
                }),
        )
}

fn render_status_bar(snapshot: &Arc<AppSnapshot>) -> impl IntoElement {
    let error = snapshot
        .error
        .as_deref()
        .or(snapshot.catalog_error.as_deref())
        .unwrap_or("");
    div()
        .id("status-bar")
        .flex()
        .items_center()
        .h(px(26.))
        .px_4()
        .border_t_1()
        .border_color(gpui_kit::base::ColorTokens::light().border)
        .text_size(px(12.))
        .child(if error.is_empty() {
            SharedString::from("Ready")
        } else {
            SharedString::from(error.to_string())
        })
}

fn labelled(name: &str, value: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_xs()
                .text_color(gpui_kit::base::ColorTokens::light().muted_foreground)
                .child(name.to_string()),
        )
        .child(div().text_sm().child(value.to_string()))
}
