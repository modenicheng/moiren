use gpui_kit::{
    App, Bounds, ContentMask, FontWeight, PaintQuad, Pixels, ShapedLine, TextAlign,
    TransformationMatrix, point, px, size,
};
use rust_i18n::t;

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, CardFault};
use crate::ui::theme;

use super::shared::{Paint, ShapeStyle};

/// The hovered card's fault banner, hung below the card the tooltip is
/// for. The general painter below does the drawing; this only resolves
/// the canvas's card and clip.
pub(in crate::ui) fn paint_fault_tooltip(
    frame: &CanvasFrame,
    index: usize,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let node = &frame.world.nodes[index];
    if node.faults.is_empty() {
        return;
    }
    let card = frame.window_rect(frame.nodes[index].card);
    paint_fault_banner(
        &node.faults,
        card,
        frame.bounds,
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
}

/// The fault banner: a window-pixel panel hung below `anchor`, flipping
/// above it near `clip`'s bottom edge and clamped inside `clip`, listing
/// every fault's translated cause and hint. Shared by the graph page (the
/// hovered card) and the mixer page (the hovered strip), so both surfaces'
/// hover errors read identically.
pub(in crate::ui) fn paint_fault_banner(
    faults: &[CardFault],
    anchor: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    const PAD: f32 = 8.;
    const GAP: f32 = 6.;
    const ICON: f32 = 14.;
    const MAX_LINE: f32 = 360.;
    if faults.is_empty() {
        return;
    }
    let line_height = theme::TEXT_SM + 6.;
    let mut lines: Vec<(ShapedLine, bool)> = Vec::new();
    for fault in faults {
        lines.push((
            shapes
                .fitted(
                    window,
                    &t!(fault.cause.as_str()),
                    ShapeStyle::new(
                        theme::TEXT_SM,
                        theme::FONT_UI,
                        FontWeight::SEMIBOLD,
                        theme::DANGER,
                    ),
                    MAX_LINE,
                )
                .clone(),
            true,
        ));
        if let Some(hint) = &fault.hint {
            lines.push((
                shapes
                    .fitted(
                        window,
                        &t!(hint.as_str()),
                        ShapeStyle::new(
                            theme::TEXT_SM,
                            theme::FONT_UI,
                            FontWeight::NORMAL,
                            theme::TEXT,
                        ),
                        MAX_LINE,
                    )
                    .clone(),
                false,
            ));
        }
    }
    let text_width = lines
        .iter()
        .map(|(line, _)| f32::from(line.width()))
        .fold(0., f32::max);
    let width = PAD + ICON + 6. + text_width + PAD;
    let height = 2. * PAD + lines.len() as f32 * line_height;
    let clip_left = f32::from(clip.origin.x);
    let clip_right = clip_left + f32::from(clip.size.width);
    let clip_bottom = f32::from(clip.origin.y) + f32::from(clip.size.height);
    let left = f32::from(anchor.origin.x)
        .min(clip_right - width)
        .max(clip_left);
    let below = f32::from(anchor.origin.y) + f32::from(anchor.size.height) + GAP;
    let top = if below + height > clip_bottom {
        f32::from(anchor.origin.y) - GAP - height
    } else {
        below
    };
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        window.paint_quad(PaintQuad {
            bounds: Bounds {
                origin: point(px(left), px(top)),
                size: size(px(width), px(height)),
            },
            corner_radii: theme::RADIUS_SM.into(),
            background: theme::SURFACE.into(),
            border_widths: 1f32.into(),
            border_color: theme::BORDER_STRONG,
            border_style: gpui_kit::BorderStyle::default(),
        });
        let text_left = left + PAD + ICON + 6.;
        for (row, (line, cause)) in lines.iter().enumerate() {
            let row_top = top + PAD + row as f32 * line_height;
            if *cause {
                let _ = window.paint_svg(
                    Bounds {
                        origin: point(px(left + PAD), px(row_top + (line_height - ICON) / 2.)),
                        size: size(px(ICON), px(ICON)),
                    },
                    "icons/alert-triangle.svg".into(),
                    None,
                    TransformationMatrix::unit(),
                    theme::DANGER,
                    cx,
                );
            }
            let _ = line.paint(
                point(px(text_left), px(row_top)),
                px(line_height),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    });
}
