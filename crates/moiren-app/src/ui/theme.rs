//! Shared workstation tokens. Pages and the action library use one palette.

use gpui_kit::{Hsla, hsla};

pub mod controls;

// --- Accents ---------------------------------------------------------------
pub const PRIMARY: Hsla = controls::FOCUS;

// Capture side: input devices, application sources, live signal.
pub const SIGNAL_DEVICE: Hsla = hsla(0.522059, 0.857143, 0.533333, 1.0);

// --- Surfaces --------------------------------------------------------------
pub const WINDOW_BG: Hsla = controls::CANVAS;
pub const SURFACE: Hsla = controls::PANEL;
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

// --- Text ------------------------------------------------------------------
pub const TEXT: Hsla = controls::TEXT;
pub const TEXT_MUTED: Hsla = controls::SECONDARY;
pub const TEXT_FAINT: Hsla = controls::MUTED;

// --- Semantic status -------------------------------------------------------
pub const SUCCESS: Hsla = SIGNAL_DEVICE;
pub const WARNING: Hsla = hsla(0.120155, 0.964126, 0.562745, 1.0);
pub const DANGER: Hsla = controls::DANGER;

// --- Level meters ----------------------------------------------------------
/// Translucent overlay that darkens the transient zone between the bright
/// RMS body and the peak reading.
pub const METER_PEAK_ZONE: Hsla = hsla(0., 0., 0., 0.45);

// --- Spacing scale (4pt based) ---------------------------------------------
pub const SPACE_2: f32 = 8.;
pub const SPACE_3: f32 = 12.;
pub const SPACE_4: f32 = 16.;

// --- Studio geometry (Figma v0.1.0 Graph measurements) --------------------
pub const TOOLBAR_HEIGHT: f32 = 44.;
pub const LIBRARY_WIDTH: f32 = 210.;
/// Always-visible icon rail on the library's outer edge; also its collapsed form.
pub const LIBRARY_RAIL_WIDTH: f32 = 44.;
pub const INSPECTOR_WIDTH: f32 = 280.;
pub const NODE_WIDTH: f32 = 270.;
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
    pub const ICON_SIZE: f32 = 28.;
    pub const TITLE_WIDTH: f32 = 181.;
    pub const METER_HEIGHT: f32 = 2.;
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
