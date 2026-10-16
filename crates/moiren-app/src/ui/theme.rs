//! Shared workstation tokens. Pages and the action library use one palette.
#![allow(dead_code)]

use gpui_kit::{Hsla, hsla};

pub mod controls;

// --- Accents ---------------------------------------------------------------
pub const PRIMARY: Hsla = controls::FOCUS;
pub const PRIMARY_HOVER: Hsla = hsla(0.715686, 0.525773, 0.809804, 1.0);
pub const PRIMARY_PRESSED: Hsla = hsla(0.705479, 0.470968, 0.696078, 1.0);
/// Ink that sits on an accent fill (dark-scheme accents are light).
pub const ON_PRIMARY: Hsla = hsla(0.583333, 0.166667, 0.094118, 1.0);

// Capture side: input devices, application sources, live signal.
pub const SIGNAL_DEVICE: Hsla = hsla(0.522059, 0.857143, 0.533333, 1.0);
// Render side: output devices, test tone, recording.
pub const SIGNAL_RECORDER: Hsla = hsla(0.591991, 0.939024, 0.678431, 1.0);

// --- Surfaces --------------------------------------------------------------
pub const WINDOW_BG: Hsla = controls::CANVAS;
pub const SURFACE: Hsla = controls::PANEL;
pub const SURFACE_2: Hsla = controls::PANEL;
pub const SURFACE_3: Hsla = controls::RAISED;
pub const SURFACE_4: Hsla = hsla(0.571429, 0.142857, 0.192157, 1.0); // hover / pressed control
pub const OVERLAY_SCRIM: Hsla = Hsla {
    a: 0.58,
    ..WINDOW_BG
};

// --- Lines -----------------------------------------------------------------
pub const BORDER: Hsla = controls::BORDER;
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
pub const TEXT: Hsla = controls::TEXT;
pub const TEXT_MUTED: Hsla = controls::SECONDARY;
pub const TEXT_FAINT: Hsla = controls::MUTED;

// --- Semantic status -------------------------------------------------------
pub const SUCCESS: Hsla = SIGNAL_DEVICE;
pub const WARNING: Hsla = hsla(0.120155, 0.964126, 0.562745, 1.0);
pub const DANGER: Hsla = controls::DANGER;

// --- Level meters ----------------------------------------------------------
pub const METER_LOW: Hsla = SIGNAL_DEVICE;
pub const METER_MID: Hsla = PRIMARY;
pub const METER_HIGH: Hsla = WARNING;

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
pub const NODE_WIDTH: f32 = 270.;
pub const NODE_WIDE_WIDTH: f32 = 270.;
pub const NODE_HEIGHT: f32 = 112.;
pub const NODE_SOURCE: Hsla = hsla(0.439203, 0.643725, 0.515686, 1.);
pub const NODE_DSP: Hsla = hsla(0.722741, 0.781022, 0.731373, 1.);
pub const NODE_SINK: Hsla = hsla(0.125581, 0.964126, 0.562745, 1.);
pub const NODE_UTILITY: Hsla = TEXT_FAINT;
pub const NODE_ANALYSIS: Hsla = SIGNAL_DEVICE;
pub const CANVAS_GRID: f32 = 24.;

// Graph node card metrics (graph.slint GraphCardMetrics).
pub mod card {
    pub const DECO_HEIGHT: f32 = 6.;
    pub const PADDING: f32 = 12.;
    pub const GAP: f32 = 10.;
    pub const HEADER_HEIGHT: f32 = 28.;
    pub const ICON_SIZE: f32 = 28.;
    pub const TITLE_WIDTH: f32 = 181.;
    pub const METER_HEIGHT: f32 = 2.;
    pub const ROUTING_HEIGHT: f32 = 12.;
    pub const METER_Y: f32 = 60.;
    pub const PORT_Y: f32 = 80.;
    pub const FADER_Y: f32 = 68.;
    pub const BALANCE_Y: f32 = 88.;
    pub const TRACK_WIDTH: f32 = 211.;
    pub const TRACK_HEIGHT: f32 = 4.;
    pub const VALUE_WIDTH: f32 = 25.;
    pub const TRACK_FILL: super::Hsla = gpui_kit::hsla(0.597222, 0.151899, 0.309804, 1.);
    pub const THUMB: super::Hsla = gpui_kit::hsla(0., 0., 0.85098, 1.);
}

// --- Radii -----------------------------------------------------------------
pub const RADIUS_SM: f32 = 4.;
pub const RADIUS_MD: f32 = 6.;
pub const RADIUS_LG: f32 = 8.;

// --- Typography ------------------------------------------------------------
pub const FONT_UI: &str = controls::FONT;
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
