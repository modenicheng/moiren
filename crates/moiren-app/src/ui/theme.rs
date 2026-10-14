//! Moiren design tokens — the "Graph" deep theme, ported from theme.slint.
//!
//! Color-first dark theme (Figma "Graph" palette). Deliberately **no green**:
//! a running session reads as the device blue, caution/recording as gold.
//! See docs in the original theme.slint for provenance; keep raw literals out
//! of pages — components read `theme::…` only.
#![allow(dead_code)]

use gpui_kit::{Hsla, hsla};

pub mod controls;

// --- Accents ---------------------------------------------------------------
pub const PRIMARY: Hsla = hsla(0.711640, 0.496063, 0.750980, 1.0);
pub const PRIMARY_HOVER: Hsla = hsla(0.715686, 0.525773, 0.809804, 1.0);
pub const PRIMARY_PRESSED: Hsla = hsla(0.705479, 0.470968, 0.696078, 1.0);
/// Ink that sits on an accent fill (dark-scheme accents are light).
pub const ON_PRIMARY: Hsla = hsla(0.583333, 0.166667, 0.094118, 1.0);

// Capture side: input devices, application sources, live signal.
pub const SIGNAL_DEVICE: Hsla = hsla(0.583333, 0.728571, 0.725490, 1.0);
// Render side: output devices, test tone, recording.
pub const SIGNAL_RECORDER: Hsla = hsla(0.101515, 0.696203, 0.690196, 1.0);

// --- Surfaces --------------------------------------------------------------
pub const WINDOW_BG: Hsla = hsla(0.571429, 0.170732, 0.080392, 1.0); // canvas behind cards
pub const SURFACE: Hsla = hsla(0.574074, 0.183673, 0.096078, 1.0); // chrome: title bar, rail
pub const SURFACE_2: Hsla = hsla(0.574074, 0.152542, 0.115686, 1.0); // cards, graph nodes
pub const SURFACE_3: Hsla = hsla(0.569444, 0.153846, 0.152941, 1.0); // controls, inputs
pub const SURFACE_4: Hsla = hsla(0.571429, 0.142857, 0.192157, 1.0); // hover / pressed control
pub const OVERLAY_SCRIM: Hsla = Hsla {
    a: 0.58,
    ..WINDOW_BG
};

// --- Lines -----------------------------------------------------------------
pub const BORDER: Hsla = hsla(0.564103, 0.156627, 0.162745, 1.0);
pub const BORDER_STRONG: Hsla = hsla(0.562500, 0.142857, 0.219608, 1.0);
pub const HAIRLINE: Hsla = hsla(0.560606, 0.169231, 0.127451, 1.0);
pub const GRIP: Hsla = hsla(0.578431, 0.104294, 0.319608, 1.0);
/// Barely-there graph paper for the routing canvas.
pub const GRID_LINE: Hsla = Hsla {
    a: 0.03,
    ..WINDOW_BG
};
pub const FOCUS_RING: Hsla = Hsla { a: 0.55, ..PRIMARY };

// --- Text ------------------------------------------------------------------
pub const TEXT: Hsla = hsla(0.550000, 0.238095, 0.917647, 1.0);
pub const TEXT_MUTED: Hsla = hsla(0.563492, 0.132075, 0.688235, 1.0);
pub const TEXT_FAINT: Hsla = hsla(0.566667, 0.119048, 0.494118, 1.0);

// --- Semantic status -------------------------------------------------------
pub const SUCCESS: Hsla = SIGNAL_DEVICE;
pub const WARNING: Hsla = SIGNAL_RECORDER;
pub const DANGER: Hsla = hsla(0.014706, 0.621951, 0.678431, 1.0);

// --- Level meters ----------------------------------------------------------
pub const METER_LOW: Hsla = SIGNAL_DEVICE;
pub const METER_MID: Hsla = PRIMARY;
pub const METER_HIGH: Hsla = SIGNAL_RECORDER;

// --- Spacing scale (4pt based) ---------------------------------------------
pub const SPACE_1: f32 = 4.;
pub const SPACE_2: f32 = 8.;
pub const SPACE_3: f32 = 12.;
pub const SPACE_4: f32 = 16.;
pub const SPACE_5: f32 = 24.;
pub const SPACE_6: f32 = 32.;

// --- Studio geometry (Figma v0.1.0 Graph measurements) --------------------
pub const TOOLBAR_HEIGHT: f32 = 44.;
pub const LIBRARY_WIDTH: f32 = 210.;
pub const INSPECTOR_WIDTH: f32 = 280.;
pub const INSPECTOR_COLLAPSED_AT: f32 = 1280.;
pub const LIBRARY_COLLAPSED_AT: f32 = 1100.;
pub const NODE_WIDTH: f32 = 186.;
pub const NODE_WIDE_WIDTH: f32 = 226.;
pub const NODE_HEIGHT: f32 = 105.;
pub const CANVAS_GRID: f32 = 24.;

// Graph node card metrics (graph.slint GraphCardMetrics).
pub mod card {
    pub const PADDING: f32 = 16.;
    pub const GAP: f32 = 10.;
    pub const HEADER_HEIGHT: f32 = 32.;
    pub const METER_HEIGHT: f32 = 9.;
    pub const ROUTING_HEIGHT: f32 = 12.;
    pub const METER_Y: f32 = PADDING + HEADER_HEIGHT + GAP;
    pub const PORT_Y: f32 = METER_Y + METER_HEIGHT + GAP + ROUTING_HEIGHT / 2.;
}

// --- Radii -----------------------------------------------------------------
pub const RADIUS_SM: f32 = 4.;
pub const RADIUS_MD: f32 = 10.;
pub const RADIUS_LG: f32 = 14.;

// --- Typography ------------------------------------------------------------
pub const FONT_UI: &str = "Segoe UI";
pub const FONT_MONO: &str = "Cascadia Mono";
pub const TEXT_XS: f32 = 10.;
pub const TEXT_SM: f32 = 12.;
pub const TEXT_MD: f32 = 13.;
pub const TEXT_LG: f32 = 15.;

/// Soft fill for an accent (16% alpha over transparent), for chips/badges.
pub fn soft(hsla: Hsla) -> Hsla {
    Hsla { a: 0.16, ..hsla }
}

fn hsla_from_rgb(r: u8, g: u8, b: u8) -> Hsla {
    // Slint hex colors are sRGB; gpui's rgb() keeps components as given and
    // converts to linear on paint. Theme colors are hand-tuned sRGB hex from
    // the Figma palette, so pass through gpui's rgb() for identical rendering.
    Hsla::from(gpui_kit::rgb(
        0x1000000 * (r as u32) + 0x10000 * (g as u32) + 0x100 * (b as u32) + 0xff,
    ))
}
