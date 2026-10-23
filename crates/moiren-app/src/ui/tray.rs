//! System tray bridge via `tray-icon` (gpui has no tray API).
//!
//! Events are drained by the application owner even when no window exists.
//! The tray owns no audio state and only produces semantic actions.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use super::branding::{self, TrayTheme};

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Tray icon rectangle in physical screen pixels; anchors the mixer flyout
/// next to the notification area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayAnchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Semantic actions produced by tray interactions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayAction {
    ShowWindow,
    ShowMixer(TrayAnchor),
    StopSession,
    ExitApp,
}

struct TrayHandle {
    _icon: TrayIcon,
    stop_item: MenuItem,
}

impl TrayHandle {
    fn build(theme: TrayTheme) -> anyhow::Result<Self> {
        let show = MenuItem::with_id("show", "Show Moiren", true, None);
        let stop = MenuItem::with_id("stop", "Stop Session", true, None);
        let exit = MenuItem::with_id("exit", "Exit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &stop, &exit])
            .map_err(|e| anyhow::anyhow!("tray menu: {e}"))?;

        let icon = tray_icon::Icon::from_rgba(branding::tray_rgba(theme).to_vec(), 32, 32)
            .map_err(|e| anyhow::anyhow!("tray icon: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Moiren")
            .with_icon(icon)
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| anyhow::anyhow!("tray icon build: {e}"))?;
        Ok(Self {
            _icon: tray,
            stop_item: stop,
        })
    }
}

/// Owns the tray icon and dispatches drained events into a channel the UI
/// poll loop reads. No window or view is retained by the tray.
pub struct AppTray {
    _handle: TrayHandle,
    actions: mpsc::Receiver<TrayAction>,
    theme: TrayTheme,
    theme_check: Instant,
    session_active: std::cell::Cell<Option<bool>>,
}

impl AppTray {
    pub fn new() -> anyhow::Result<Self> {
        let theme = branding::system_tray_theme();
        let handle = TrayHandle::build(theme)?;
        let (tx, rx) = mpsc::channel::<TrayAction>();
        let tx_menu = tx.clone();
        let tx_click = tx;

        // Route menu picks into semantic actions.
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                "show" => TrayAction::ShowWindow,
                "stop" => TrayAction::StopSession,
                "exit" => TrayAction::ExitApp,
                _ => return,
            };
            let _ = tx_menu.send(action);
        }));

        // Single left-click opens the tray mixer flyout anchored to the icon;
        // double-click still shows the main window (Windows convention).
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let action = match event {
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    rect,
                    ..
                } => TrayAction::ShowMixer(TrayAnchor {
                    x: rect.position.x,
                    y: rect.position.y,
                    width: f64::from(rect.size.width),
                    height: f64::from(rect.size.height),
                }),
                TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                } => TrayAction::ShowWindow,
                _ => return,
            };
            let _ = tx_click.send(action);
        }));

        Ok(Self {
            _handle: handle,
            actions: rx,
            theme,
            theme_check: Instant::now(),
            session_active: std::cell::Cell::new(None),
        })
    }

    /// Drain pending tray actions; called from the UI clock.
    pub fn poll_actions(&mut self) -> Vec<TrayAction> {
        if self.theme_check.elapsed() >= Duration::from_secs(1) {
            self.theme_check = Instant::now();
            let theme = branding::system_tray_theme();
            if theme != self.theme {
                let updated =
                    tray_icon::Icon::from_rgba(branding::tray_rgba(theme).to_vec(), 32, 32)
                        .map_err(anyhow::Error::from)
                        .and_then(|icon| {
                            self._handle
                                ._icon
                                .set_icon(Some(icon))
                                .map_err(anyhow::Error::from)
                        });
                match updated {
                    Ok(()) => self.theme = theme,
                    Err(error) => eprintln!("Moiren tray theme: {error}"),
                }
            }
        }
        let mut drained = Vec::new();
        while let Ok(action) = self.actions.try_recv() {
            drained.push(action);
        }
        drained
    }

    /// Enable/disable the stop item with the session state.
    pub fn set_session_active(&self, active: bool) {
        if self.session_active.replace(Some(active)) != Some(active) {
            self._handle.stop_item.set_enabled(active);
        }
    }
}

impl Drop for AppTray {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}
