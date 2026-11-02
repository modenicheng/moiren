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
/// Neutral value-region fill for slider and fader tracks. Signal cyan stays
/// reserved for meters and status, so a lit track never reads as audio.
pub const TRACK_FILL: Hsla = hsla(0.597222, 0.151899, 0.309804, 1.0);

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
pub const NODE_SOURCE: Hsla = hsla(0.439203, 0.643725, 0.515686, 1.);
pub const NODE_DSP: Hsla = hsla(0.722741, 0.781022, 0.731373, 1.);
pub const NODE_SINK: Hsla = hsla(0.125581, 0.964126, 0.562745, 1.);
pub const NODE_UTILITY: Hsla = TEXT_FAINT;
pub const NODE_ANALYSIS: Hsla = SIGNAL_DEVICE;
pub const CANVAS_GRID: f32 = 24.;

// Graph node card metrics (design 18): header + tint rail + per-kind
// body + meter strip + port rails, all in world units.
pub mod card {
    pub const PADDING: f32 = 10.;
    pub const HEADER_H: f32 = 30.;
    pub const RAIL_W: f32 = 3.;
    pub const RADIUS: f32 = 6.;
    /// Card outline in window pixels, unscaled so stacked cards keep a
    /// crisp edge at every zoom. A card at rest or re-preparing wears the
    /// thin outline; selection and faults draw the heavier one so they
    /// stand out from the graph without any other change to the card.
    pub const OUTLINE_W: f32 = 1.5;
    pub const OUTLINE_REST_W: f32 = 1.;
    pub const ICON_SIZE: f32 = 16.;
    pub const TITLE_SIZE: f32 = 12.;
    pub const TITLE_WIDTH: f32 = 170.;
    /// Status glyph slots at the header's right edge; the badge is the
    /// error/loading marker, the expander opens the detail window.
    pub const BADGE_SIZE: f32 = 14.;
    pub const BODY_TOP: f32 = HEADER_H + 6.;
    /// Shell body: one detail line. Kinds with richer bodies extend the
    /// node's `body_height` and the ports follow.
    pub const BODY_H: f32 = 18.;
    /// Knob bodies: one dial row with label and value beside it.
    pub const KNOB_BODY_H: f32 = 44.;
    pub const DIAL_SIZE: f32 = 40.;
    pub const DIAL_TRACK_W: f32 = 3.;
    /// The dial's sweep in screen degrees (y down): from bottom-left over
    /// the top to bottom-right. Every knob turns through the same arc;
    /// where its value arc anchors is the control's `KnobSpec::origin`.
    pub const DIAL_SWEEP_START: f32 = 135.;
    pub const DIAL_SWEEP_DEG: f32 = 270.;
    /// Binding-chip bodies (source/sink cards, rack chains).
    pub const CHIP_BODY_H: f32 = 26.;
    pub const CHIP_H: f32 = 22.;
    pub const CHIP_ICON: f32 = 14.;
    /// Curve-thumbnail bodies (EQ response, dynamics transfer).
    pub const CURVE_BODY_H: f32 = 64.;
    /// The spectrum analyzer is the one card wider than the strips: a 4:3
    /// plot gives its 20 Hz .. 20 kHz axis room to breathe.
    pub const SPECTRUM_W: f32 = 360.;
    /// Its plot height, sized so the card stands as tall as a strip is wide
    /// (header, body, meter reserve and port margins add up to the strip
    /// width), which with `SPECTRUM_W` makes the card 4:3.
    pub const SPECTRUM_BODY_H: f32 = super::NODE_WIDTH
        - BODY_TOP
        - (METER_CHANNELS as f32 * METER_H + METER_CHANNEL_GAP)
        - 2. * PORT_BOTTOM;
    /// Matrix-preview cells.
    pub const MATRIX_CELL: f32 = 9.;
    pub const MATRIX_GAP: f32 = 2.;
    pub const METER_H: f32 = 3.;
    pub const METER_CHANNEL_GAP: f32 = 2.;
    pub const METER_CHANNELS: usize = 2;
    /// Base distance from the card top to the first port anchor; taller
    /// bodies push the port rail down instead of overlapping the meter.
    pub const PORT_Y: f32 = 80.;
    pub const PORT_SPACING: f32 = 18.;
    pub const PORT_BOTTOM: f32 = 14.;
    pub const HINT_SIZE: f32 = 9.;
    /// Neutral knob pointer (same neutral as slider thumbs).
    pub const THUMB: super::Hsla = gpui_kit::hsla(0., 0., 0.85098, 1.);
}

/// The user-selectable node tint palette (`GraphNodeState::tint` indexes
/// this). Deliberately drawn from the working palette so a user override
/// can never land off-theme; category tints remain the default.
pub fn node_tint(index: u8) -> Option<Hsla> {
    Some(match index {
        0 => PRIMARY,
        1 => SIGNAL_DEVICE,
        2 => NODE_SOURCE,
        3 => NODE_DSP,
        4 => NODE_SINK,
        5 => hsla(0.988_158, 0.703_704, 0.627_451, 1.), // pink
        6 => hsla(0.077_381, 0.825_397, 0.603_922, 1.), // orange
        7 => WARNING,
        _ => return None,
    })
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
