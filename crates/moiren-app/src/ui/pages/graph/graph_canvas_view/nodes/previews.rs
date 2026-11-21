use gpui_kit::{App, Bounds, FontWeight, Hsla, TextAlign, Window, point, px, size};

use crate::ui::pages::graph::graph_canvas::CanvasFrame;
use crate::ui::theme;

use super::shared::{Paint, ShapeStyle, rounded};

/// The channel-mapper preview: one intensity cell per weight, tint-filled
/// over a dark grid so identity reads as a diagonal.
pub(super) fn paint_matrix_preview(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    matrix: &[Vec<f64>],
    tint: Hsla,
    scale: f32,
    window: &mut Window,
) {
    let cell = (theme::card::MATRIX_CELL + theme::card::MATRIX_GAP) * scale;
    let grid = frame.window_rect(bounds);
    for (row, weights) in matrix.iter().enumerate().take(8) {
        for (col, weight) in weights.iter().enumerate().take(8) {
            let cell_bounds = Bounds {
                origin: point(
                    grid.origin.x + px(col as f32 * cell),
                    grid.origin.y + px(row as f32 * cell),
                ),
                size: size(
                    px(theme::card::MATRIX_CELL * scale),
                    px(theme::card::MATRIX_CELL * scale),
                ),
            };
            rounded(cell_bounds, theme::WINDOW_BG, 1.5 * scale, window);
            let intensity = (weight.abs().min(1.) as f32) * 0.85 + 0.15;
            rounded(cell_bounds, tint.opacity(intensity), 1.5 * scale, window);
        }
    }
}

/// The rack chain: one glyph per stage in signal order, with a count when
/// the chain outgrows the body.
pub(super) fn paint_rack_chain(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    stages: &[&'static str],
    tint: Hsla,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let grid = frame.window_rect(bounds);
    let icon = 14. * scale;
    let stride = 20. * scale;
    let top = grid.origin.y + px((f32::from(grid.size.height) - icon) / 2.);
    let shown = stages.len().min(12);
    for (index, stage) in stages.iter().take(shown).enumerate() {
        let _ = window.paint_svg(
            Bounds {
                origin: point(grid.origin.x + px(index as f32 * stride), top),
                size: size(px(icon), px(icon)),
            },
            format!("icons/{stage}.svg").into(),
            None,
            gpui_kit::TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
    if stages.len() > shown {
        let label = format!("+{}", stages.len() - shown);
        let line = shapes.fitted(
            window,
            &label,
            ShapeStyle::new(
                theme::TEXT_XS * scale,
                theme::FONT_MONO,
                FontWeight::NORMAL,
                theme::TEXT_FAINT,
            ),
            40. * scale,
        );
        let _ = line.paint(
            point(grid.origin.x + px(shown as f32 * stride), top),
            px(theme::TEXT_XS * scale + 2.),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}
