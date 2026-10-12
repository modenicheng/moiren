use super::{AppTray, MainWindow, bindings};
use crate::service::{AppRequest, ServiceRuntime, SessionPhase, start_service};
use anyhow::Context;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// Presentation survives hiding independently of the service/audio lifetime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowPresentation {
    pub visible: bool,
    pub minimized: bool,
    pub page: i32,
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
    let service = start_service().context("starting application service")?;
    let handle = service.handle();
    let mut owned = OwnedService(Some(service));
    let result = run_event_loop(handle);
    let cleanup = owned.finish();
    result.and(cleanup)
}
fn run_event_loop(handle: crate::service::AppHandle) -> anyhow::Result<()> {
    let window = MainWindow::new().context("creating main window")?;
    let tray = AppTray::new().context("creating tray")?;
    let presentation = Rc::new(RefCell::new(WindowPresentation::default()));
    let state = Rc::new(RefCell::new(bindings::BindingState::default()));
    bindings::wire(&window, handle.clone(), state.clone());
    let weak = window.as_weak();
    let p = presentation.clone();
    let signal_state = state.clone();
    window.on_close_window(move || {
        if let Some(window) = weak.upgrade() {
            p.borrow_mut().hide_to_tray();
            bindings::hide_signals(&signal_state, &window);
            let _ = window.hide();
        }
    });
    let weak = window.as_weak();
    let p = presentation.clone();
    let signal_state = state.clone();
    window.window().on_close_requested(move || {
        if let Some(window) = weak.upgrade() {
            p.borrow_mut().hide_to_tray();
            bindings::hide_signals(&signal_state, &window);
            let _ = window.hide();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = window.as_weak();
    let p = presentation.clone();
    let signal_state = state.clone();
    window.on_minimize(move || {
        if let Some(window) = weak.upgrade() {
            p.borrow_mut().minimized = true;
            bindings::hide_signals(&signal_state, &window);
            window.window().set_minimized(true);
        }
    });
    let weak = window.as_weak();
    window.on_toggle_maximize(move || {
        if let Some(window) = weak.upgrade() {
            window
                .window()
                .set_maximized(!window.window().is_maximized());
        }
    });
    let h = handle.clone();
    window.on_exit_app(move || h.request_exit());
    let weak = window.as_weak();
    let p = presentation.clone();
    tray.on_show_window(move || {
        if let Some(window) = weak.upgrade() {
            window.window().set_minimized(false);
            p.borrow_mut().show();
            let _ = window.show();
        }
    });
    let h = handle.clone();
    let s = state.clone();
    tray.on_stop_session(move || bindings::dispatch(&s, &h, AppRequest::Stop));
    let h = handle.clone();
    tray.on_exit_app(move || h.request_exit());
    let weak_window = window.as_weak();
    let window_presentation = presentation.clone();
    let signal_state = state.clone();
    let service_handle = handle.clone();
    window
        .global::<super::Backend>()
        .on_presentation_changed(move || {
            if let (Some(window), Some(snapshot)) =
                (weak_window.upgrade(), service_handle.try_snapshot())
            {
                let presentation = {
                    let mut presentation = window_presentation.borrow_mut();
                    presentation.page = window.global::<super::Backend>().get_view();
                    presentation.minimized = window.window().is_minimized();
                    presentation.clone()
                };
                bindings::update_signals(
                    &signal_state,
                    &service_handle,
                    &snapshot,
                    &window,
                    &presentation,
                );
            }
        });
    // Weak component references avoid cycles; strong roots/timer remain owned
    // on this thread until explicit service Exited ends the event loop.
    bindings::dispatch(&state, &handle, AppRequest::RefreshCatalog);
    let p = presentation.clone();
    let weak = window.as_weak();
    let weak_tray = tray.as_weak();
    let h = handle.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            if let (Some(window), Some(tray), Some(snapshot)) =
                (weak.upgrade(), weak_tray.upgrade(), h.try_snapshot())
            {
                {
                    let mut presentation = p.borrow_mut();
                    presentation.page = window.global::<super::Backend>().get_view();
                    presentation.minimized = window.window().is_minimized();
                }
                bindings::update(&state, &snapshot, &window, &tray);
                let presentation = p.borrow().clone();
                bindings::update_signals(&state, &h, &snapshot, &window, &presentation);
                if snapshot.phase == SessionPhase::Exited {
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    window.show().context("showing main window")?;
    presentation.borrow_mut().show();
    tray.show().context("showing tray")?;
    slint::run_event_loop_until_quit().context("running Slint event loop")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construction_error_guard_requests_exit_and_joins() {
        let service = ServiceRuntime::spawn().unwrap();
        let handle = service.handle();
        {
            let _guard = OwnedService(Some(service));
        }
        assert_eq!(handle.try_snapshot().unwrap().phase, SessionPhase::Exited);
    }
}
