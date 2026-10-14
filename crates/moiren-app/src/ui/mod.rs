//! gpui-kit presentation layer. The service runtime owns audio; this module
//! only projects snapshots into windows and dispatches [`AppRequest`]s back.
mod app_view;
mod pages;
mod scroll_policy;
mod state;
mod theme;
#[cfg(windows)]
#[allow(unsafe_code)]
mod touchpad;
mod tray;
mod widgets;
mod window;

pub mod signals;

pub use pages::Page;
pub use window::PageCell;
#[cfg(feature = "gpui-test")]
pub use window::shell_for_tests;
/// 32x32 RGBA tray/application icon: violet rounded square on dark.
/// Generated in code so the binary stays asset-free for the tray (the real
/// logo lands with the branding pass).
pub(crate) fn tray_icon_rgba() -> Vec<u8> {
    let size = 32;
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            // Rounded-rect coverage, 3px corner radius.
            let r = 3.0;
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = {
                let dx = (r - fx).max(fx - (size as f32 - r)).max(0.);
                let dy = (r - fy).max(fy - (size as f32 - r)).max(0.);
                dx * dx + dy * dy <= r * r
            };
            let (bg, a) = if inside {
                ([0x35, 0x2b, 0x52], 255) // deep violet
            } else {
                ([0, 0, 0], 0)
            };
            rgba.extend_from_slice(&[bg[0], bg[1], bg[2], a]);
        }
    }
    rgba
}

pub use state::{SelectionState, UiState};
pub use window::{WindowPresentation, run};
