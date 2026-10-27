use gpui_kit::{
    App, BorderStyle, Bounds, ContentMask, Corners, Font, FontWeight, Hsla, PaintQuad, Pixels,
    ShapedLine, TextAlign, TextRun, TransformationMatrix, Window, font, point, px, size,
    transparent_black,
};

use super::super::graph_canvas::{CanvasFrame, CanvasNode, NodeLayout, TextLayout};
use crate::ui::theme;

fn shape_text(
    window: &Window,
    text: &str,
    font_size: f32,
    font: Font,
    color: Hsla,
    available: f32,
) -> ShapedLine {
    let shape = |text: &str| {
        window.text_system().shape_line(
            text.to_owned().into(),
            px(font_size),
            &run(text, font.clone(), color),
            None,
        )
    };
    let original = shape(text);
    if f32::from(original.width()) <= available {
        return original;
    }
    let boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    let mut lower = 0;
    let mut upper = boundaries.len();
    let mut best = shape("");
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        let candidate = shape(&format!("{}…", &text[..boundaries[middle]]));
        if f32::from(candidate.width()) <= available {
            best = candidate;
            lower = middle + 1;
        } else {
            upper = middle;
        }
    }
    best
}

fn run(text: &str, font: Font, color: Hsla) -> [TextRun; 1] {
    [TextRun {
        len: text.len(),
        font,
        color,
        ..TextRun::default()
    }]
}

struct Label<'a> {
    text: &'a str,
    family: &'a str,
    color: Hsla,
    right_aligned: bool,
    weight: FontWeight,
}

fn paint_label(
    frame: &CanvasFrame,
    layout: &TextLayout,
    label: Label<'_>,
    window: &mut Window,
    cx: &mut App,
) {
    let rect = frame.window_rect(layout.bounds);
    let line = shape_text(
        window,
        label.text,
        layout.font_size,
        Font {
            weight: label.weight,
            ..font(label.family)
        },
        label.color,
        layout.bounds.size.width,
    );
    let left = if label.right_aligned {
        rect.origin.x + rect.size.width - line.width()
    } else {
        rect.origin.x
    };
    let _ = line.paint(
        point(left, rect.origin.y),
        px(layout.line_height),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn rounded(bounds: Bounds<Pixels>, color: Hsla, radius: f32, window: &mut Window) {
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: radius.into(),
        background: color.into(),
        border_widths: 0f32.into(),
        border_color: transparent_black(),
        border_style: BorderStyle::default(),
    });
}

pub(super) fn paint_node(
    frame: &CanvasFrame,
    node: &CanvasNode,
    layout: &NodeLayout,
    selected: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let tint = node.tint();
    let card = frame.window_rect(layout.card);
    window.paint_quad(PaintQuad {
        bounds: card,
        corner_radii: (6. * layout.scale).into(),
        background: theme::SURFACE_3.into(),
        border_widths: if selected { 1. } else { 0. }.into(),
        border_color: theme::GRIP,
        border_style: BorderStyle::default(),
    });
    window.with_content_mask(Some(ContentMask { bounds: card }), |window| {
        window.paint_quad(PaintQuad {
            bounds: Bounds {
                origin: card.origin,
                size: size(card.size.width, px(theme::card::DECO_HEIGHT * layout.scale)),
            },
            corner_radii: Corners {
                top_left: px(6. * layout.scale),
                top_right: px(6. * layout.scale),
                bottom_left: px(0.),
                bottom_right: px(0.),
            },
            background: tint.into(),
            border_widths: 0f32.into(),
            border_color: transparent_black(),
            border_style: BorderStyle::default(),
        });
        let mut icon = frame.window_rect(layout.icon);
        if let Some(name) = node.icon {
            let path = if let Some(name) = name.strip_prefix("figma-") {
                if name == "input" {
                    // The exported mic is the inner group of a 28px design slot.
                    icon.origin += point(px(4.8333 * layout.scale), px(1.3333 * layout.scale));
                    icon.size = size(px(18.3333 * layout.scale), px(24.1667 * layout.scale));
                }
                format!("figma-nodes/{name}.svg")
            } else {
                format!("icons/{name}.svg")
            };
            let _ = window.paint_svg(
                icon,
                path.into(),
                None,
                TransformationMatrix::unit(),
                if name == "figma-fader" {
                    theme::controls::PRIMARY
                } else {
                    tint
                },
                cx,
            );
        }
        paint_label(
            frame,
            &layout.title,
            Label {
                text: &node.name,
                family: theme::FONT_UI,
                color: theme::controls::ON_ACCENT,
                right_aligned: false,
                weight: FontWeight::SEMIBOLD,
            },
            window,
            cx,
        );
        paint_label(
            frame,
            &layout.detail,
            Label {
                text: &node.detail,
                family: theme::FONT_UI,
                color: theme::TEXT_MUTED,
                right_aligned: false,
                weight: FontWeight::NORMAL,
            },
            window,
            cx,
        );
        let _ = window.paint_svg(
            frame.window_rect(layout.more),
            "figma-nodes/more.svg".into(),
            None,
            TransformationMatrix::unit(),
            theme::TEXT_MUTED,
            cx,
        );
        let mut divider = card;
        divider.origin.y += px(53. * layout.scale);
        divider.size.height = px(1.);
        rounded(divider, theme::BORDER, 0., window);
    });
    let radius = 4. * layout.scale;
    for (anchor, asset) in layout
        .inputs
        .iter()
        .copied()
        .map(|anchor| (anchor, "input-port"))
        .chain(layout.output.map(|anchor| (anchor, "output-port")))
    {
        let bounds = frame.window_rect(Bounds {
            origin: point(anchor.0 - radius, anchor.1 - radius),
            size: size(radius * 2., radius * 2.),
        });
        let _ = window.paint_svg(
            bounds,
            format!("figma-nodes/{asset}.svg").into(),
            None,
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
}
