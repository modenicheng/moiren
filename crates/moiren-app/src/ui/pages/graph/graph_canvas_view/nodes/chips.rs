use gpui_kit::{App, BorderStyle, PaintQuad, TransformationMatrix};

use crate::ui::pages::graph::graph_canvas::{BindingChip, CanvasFrame, ChipLayout};
use crate::ui::theme;

use super::shared::{Label, Paint, paint_label};

/// A binding chip: what this source/sink card is wired to. Unbound chips
/// sit quieter — surface fill, faint text — so the missing binding reads
/// as a prompt rather than an error.
pub(super) fn paint_binding_chip(
    frame: &CanvasFrame,
    layout: &ChipLayout,
    chip: &BindingChip,
    tint: gpui_kit::Hsla,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let bounds = frame.window_rect(layout.bounds);
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: (theme::RADIUS_SM * scale).into(),
        background: if chip.unbound {
            theme::SURFACE_4.into()
        } else {
            theme::SURFACE.into()
        },
        border_widths: 0f32.into(),
        border_color: theme::controls::TRANSPARENT,
        border_style: BorderStyle::default(),
    });
    let _ = window.paint_svg(
        frame.window_rect(layout.icon),
        format!("icons/{}.svg", chip.icon).into(),
        None,
        TransformationMatrix::unit(),
        if chip.unbound {
            theme::TEXT_FAINT
        } else {
            tint
        },
        cx,
    );
    paint_label(
        frame,
        &layout.label,
        Label {
            text: &chip.label,
            family: theme::FONT_UI,
            color: if chip.unbound {
                theme::TEXT_FAINT
            } else {
                theme::TEXT
            },
            right_aligned: false,
            weight: gpui_kit::FontWeight::NORMAL,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
    let _ = window.paint_svg(
        frame.window_rect(layout.chevron),
        "icons/chevron-down.svg".into(),
        None,
        TransformationMatrix::unit(),
        theme::TEXT_FAINT,
        cx,
    );
}
