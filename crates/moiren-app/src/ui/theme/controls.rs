//! Shared tokens for the approved v0.1.2 action components.
//!
//! Source: Moiren UI v0.1.2, Navigation & Actions, checked 2026-10-14.
//! The experimental graph theme is migrated as its components are replaced.

use gpui_kit::{Hsla, hsla};

pub const CANVAS: Hsla = hsla(0.611111, 0.25, 0.047059, 1.0); // #090b0f
pub const PANEL: Hsla = hsla(0.6, 0.227273, 0.086275, 1.0); // #11151b
pub const RAISED: Hsla = hsla(0.602564, 0.19403, 0.131373, 1.0); // #1b2028
pub const BORDER: Hsla = hsla(0.604167, 0.16, 0.196078, 1.0); // #2a303a
pub const FOCUS: Hsla = hsla(0.722741, 0.781022, 0.731373, 1.0); // #a985f0
pub const TEXT: Hsla = hsla(0.583333, 0.285714, 0.972549, 1.0); // #f6f8fa
pub const SECONDARY: Hsla = hsla(0.6, 0.153846, 0.745098, 1.0); // #b4bcc8
pub const MUTED: Hsla = hsla(0.602564, 0.127451, 0.6, 1.0); // #8c96a6
pub const DISABLED: Hsla = hsla(0.6, 0.147059, 0.266667, 1.0); // #3a424e
pub const ON_ACCENT: Hsla = hsla(0.0, 0.0, 1.00000000, 1.0); // #ffffff
pub const PRIMARY: Hsla = hsla(0.725333, 0.523013, 0.531373, 1.0); // #7549c6
pub const PRIMARY_HOVER: Hsla = hsla(0.718667, 0.675676, 0.637255, 1.0); // #8b64e1
pub const PRIMARY_PRESSED: Hsla = hsla(0.735632, 0.537037, 0.423529, 1.0); // #6232a6
pub const PRIMARY_MUTED: Hsla = hsla(0.737443, 0.623932, 0.229412, 1.0); // #35165f
pub const DANGER: Hsla = hsla(0.0, 0.90604, 0.707843, 1.0); // #f87171
pub const DANGER_HOVER: Hsla = hsla(0.0, 0.911111, 0.735294, 1.0); // #f97e7e
pub const DANGER_PRESSED: Hsla = hsla(0.0, 0.545894, 0.594118, 1.0); // #d05f5f
pub const DANGER_DISABLED: Hsla = hsla(0.981061, 0.366667, 0.235294, 1.0); // #52262b
pub const CONTROL_DISABLED: Hsla = hsla(0.619048, 0.2, 0.068627, 1.0); // #0e1015
pub const TAB_DISABLED: Hsla = hsla(0.6, 0.102041, 0.480392, 1.0); // #6e7887
pub const CLOSE_PRESSED: Hsla = hsla(0.986877, 0.554585, 0.44902, 1.0); // #b2333d
pub const INDICATOR: Hsla = hsla(0.104167, 0.307692, 0.94902, 1.0); // #f6f3ee
pub const TRANSPARENT: Hsla = hsla(0., 0., 0., 0.);
pub const FONT: &str = "Inter";
pub const RADIUS: f32 = 4.;
pub const ICON: f32 = 16.;
pub const HIT_TARGET: f32 = 32.;
pub const TOOLBAR: f32 = 44.;
pub const TAB_WIDTH: f32 = 112.;
pub const WINDOW_WIDTH: f32 = 46.;
pub const INDICATOR_SIZE: f32 = 4.;
pub const INDICATOR_INSET: f32 = 4.;

/// Figma's two density modes; icon and minimum control sizes stay legible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// A semantic size, resolved together with density rather than at callsites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub height: f32,
    pub padding: f32,
    pub gap: f32,
    pub font: f32,
    pub line_height: f32,
}

impl ControlSize {
    pub fn metrics(self, density: Density) -> Metrics {
        let compact = density == Density::Compact;
        match self {
            Self::Small => Metrics {
                height: 28.,
                padding: if compact { 6. } else { 8. },
                gap: 4.,
                font: 11.,
                line_height: 14.,
            },
            Self::Medium => Metrics {
                height: if compact { 30. } else { 32. },
                padding: if compact { 8. } else { 12. },
                gap: if compact { 6. } else { 8. },
                font: 12.,
                line_height: 16.,
            },
            Self::Large => Metrics {
                height: if compact { 32. } else { 36. },
                padding: if compact { 12. } else { 16. },
                gap: if compact { 6. } else { 8. },
                font: 13.,
                line_height: 16.,
            },
        }
    }
}
