use gpui_kit::{
    App, Axis, BorderStyle, Bounds, ContentMask, Corners, Font, FontWeight, Hsla, PaintQuad, Path,
    PathBuilder, Pixels, ShapedLine, TextAlign, TextRun, TransformationMatrix, Window, font, point,
    px, size, transparent_black,
};

use super::super::graph_canvas::{
    BindingChip, CanvasFrame, CanvasNode, CardKnob, ChipLayout, KnobLayout, MeterLayout,
    NodeLayout, TextLayout,
};
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

fn fill_quad(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: 0f32.into(),
        background: color.into(),
        border_widths: 0f32.into(),
        border_color: transparent_black(),
        border_style: BorderStyle::default(),
    });
}

/// One node card's meter tracks and live bars, in window space. The zone
/// segmentation, dark RMS tail, held marker, and clip latch are the same
/// `meter_spans` math every other meter surface paints (design 17 D1); only
/// the surface differs. Shared by the cached canvas paint and the hot
/// layer's overlay paint so both draw identical geometry; callers own the
/// card content mask. Curve nodes run their meters vertically along the
/// body's right edge, everything else as a horizontal strip under the
/// body (design 18 B2).
pub(in crate::ui) fn paint_meter_bars(
    frame: &CanvasFrame,
    layout: &NodeLayout,
    reading: Option<&super::NodeMeterReading>,
    window: &mut Window,
) {
    let stride = (theme::card::METER_H + theme::card::METER_CHANNEL_GAP) * layout.scale;
    for channel in 0..theme::card::METER_CHANNELS {
        // The clip lamp reserves the last 3 world units of the row — the
        // same loud-end indicator every meter surface uses; the fill never
        // changes background for clipping.
        let (track, axis) = match layout.meter {
            MeterLayout::Horizontal { bounds } => {
                let mut meter = bounds;
                meter.origin.y += channel as f32 * stride;
                meter.size.width -= 3. * layout.scale;
                (meter, Axis::Horizontal)
            }
            MeterLayout::Vertical { bounds } => {
                let mut meter = bounds;
                meter.origin.x -= channel as f32 * stride;
                meter.size.height -= 3. * layout.scale;
                (meter, Axis::Vertical)
            }
        };
        let track = frame.window_rect(track);
        let clip = reading.is_some_and(|reading| reading.clips[channel]);
        rounded(track, theme::WINDOW_BG, layout.scale, window);
        if let Some(level) = reading.and_then(|reading| reading.levels.get(channel)) {
            match axis {
                Axis::Horizontal => {
                    crate::ui::widgets::meters::paint_meter_fill_horizontal(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
                _ => {
                    crate::ui::widgets::meters::paint_meter_fill(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
            }
        }
        crate::ui::widgets::meters::paint_clip_lamp(track, axis, clip, window);
    }
}

/// One knob dial: a neutral track arc with the value portion filled in
/// `TRACK_FILL` (a lit track never reads as audio) and a pointer at the
/// value angle. Screen angles run 135°..405° with y pointing down, so
/// the sweep starts bottom-left, passes the top, and ends bottom-right.
fn paint_knob(
    frame: &CanvasFrame,
    layout: &KnobLayout,
    knob: &CardKnob,
    scale: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let dial = frame.window_rect(layout.dial);
    let (cx_, cy_) = (
        f32::from(dial.origin.x) + f32::from(dial.size.width) / 2.,
        f32::from(dial.origin.y) + f32::from(dial.size.height) / 2.,
    );
    let radius = f32::from(dial.size.width) / 2. - theme::card::DIAL_TRACK_W * scale;
    let stroke = theme::card::DIAL_TRACK_W * scale;
    let arc = |from_deg: f32, to_deg: f32, width: f32| -> anyhow::Result<Path<Pixels>> {
        let mut builder = PathBuilder::stroke(px(width));
        let steps = (((to_deg - from_deg).abs() / 15.).ceil() as usize).max(1);
        for step in 0..=steps {
            let angle = (from_deg + (to_deg - from_deg) * step as f32 / steps as f32).to_radians();
            let p = point(
                px(cx_ + angle.cos() * radius),
                px(cy_ + angle.sin() * radius),
            );
            if step == 0 {
                builder.move_to(p);
            } else {
                builder.line_to(p);
            }
        }
        builder.build()
    };
    if let Ok(path) = arc(135., 405., stroke) {
        window.paint_path(path, theme::WINDOW_BG);
    }
    if knob.position > 0.001
        && let Ok(path) = arc(135., 135. + 270. * knob.position, stroke)
    {
        window.paint_path(path, theme::TRACK_FILL);
    }
    // Pointer from the hub to the value angle.
    let angle = (135. + 270. * knob.position).to_radians();
    let inner = radius * 0.35;
    let outer = radius - stroke;
    if let Ok(path) = {
        let mut builder = PathBuilder::stroke(px(stroke * 1.6));
        builder.move_to(point(
            px(cx_ + angle.cos() * inner),
            px(cy_ + angle.sin() * inner),
        ));
        builder.line_to(point(
            px(cx_ + angle.cos() * outer),
            px(cy_ + angle.sin() * outer),
        ));
        builder.build()
    } {
        window.paint_path(path, theme::card::THUMB);
    }
    paint_label(
        frame,
        &layout.label,
        Label {
            text: &knob.label,
            family: theme::FONT_UI,
            color: theme::TEXT_FAINT,
            right_aligned: false,
            weight: FontWeight::NORMAL,
        },
        window,
        cx,
    );
    paint_label(
        frame,
        &layout.value,
        Label {
            text: &knob.value_text,
            family: theme::FONT_MONO,
            color: theme::TEXT,
            right_aligned: false,
            weight: FontWeight::NORMAL,
        },
        window,
        cx,
    );
}

/// A binding chip: what this source/sink card is wired to. Unbound chips
/// sit quieter — surface fill, faint text — so the missing binding reads
/// as a prompt rather than an error.
fn paint_binding_chip(
    frame: &CanvasFrame,
    layout: &ChipLayout,
    chip: &BindingChip,
    tint: Hsla,
    scale: f32,
    window: &mut Window,
    cx: &mut App,
) {
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
        border_color: transparent_black(),
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
            weight: FontWeight::NORMAL,
        },
        window,
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

/// The channel-mapper preview: one intensity cell per weight, tint-filled
/// over a dark grid so identity reads as a diagonal.
fn paint_matrix_preview(
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
fn paint_rack_chain(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    stages: &[&'static str],
    tint: Hsla,
    scale: f32,
    window: &mut Window,
    cx: &mut App,
) {
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
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
    if stages.len() > shown {
        let label = format!("+{}", stages.len() - shown);
        let line = shape_text(
            window,
            &label,
            theme::TEXT_XS * scale,
            Font {
                weight: FontWeight::NORMAL,
                ..font(theme::FONT_MONO)
            },
            theme::TEXT_FAINT,
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

/// The preparing state's language-neutral marker: three dots in a soft
/// chip, fixed width regardless of locale.
fn paint_preparing_badge(frame: &CanvasFrame, layout: &NodeLayout, window: &mut Window) {
    let badge = frame.window_rect(layout.badge);
    fill_quad(badge, theme::soft(theme::PRIMARY), window);
    let dot = 2. * layout.scale;
    let gap = 3. * layout.scale;
    let total = 3. * dot + 2. * gap;
    let top = badge.origin.y + px((f32::from(badge.size.height) - dot) / 2.);
    let left = badge.origin.x + px((f32::from(badge.size.width) - total) / 2.);
    for index in 0..3 {
        rounded(
            Bounds {
                origin: point(left + px(index as f32 * (dot + gap)), top),
                size: size(px(dot), px(dot)),
            },
            theme::PRIMARY.opacity(0.85),
            dot / 2.,
            window,
        );
    }
}

/// One port anchor: the dot straddles the card edge (filled disc when
/// occupied, ring otherwise) with a channel-count hint inside the card.
struct PortPaint<'a> {
    anchor: (f32, f32),
    ring: &'static str,
    hint: Option<&'a TextLayout>,
    hint_text: String,
    hint_right: bool,
    occupied: bool,
}

fn paint_port(
    frame: &CanvasFrame,
    tint: Hsla,
    scale: f32,
    port: PortPaint<'_>,
    window: &mut Window,
    cx: &mut App,
) {
    let radius = 4. * scale;
    let bounds = frame.window_rect(Bounds {
        origin: point(port.anchor.0 - radius, port.anchor.1 - radius),
        size: size(radius * 2., radius * 2.),
    });
    if port.occupied {
        rounded(bounds, tint, radius, window);
    } else {
        let _ = window.paint_svg(
            bounds,
            format!("figma-nodes/{}.svg", port.ring).into(),
            None,
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
    if let Some(hint) = port.hint {
        paint_label(
            frame,
            hint,
            Label {
                text: &port.hint_text,
                family: theme::FONT_MONO,
                color: theme::TEXT_FAINT,
                right_aligned: port.hint_right,
                weight: FontWeight::NORMAL,
            },
            window,
            cx,
        );
    }
}

/// The design 18 node card: header with icon, display name and status
/// glyphs; full-height tint rail; per-kind body (the detail line until
/// the body widgets land); the meter strip; and port rails with channel
/// hints. States map as: selected = restrained gray border, faulted =
/// danger border + badge, preparing = dots badge + body scrim.
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
    let (border_width, border_color) = if node.faulted {
        (1., theme::DANGER)
    } else if selected {
        (1., theme::GRIP)
    } else {
        (0., transparent_black())
    };
    window.paint_quad(PaintQuad {
        bounds: card,
        corner_radii: (6. * layout.scale).into(),
        background: theme::SURFACE_3.into(),
        border_widths: border_width.into(),
        border_color,
        border_style: BorderStyle::default(),
    });
    window.with_content_mask(Some(ContentMask { bounds: card }), |window| {
        // Tint rail with the card's rounded left corners.
        let rail = frame.window_rect(layout.rail);
        window.paint_quad(PaintQuad {
            bounds: rail,
            corner_radii: Corners {
                top_left: px(6. * layout.scale),
                bottom_left: px(6. * layout.scale),
                top_right: px(0.),
                bottom_right: px(0.),
            },
            background: tint.into(),
            border_widths: 0f32.into(),
            border_color: transparent_black(),
            border_style: BorderStyle::default(),
        });
        // Header.
        if let Some(name) = node.icon {
            let path = if let Some(name) = name.strip_prefix("figma-") {
                format!("figma-nodes/{name}.svg")
            } else {
                format!("icons/{name}.svg")
            };
            let _ = window.paint_svg(
                frame.window_rect(layout.icon),
                path.into(),
                None,
                TransformationMatrix::unit(),
                tint,
                cx,
            );
        }
        paint_label(
            frame,
            &layout.title,
            Label {
                text: &node.name,
                family: theme::FONT_UI,
                color: theme::TEXT,
                right_aligned: false,
                weight: FontWeight::SEMIBOLD,
            },
            window,
            cx,
        );
        if node.faulted {
            let _ = window.paint_svg(
                frame.window_rect(layout.badge),
                "icons/alert-triangle.svg".into(),
                None,
                TransformationMatrix::unit(),
                theme::DANGER,
                cx,
            );
        } else if node.preparing {
            paint_preparing_badge(frame, layout, window);
        }
        let _ = window.paint_svg(
            frame.window_rect(layout.expand),
            "figma-nodes/more.svg".into(),
            None,
            TransformationMatrix::unit(),
            theme::TEXT_FAINT,
            cx,
        );
        // Body: knobs where the kind exposes its main control, else the
        // detail line; the preparing scrim keeps the header readable.
        if node.preparing {
            let mut scrim = frame.window_rect(layout.body);
            scrim.origin.y -= px(2. * layout.scale);
            scrim.size.height =
                card.origin.y + card.size.height - scrim.origin.y - px(2. * layout.scale);
            fill_quad(scrim, theme::WINDOW_BG.opacity(0.45), window);
        }
        if !node.knobs.is_empty() {
            for (knob, knob_layout) in node.knobs.iter().zip(&layout.knobs) {
                paint_knob(frame, knob_layout, knob, layout.scale, window, cx);
            }
        } else if let Some(chip) = node.binding.as_ref().zip(layout.chip.as_ref()) {
            let (chip, chip_layout) = chip;
            paint_binding_chip(frame, chip_layout, chip, tint, layout.scale, window, cx);
        } else if let (Some(matrix), Some(bounds)) = (&node.matrix, layout.matrix) {
            paint_matrix_preview(frame, bounds, matrix, tint, layout.scale, window);
        } else if let (Some(bounds), false) = (layout.rack, node.rack_stages.is_empty()) {
            paint_rack_chain(
                frame,
                bounds,
                &node.rack_stages,
                tint,
                layout.scale,
                window,
                cx,
            );
        } else {
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
        }
        paint_meter_bars(frame, layout, reading, window);
    });
    // Port dots straddle the card edge, outside the content mask; the
    // channel hints sit just inside. Occupied inputs render as filled
    // discs — the single-connection rule makes occupancy meaningful only
    // on the input side.
    let scale = layout.scale;
    for ((anchor, port), hint) in layout
        .inputs
        .iter()
        .copied()
        .zip(&node.input_ports)
        .zip(&layout.input_hints)
    {
        paint_port(
            frame,
            tint,
            scale,
            PortPaint {
                anchor,
                ring: "input-port",
                hint: Some(hint),
                hint_text: format!("{}ch", port.channels),
                hint_right: false,
                occupied: frame
                    .world
                    .wires
                    .iter()
                    .any(|wire| wire.dst_port == port.id),
            },
            window,
            cx,
        );
    }
    if let Some(anchor) = layout.output {
        paint_port(
            frame,
            tint,
            scale,
            PortPaint {
                anchor,
                ring: "output-port",
                hint: layout.output_hint.as_ref(),
                hint_text: format!(
                    "{}ch",
                    node.output_port
                        .map(|port| port.channels)
                        .unwrap_or(node.channels)
                ),
                hint_right: true,
                occupied: false,
            },
            window,
            cx,
        );
    }
}
