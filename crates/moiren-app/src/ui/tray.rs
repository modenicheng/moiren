//! System tray bridge via `tray-icon` (gpui has no tray API).
//!
//! Windows: tray events arrive on a global crossbeam channel. We drain it on
//! the UI clock (the same 33ms poll that reads service snapshots), so no extra
//! event loop integration is needed. The tray owns no audio state; every
//! action is forwarded to the shell as a [`TrayAction`].

use std::sync::mpsc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Semantic actions produced by tray interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    StopSession,
    ExitApp,
}

struct TrayHandle {
    _icon: TrayIcon,
    stop_item: MenuItem,
}

impl TrayHandle {
    fn build() -> anyhow::Result<Self> {
        let show = MenuItem::with_id("show", "Show Moiren", true, None);
        let stop = MenuItem::with_id("stop", "Stop Session", true, None);
        let exit = MenuItem::with_id("exit", "Exit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &stop, &exit])
            .map_err(|e| anyhow::anyhow!("tray menu: {e}"))?;

        let icon = tray_icon::Icon::from_rgba(crate::ui::tray_icon_rgba(), 32, 32)
            .map_err(|e| anyhow::anyhow!("tray icon: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Moiren")
            .with_icon(icon)
            .build()
            .map_err(|e| anyhow::anyhow!("tray icon build: {e}"))?;
        Ok(Self {
            _icon: tray,
            stop_item: stop,
        })
    }
}

/// Owns the tray icon and dispatches drained events into a channel the UI
/// poll loop reads. Keeps `'static` handles so the icon survives hides.
pub struct AppTray {
    _handle: TrayHandle,
    actions: mpsc::Receiver<TrayAction>,
}

impl AppTray {
    pub fn new() -> anyhow::Result<Self> {
        let handle = TrayHandle::build()?;
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

        // Left-click / double-click shows the window (Windows convention).
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let show = matches!(
                event,
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                }
            );
            if show {
                let _ = tx_click.send(TrayAction::ShowWindow);
            }
        }));

        Ok(Self {
            _handle: handle,
            actions: rx,
        })
    }

    /// Drain pending tray actions; called from the UI clock.
    pub fn poll_actions(&mut self) -> Vec<TrayAction> {
        let mut drained = Vec::new();
        while let Ok(action) = self.actions.try_recv() {
            drained.push(action);
        }
        drained
    }

    /// Enable/disable the stop item with the session state.
    pub fn set_session_active(&self, active: bool) {
        self._handle.stop_item.set_enabled(active);
    }
}
