#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TrayTheme {
    Light,
    Dark,
}

pub(super) fn tray_rgba(theme: TrayTheme) -> &'static [u8] {
    match theme {
        TrayTheme::Light => include_bytes!(concat!(env!("OUT_DIR"), "/tray-light-32.rgba")),
        TrayTheme::Dark => include_bytes!(concat!(env!("OUT_DIR"), "/tray-dark-32.rgba")),
    }
}

pub(super) fn system_tray_theme() -> TrayTheme {
    #[cfg(windows)]
    {
        let setting = windows_registry::CURRENT_USER
            .open("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .and_then(|key| key.get_u32("SystemUsesLightTheme"));
        if setting == Ok(0) {
            return TrayTheme::Dark;
        }
    }
    TrayTheme::Light
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn set_window_icon(window: &gpui_kit::Window) -> anyhow::Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                GetSystemMetrics, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON, SM_CXSMICON,
                SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
            },
        },
        core::PCWSTR,
    };
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("native window handle: {error}"))?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return Ok(());
    };
    // GPUI has no Windows icon option. Shared resource handles live with the
    // module and must not be destroyed when a disposable window is dropped.
    unsafe {
        let module = HINSTANCE(GetModuleHandleW(None)?.0);
        for (kind, width, height) in [(0, SM_CXSMICON, SM_CYSMICON), (1, SM_CXICON, SM_CYICON)] {
            let icon = LoadImageW(
                Some(module),
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetrics(width),
                GetSystemMetrics(height),
                LR_SHARED,
            )?;
            SendMessageW(
                HWND(raw.hwnd.get() as *mut _),
                WM_SETICON,
                Some(WPARAM(kind)),
                Some(LPARAM(icon.0 as isize)),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_logos_are_transparent_strokes_not_solid_placeholder_blocks() {
        for theme in [TrayTheme::Light, TrayTheme::Dark] {
            let rgba = tray_rgba(theme);
            assert_eq!(rgba.len(), 32 * 32 * 4);
            let painted = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] > 0)
                .count();
            assert!(painted > 150 && painted < 700, "painted={painted}");
            assert_eq!(rgba[3], 0);
            assert!(
                rgba.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
            );
        }
        assert_ne!(tray_rgba(TrayTheme::Light), tray_rgba(TrayTheme::Dark));
    }

    #[test]
    fn application_icon_has_pale_rounded_background_and_purple_logo() {
        let rgba = include_bytes!(concat!(env!("OUT_DIR"), "/app-32.rgba"));
        assert_eq!(rgba[3], 0);
        assert!(rgba.as_chunks::<4>().0.contains(&[245, 243, 238, 255]));
        assert!(rgba.as_chunks::<4>().0.contains(&[112, 84, 179, 255]));
        let ico = include_bytes!(concat!(env!("OUT_DIR"), "/moiren.ico"));
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 9, 0]);
    }
}
