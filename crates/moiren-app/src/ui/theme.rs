//! Shared workstation tokens. Pages and the action library use one palette.

use gpui_kit::component;
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

// --- Stroke widths (design 21 §1.3) -----------------------------------------
/// A 1px rule: dividers, separators and underline strokes. (Borders drawn
/// through gpui's semantic `.border_1()`-family methods already carry this
/// width by construction and stay on that API — as do the 2px focus rings,
/// which is why no FOCUS_RING_W token exists yet.)
pub const HAIRLINE_W: f32 = 1.;
/// The accent bar that marks an active tab or a selected row.
pub const ACCENT_BAR_W: f32 = 2.;

// --- Text ------------------------------------------------------------------
pub const TEXT: Hsla = controls::TEXT;
pub const TEXT_MUTED: Hsla = controls::SECONDARY;
pub const TEXT_FAINT: Hsla = controls::MUTED;

// --- Semantic status -------------------------------------------------------
pub const SUCCESS: Hsla = SIGNAL_DEVICE;
pub const WARNING: Hsla = hsla(0.120155, 0.964126, 0.562745, 1.0);
pub const DANGER: Hsla = controls::DANGER;
/// Faint red wash layered over a faulted object's own surface (graph card
/// body, mixer strip): DANGER at low alpha so the underlying surface keeps
/// its depth. The red border carries the strong signal; this only tints.
pub const DANGER_WASH: Hsla = Hsla { a: 0.12, ..DANGER };

// --- Level meters ----------------------------------------------------------
/// Translucent overlay that darkens the transient zone between the bright
/// RMS body and the peak reading.
pub const METER_PEAK_ZONE: Hsla = hsla(0., 0., 0., 0.45);

// --- Spacing scale (4pt based) ---------------------------------------------
pub const SPACE_0_5: f32 = 2.;
pub const SPACE_1: f32 = 4.;
pub const SPACE_1_5: f32 = 6.;
pub const SPACE_2: f32 = 8.;
pub const SPACE_3: f32 = 12.;
pub const SPACE_4: f32 = 16.;
pub const SPACE_6: f32 = 24.;

// --- Studio geometry (Figma v0.1.0 Graph measurements) --------------------
/// The shell toolbar's height, aliased from the action-component tokens so
/// the chrome and the navigation controls share one source (design 21 §1.4).
pub use controls::TOOLBAR as TOOLBAR_HEIGHT;
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

// --- Chrome and window geometry (design 21 §1.3) ---------------------------
/// The bottom status bar's height, shared by the chrome host and the bar
/// itself so the two never drift apart.
pub const STATUSBAR_HEIGHT: f32 = 28.;
/// A workspace page's own toolbar strip (the Devices page's tab row).
pub const PAGE_TOOLBAR_HEIGHT: f32 = 48.;
/// A collapsible section's header row, which is also the section's
/// collapsed height.
pub const SECTION_HEADER_HEIGHT: f32 = 28.;
/// Inset of the graph HUD overlays (editing toolbar, overview) from the
/// canvas edges they float against.
pub const HUD_INSET: f32 = 12.;
/// Viewport width below which the shell switches to its compact layout.
pub const COMPACT_BREAKPOINT: f32 = 1200.;
/// Window floor the platform enforces on resize and restore.
pub const WINDOW_MIN_WIDTH: f32 = 940.;
pub const WINDOW_MIN_HEIGHT: f32 = 600.;
/// How much of a restored window must stay on a display for it to count as
/// still living there; anything less falls back to the primary display.
pub const RESTORE_MIN_VISIBLE_WIDTH: f32 = 100.;
pub const RESTORE_MIN_VISIBLE_HEIGHT: f32 = 40.;

// --- Popover placement (design 21 §1.3) -------------------------------------
/// Space an anchored popover keeps from the window edges on every side.
pub const POPOVER_VIEWPORT_MARGIN: f32 = 8.;
/// Space between the anchor's edge and a popover hung beside it.
pub const POPOVER_ANCHOR_GAP: f32 = 4.;

/// Resolves a compact-width value pair: `tight` below [`COMPACT_BREAKPOINT`],
/// `roomy` above it. Generic so the chrome's text limits and the pages'
/// pixel metrics fold through one helper instead of inline `if compact`
/// pairs (design 21 §1.3).
pub fn compact<T>(is_compact: bool, tight: T, roomy: T) -> T {
    if is_compact { tight } else { roomy }
}

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
    /// Aliases of the theme type ladder (design 21 §1.2): the card header's
    /// title is a body-size label and its hint is the tiny readout size, so
    /// the canvas and the chrome track one source.
    pub use super::{TEXT_SM as TITLE_SIZE, TEXT_TINY as HINT_SIZE};
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
/// The action components' base corner radius, aliased so workstation
/// surfaces and kit components share one source (design 21 §1.4).
pub use controls::RADIUS as RADIUS_SM;
pub const RADIUS_MD: f32 = 6.;
pub const RADIUS_LG: f32 = 8.;

// --- Typography ------------------------------------------------------------
pub const FONT_UI: &str = controls::FONT;
pub const FONT_MONO: &str = "Cascadia Mono";
/// The type-size ladder, smallest to largest (design 21 §1.2). Micro and tiny
/// serve dense readouts (fader values, card hints); caption is the badge and
/// tray-dial size between XS and SM. Micro has no call site yet — it anchors
/// the ladder's floor for the first dense readout that needs it.
#[expect(dead_code)]
pub const TEXT_MICRO: f32 = 8.;
pub const TEXT_TINY: f32 = 9.;
pub const TEXT_XS: f32 = 10.;
pub const TEXT_CAPTION: f32 = 11.;
pub const TEXT_SM: f32 = 12.;
pub const TEXT_MD: f32 = 13.;
pub const TEXT_LG: f32 = 15.;

// --- Alpha system (design 21 §1.3) ------------------------------------------
/// Selection wash over content the selection must not obscure (the canvas
/// marquee fill).
pub const ALPHA_SELECTION: f32 = 0.10;
/// Halo strength for a colored wash layered behind a bright stroke (a
/// selected wire's glow, the spectrum fill) so the stroke reads over any
/// background.
pub const ALPHA_HALO: f32 = 0.28;
/// Dimmed strength for a tint that must keep its hue while clearly sitting
/// behind the surface it marks: kind tint bars, displaced-stroke fades.
pub const ALPHA_DIM: f32 = 0.55;

/// Applies [`ALPHA_HALO`] to `color`.
pub fn halo(color: Hsla) -> Hsla {
    color.alpha(ALPHA_HALO)
}

/// Applies [`ALPHA_DIM`] to `color`.
pub fn dim(color: Hsla) -> Hsla {
    color.alpha(ALPHA_DIM)
}

// --- Icons -----------------------------------------------------------------
/// The icon-size ladder (design 21 §1.3): small glyphs beside labels, the
/// medium action-component default, larger rail glyphs, and the rare
/// display-size icon. `controls::ICON` aliases [`ICON_MD`] so the kit action
/// pipeline and workstation glyphs share one source.
pub const ICON_SM: f32 = 14.;
pub const ICON_MD: f32 = 16.;
pub const ICON_LG: f32 = 18.;
/// XL has no call site yet — the title-bar mark is a brand logo, not a UI
/// icon, so it keeps its literal.
#[expect(dead_code)]
pub const ICON_XL: f32 = 24.;

/// Soft fill for an accent (16% alpha over transparent), for chips/badges.
pub fn soft(hsla: Hsla) -> Hsla {
    Hsla { a: 0.16, ..hsla }
}

/// Projects the workstation palette onto the kit's shared component theme,
/// so every gpui-component surface — switches, inputs, menus, tooltips —
/// speaks the same accent instead of the library default. This is the one
/// place to restyle them all: [`component::Theme::update`] reconciles the
/// component tokens from these colors and refreshes every window.
pub fn apply_component_tokens(cx: &mut gpui_kit::App) {
    component::Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.primary = controls::PRIMARY;
        c.primary_hover = controls::PRIMARY_HOVER;
        c.primary_active = controls::PRIMARY_PRESSED;
        c.primary_foreground = controls::ON_ACCENT;
        c.ring = controls::PRIMARY;
        c.accent = soft(controls::PRIMARY);
        c.accent_foreground = controls::TEXT;
        c.danger = controls::DANGER;
        c.danger_hover = controls::DANGER_HOVER;
        c.danger_active = controls::DANGER_PRESSED;
        c.danger_foreground = controls::ON_ACCENT;
        c.selection = soft(controls::PRIMARY);
    });
}
