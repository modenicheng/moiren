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

fn paint_track(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    value: Option<f32>,
    centered: bool,
    window: &mut Window,
) {
    let bounds = frame.window_rect(bounds);
    rounded(bounds, theme::WINDOW_BG, 2. * frame.camera.zoom, window);
    let Some(value) = value else { return };
    let value = value.clamp(0., 1.);
    let start = if centered { value.min(0.5) } else { 0. };
    let width = if centered { (value - 0.5).abs() } else { value };
    rounded(
        Bounds {
            origin: bounds.origin + point(bounds.size.width * start, px(0.)),
            size: size(bounds.size.width * width, bounds.size.height),
        },
        theme::card::TRACK_FILL,
        2. * frame.camera.zoom,
        window,
    );
    let thumb_width = px(4. * frame.camera.zoom);
    rounded(
        Bounds {
            origin: bounds.origin
                + point(
                    (bounds.size.width * value - thumb_width / 2.)
                        .clamp(px(0.), bounds.size.width - thumb_width),
                    px(-2. * frame.camera.zoom),
                ),
            size: size(thumb_width, px(8. * frame.camera.zoom)),
        },
        theme::card::THUMB,
        2. * frame.camera.zoom,
        window,
    );
}

/// One node card's meter tracks and live bars, in window space. The zone
/// segmentation, dark RMS tail, held marker, and clip latch are the same
/// `meter_spans` math every other meter surface paints (design 17 D1); only
/// the surface differs. Shared by the cached canvas paint and the hot
/// layer's overlay paint so both draw identical geometry; callers own the
/// card content mask.
pub(in crate::ui) fn paint_meter_bars(
    frame: &CanvasFrame,
    layout: &NodeLayout,
    reading: Option<&super::NodeMeterReading>,
    window: &mut Window,
) {
    for channel in 0..2 {
        let mut meter = layout.meter;
        meter.origin.y += channel as f32 * 3. * layout.scale;
        // The clip lamp reserves the last 3 world units of the row — the
        // same loud-end indicator every meter surface uses; the fill never
        // changes background for clipping.
        meter.size.width -= 3. * layout.scale;
        let track = frame.window_rect(meter);
        let clip = reading.is_some_and(|reading| reading.clips[channel]);
        rounded(track, theme::WINDOW_BG, layout.scale, window);
        if let Some(level) = reading.and_then(|reading| reading.levels.get(channel)) {
            crate::ui::widgets::meters::paint_meter_fill_horizontal(
                track,
                *level,
                theme::WINDOW_BG,
                window,
            );
        }
        crate::ui::widgets::meters::paint_clip_lamp(
            track,
            gpui_kit::Axis::Horizontal,
            clip,
            window,
        );
    }
}

/// The reading is taken at paint time, not baked into the projected world,
/// so meter motion repaints without re-projecting the graph.
pub(super) fn paint_node(
    frame: &CanvasFrame,
    node: &CanvasNode,
    layout: &NodeLayout,
    selected: bool,
    reading: Option<&super::NodeMeterReading>,
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
        paint_meter_bars(frame, layout, reading, window);
        let gain_db = if node.gain <= 0. {
            -60.
        } else {
            (20. * node.gain.log10()) as f32
        };
        paint_track(
            frame,
            layout.fader,
            Some((gain_db + 60.) / 72.),
            false,
            window,
        );
        paint_track(
            frame,
            layout.balance,
            node.balance.map(|value| (value as f32 + 1.) / 2.),
            true,
            window,
        );
        let gain = if node.gain <= 0. {
            "-inf".into()
        } else {
            format!("{gain_db:.1}")
        };
        let balance = match node.balance {
            Some(value) if value < -0.005 => format!("L {:.0}", -value * 100.),
            Some(value) if value > 0.005 => format!("R {:.0}", value * 100.),
            Some(_) => "C".into(),
            None => "\u{2014}".into(),
        };
        for (bounds, text) in [
            (&layout.gain_value, gain.as_str()),
            (&layout.balance_value, balance.as_str()),
        ] {
            paint_label(
                frame,
                bounds,
                Label {
                    text,
                    family: theme::FONT_MONO,
                    color: theme::TEXT_MUTED,
                    right_aligned: true,
                    weight: FontWeight::NORMAL,
                },
                window,
                cx,
            );
        }
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
