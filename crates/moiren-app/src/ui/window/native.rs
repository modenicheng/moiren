#[cfg(not(windows))]
pub(super) fn toggle_maximize(window: &gpui_kit::Window) -> anyhow::Result<()> {
    window.zoom_window();
    Ok(())
}

/// Monitor geometry at a physical screen point: the monitor rectangle, its
/// work area, and the DPI scale gpui uses for that monitor. Queried with the
/// same Win32 APIs gpui's displays use, so the values line up with
/// `cx.displays()`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn monitor_geometry_at(
    x: f64,
    y: f64,
) -> Option<(
    super::desktop::PhysicalRect,
    super::desktop::PhysicalRect,
    f64,
)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    fn rect(value: windows::Win32::Foundation::RECT) -> super::desktop::PhysicalRect {
        super::desktop::PhysicalRect {
            x: f64::from(value.left),
            y: f64::from(value.top),
            width: f64::from(value.right - value.left),
            height: f64::from(value.bottom - value.top),
        }
    }

    unsafe {
        let point = POINT {
            x: x as i32,
            y: y as i32,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).ok()?;
        Some((
            rect(info.rcMonitor),
            rect(info.rcWork),
            f64::from(dpi_x) / 96.,
        ))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn saved_maximized(window: &gpui_kit::Window) -> Option<bool> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetWindowPlacement, IsIconic, SW_SHOWMAXIMIZED, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
        },
    };
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // GPUI drops the maximized bit for minimized windows. Windows retains the
    // intended restore state in WINDOWPLACEMENT without changing the window.
    unsafe {
        GetWindowPlacement(hwnd, &mut placement).ok()?;
        Some(
            placement.showCmd == SW_SHOWMAXIMIZED.0 as u32
                || (IsIconic(hwnd).as_bool() && placement.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0),
        )
    }
}

#[cfg(all(test, windows, feature = "gpui-test"))]
#[allow(unsafe_code)]
mod tests {
    use crate::ui::window::test_harness::{finish_service, initial_projection, spawn_service};
    use crate::ui::window::{ActionAssets, UiState, desktop, register_fonts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{
            HTCAPTION, IsWindow, IsZoomed, SendMessageW, WM_CLOSE, WM_GETICON, WM_NCHITTEST,
        },
    };

    fn hwnd(window: &Window) -> anyhow::Result<HWND> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("window handle: {error}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("expected Win32 window");
        };
        Ok(HWND(raw.hwnd.get() as *mut _))
    }

    #[test]
    #[ignore = "opens a bounded native test window; no audio devices are opened"]
    fn native_chrome_toggles_maximize_hits_drag_regions_and_releases_closed_window() {
        let service = spawn_service();
        let handle = service.handle();
        let initial = initial_projection(&handle);
        let result = Arc::new(Mutex::new(None));
        let completion = result.clone();
        gpui_kit::application()
            .with_assets(ActionAssets)
            .run(move |cx| {
                gpui_kit::init(cx);
                register_fonts(cx).unwrap();
                let state = cx.new(|_| UiState::new(handle, initial));
                let tray = crate::ui::tray::AppTray::new().unwrap();
                let desktop = cx.new(|cx| desktop::Desktop::new(state, Some(tray), cx));
                cx.set_global(desktop::DesktopOwner {
                    _desktop: desktop.clone(),
                });
                desktop.update(cx, |desktop, cx| {
                    desktop.state_path = None;
                    desktop.saved = crate::project::WindowState::default();
                    desktop.show_window(cx).unwrap();
                });
                cx.spawn(async move |cx| {
                    let checked: anyhow::Result<()> = async {
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                        let window = cx.update(|cx| cx.windows()[0]);
                        let native = window.update(cx, |_, window, _| hwnd(window))??;
                        // These HWNDs belong only to this test's disposable windows.
                        // Native messages run outside window.update to avoid reentrant GPUI borrows.
                        unsafe {
                            for kind in [0, 1] {
                                anyhow::ensure!(
                                    SendMessageW(
                                        native,
                                        WM_GETICON,
                                        Some(windows::Win32::Foundation::WPARAM(kind)),
                                        None
                                    )
                                    .0 != 0,
                                    "window icon {kind} missing"
                                );
                            }
                        }
                        let was_maximized = unsafe { IsZoomed(native).as_bool() };
                        for expected in [!was_maximized, was_maximized] {
                            window.update(cx, |_, window, cx| {
                                window.render_frame(cx);
                                window.click("win-max", cx);
                            })?;
                            cx.background_executor()
                                .timer(Duration::from_millis(180))
                                .await;
                            anyhow::ensure!(
                                unsafe { IsZoomed(native).as_bool() } == expected,
                                "maximize must toggle to {expected}"
                            );
                        }
                        for id in ["titlebar-drag-leading", "titlebar-drag-trailing"] {
                            let mut position = window.update(cx, |_, window, cx| {
                                window.render_frame(cx);
                                window.hover(id, cx);
                                let center = window.find(id).bounds().center();
                                let scale = window.scale_factor();
                                windows::Win32::Foundation::POINT {
                                    x: (f32::from(center.x) * scale) as i32,
                                    y: (f32::from(center.y) * scale) as i32,
                                }
                            })?;
                            unsafe {
                                anyhow::ensure!(ClientToScreen(native, &mut position).as_bool());
                                let packed =
                                    ((position.y as u16 as u32) << 16) | position.x as u16 as u32;
                                let hit = SendMessageW(
                                    native,
                                    WM_NCHITTEST,
                                    None,
                                    Some(LPARAM(packed as isize)),
                                );
                                anyhow::ensure!(
                                    hit.0 == HTCAPTION as isize,
                                    "{id} must hit the native caption, got {}",
                                    hit.0
                                );
                            }
                        }
                        if !unsafe { IsZoomed(native).as_bool() } {
                            window.update(cx, |_, window, cx| window.click("win-max", cx))?;
                            cx.background_executor()
                                .timer(Duration::from_millis(180))
                                .await;
                        }
                        window.update(cx, |_, window, cx| window.click("win-min", cx))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(180))
                            .await;
                        let weak = cx.update(|cx| desktop.read(cx).shell.clone().unwrap());
                        unsafe {
                            SendMessageW(native, WM_CLOSE, None, None);
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(350))
                            .await;
                        anyhow::ensure!(weak.upgrade().is_none(), "closed shell is retained");
                        anyhow::ensure!(
                            !unsafe { IsWindow(Some(native)).as_bool() },
                            "native HWND was hidden instead of destroyed"
                        );
                        cx.update(|cx| desktop.update(cx, |desktop, cx| desktop.show_window(cx)))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await;
                        anyhow::ensure!(
                            cx.update(|cx| cx.windows().len()) == 1,
                            "tray owner must recreate one window"
                        );
                        let reopened = cx.update(|cx| cx.windows()[0]);
                        let native = reopened.update(cx, |_, window, _| hwnd(window))??;
                        anyhow::ensure!(
                            unsafe { IsZoomed(native).as_bool() },
                            "closing a minimized maximized window must retain maximization"
                        );
                        reopened.update(cx, |_, window, cx| window.click("win-close", cx))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(350))
                            .await;
                        anyhow::ensure!(
                            !unsafe { IsWindow(Some(native)).as_bool() },
                            "custom X must destroy the HWND"
                        );
                        cx.update(|cx| desktop.update(cx, |desktop, cx| desktop.show_window(cx)))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await;
                        let reopened = cx.update(|cx| cx.windows()[0]);
                        let native = reopened.update(cx, |_, window, _| hwnd(window))??;
                        unsafe {
                            SendMessageW(native, WM_CLOSE, None, None);
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(350))
                            .await;
                        anyhow::ensure!(
                            cx.update(|cx| cx.windows().is_empty()),
                            "native close must remove the GPUI window"
                        );
                        anyhow::ensure!(
                            !unsafe { IsWindow(Some(native)).as_bool() },
                            "native close must destroy the HWND"
                        );
                        Ok(())
                    }
                    .await;
                    *completion.lock().unwrap() = Some(checked);
                    cx.update(|cx| cx.quit());
                })
                .detach();
            });
        finish_service(service);
        result
            .lock()
            .unwrap()
            .take()
            .expect("native test completed")
            .unwrap();
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn toggle_maximize(window: &gpui_kit::Window) -> anyhow::Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsZoomed, SW_MAXIMIZE, SW_RESTORE, ShowWindowAsync},
    };

    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("native window handle: {error}"))?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        anyhow::bail!("expected a Win32 window");
    };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    // GPUI's Windows zoom only maximizes. Ask Windows for the actual state,
    // including Snap/system-menu changes, and retain its normal restore bounds.
    unsafe {
        let command = if IsZoomed(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        ShowWindowAsync(hwnd, command).ok()?;
    }
    Ok(())
}
