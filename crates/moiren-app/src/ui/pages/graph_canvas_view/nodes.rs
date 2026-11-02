use gpui_kit::{
    App, Axis, BorderStyle, Bounds, ContentMask, Font, FontWeight, Hsla, PaintQuad, Path,
    PathBuilder, Pixels, ShapedLine, TextAlign, TextRun, TransformationMatrix, Window, font, point,
    px, size, transparent_black,
};
use rust_i18n::t;

use super::super::graph_canvas::{
    BindingChip, CanvasFrame, CanvasNode, CardKnob, ChipLayout, CurveSpec, KnobLayout, MeterLayout,
    NodeLayout, TextLayout, curves,
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

/// A static curve thumbnail: baseline, the sampled curve, and a
/// center/threshold tick. The same math the detail window paints, at
/// card scale.
fn paint_curve_thumb(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    spec: &CurveSpec,
    scale: f32,
    window: &mut Window,
) {
    let plot = frame.window_rect(bounds);
    let width = f32::from(plot.size.width);
    let height = f32::from(plot.size.height);
    let origin_x = f32::from(plot.origin.x);
    let origin_y = f32::from(plot.origin.y);
    fn line(
        origin_x: f32,
        origin_y: f32,
        from: (f32, f32),
        to: (f32, f32),
        stroke: f32,
        color: Hsla,
        window: &mut Window,
    ) {
        let mut builder = gpui_kit::PathBuilder::stroke(px(stroke));
        builder.move_to(point(px(origin_x + from.0), px(origin_y + from.1)));
        builder.line_to(point(px(origin_x + to.0), px(origin_y + to.1)));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
    fn stroke_points(
        origin_x: f32,
        origin_y: f32,
        scale: f32,
        samples: &[(f64, f64)],
        x_of: &dyn Fn(f64) -> f32,
        y_of: &dyn Fn(f64) -> f32,
        window: &mut Window,
    ) {
        let mut builder = gpui_kit::PathBuilder::stroke(px(1.5 * scale));
        let mut first = true;
        for (x, y) in samples {
            let p = point(px(origin_x + x_of(*x)), px(origin_y + y_of(*y)));
            if first {
                builder.move_to(p);
                first = false;
            } else {
                builder.line_to(p);
            }
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, theme::TRACK_FILL);
        }
    }
    match spec {
        CurveSpec::Eq(settings) => {
            let x_of = |freq: f64| ((freq / 20.).ln() / 1000f64.ln()) as f32 * width;
            let y_of = |db: f64| height / 2. - (db as f32 / 24.) * height / 2.;
            line(
                origin_x,
                origin_y,
                (0., height / 2.),
                (width, height / 2.),
                scale,
                theme::HAIRLINE,
                window,
            );
            let center = x_of(settings.frequency_hz);
            line(
                origin_x,
                origin_y,
                (center, 0.),
                (center, height),
                scale,
                theme::BORDER_STRONG,
                window,
            );
            let samples = curves::sample_curve_log_x(
                |freq| curves::eq_magnitude_db(settings, 48_000., freq),
                20.,
                20_000.,
                64,
            );
            stroke_points(origin_x, origin_y, scale, &samples, &x_of, &y_of, window);
        }
        CurveSpec::Dynamics(curve) => {
            let span = 60.;
            let x_of = |db: f64| ((db + span) / (2. * span)) as f32 * width;
            let y_of = |db: f64| height - ((db + span) / (2. * span)) as f32 * height;
            line(
                origin_x,
                origin_y,
                (0., height),
                (width, 0.),
                scale,
                theme::HAIRLINE,
                window,
            );
            let threshold = x_of(curve.threshold_db);
            line(
                origin_x,
                origin_y,
                (threshold, 0.),
                (threshold, height),
                scale,
                theme::BORDER_STRONG,
                window,
            );
            let samples =
                curves::sample_curve_linear_x(|db| curve.transfer_db(db), -span, span, 64);
            stroke_points(origin_x, origin_y, scale, &samples, &x_of, &y_of, window);
        }
    }
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

/// The fault tooltip under a hovered faulted card: every fault's cause
/// with its hint beneath, translated from the catalog keys the service
/// published exactly as the error strip does. Painted in window pixels —
/// a tooltip reads the same at every zoom — below the card, flipping
/// above it when the canvas bottom is too close, and above every card
/// because the hot layer paints it.
pub(in crate::ui) fn paint_fault_tooltip(
    frame: &CanvasFrame,
    index: usize,
    window: &mut Window,
    cx: &mut App,
) {
    const PAD: f32 = 8.;
    const GAP: f32 = 6.;
    const ICON: f32 = 14.;
    const MAX_LINE: f32 = 360.;
    let node = &frame.world.nodes[index];
    if node.faults.is_empty() {
        return;
    }
    let line_height = theme::TEXT_SM + 6.;
    let mut lines: Vec<(ShapedLine, bool)> = Vec::new();
    for fault in &node.faults {
        lines.push((
            shape_text(
                window,
                &t!(fault.cause.as_str()),
                theme::TEXT_SM,
                Font {
                    weight: FontWeight::SEMIBOLD,
                    ..font(theme::FONT_UI)
                },
                theme::DANGER,
                MAX_LINE,
            ),
            true,
        ));
        if let Some(hint) = &fault.hint {
            lines.push((
                shape_text(
                    window,
                    &t!(hint.as_str()),
                    theme::TEXT_SM,
                    font(theme::FONT_UI),
                    theme::TEXT,
                    MAX_LINE,
                ),
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
    let card = frame.window_rect(frame.nodes[index].card);
    let canvas = frame.bounds;
    let canvas_left = f32::from(canvas.origin.x);
    let canvas_right = canvas_left + f32::from(canvas.size.width);
    let canvas_bottom = f32::from(canvas.origin.y) + f32::from(canvas.size.height);
    let left = f32::from(card.origin.x)
        .min(canvas_right - width)
        .max(canvas_left);
    let below = f32::from(card.origin.y + card.size.height) + GAP;
    let top = if below + height > canvas_bottom {
        f32::from(card.origin.y) - GAP - height
    } else {
        below
    };
    window.with_content_mask(Some(ContentMask { bounds: canvas }), |window| {
        window.paint_quad(PaintQuad {
            bounds: Bounds {
                origin: point(px(left), px(top)),
                size: size(px(width), px(height)),
            },
            corner_radii: theme::RADIUS_SM.into(),
            background: theme::SURFACE.into(),
            border_widths: 1f32.into(),
            border_color: theme::BORDER_STRONG,
            border_style: BorderStyle::default(),
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

/// The design 18 node card: header with icon, display name and the
/// expander glyph; full-height tint rail; per-kind body (the detail line
/// until the body widgets land); the meter strip; and port rails with
/// channel hints. Every card wears a gray outline so stacked cards keep
/// their edges, and the outline alone carries state — danger while a
/// fault names the node, warning while the engine re-prepares it, the
/// primary highlight while selected — so a card's body never changes
/// with its state. Faults themselves surface in [`paint_fault_tooltip`].
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
    let radius = theme::card::RADIUS * layout.scale;
    let outline = if node.faulted {
        theme::DANGER
    } else if node.preparing {
        theme::WARNING
    } else if selected {
        theme::PRIMARY
    } else {
        theme::GRIP
    };
    rounded(card, theme::SURFACE_3, radius, window);
    window.with_content_mask(Some(ContentMask { bounds: card }), |window| {
        // Tint rail: the card's own left edge just inside the outline. A
        // rail-wide quad cannot carry the card's corner radius (gpui clamps
        // radii to the quad's size and the corners poke out square), so
        // paint the inset card in tint and mask it down to the rail strip.
        let inset = px(theme::card::OUTLINE_W);
        let inner = Bounds {
            origin: card.origin + point(inset, inset),
            size: size(card.size.width - inset * 2., card.size.height - inset * 2.),
        };
        let rail = frame.window_rect(layout.rail);
        let strip = Bounds {
            origin: inner.origin,
            size: size(rail.size.width, inner.size.height),
        };
        window.with_content_mask(Some(ContentMask { bounds: strip }), |window| {
            rounded(
                inner,
                tint,
                (radius - theme::card::OUTLINE_W).max(0.),
                window,
            );
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
        let _ = window.paint_svg(
            frame.window_rect(layout.expand),
            "figma-nodes/more.svg".into(),
            None,
            TransformationMatrix::unit(),
            theme::TEXT_FAINT,
            cx,
        );
        // Body: knobs where the kind exposes its main control, else the
        // detail line. States never touch the body — a faulted or
        // preparing card keeps every control usable under its outline.
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
        } else if let (Some(spec), Some(bounds)) = (&node.curve, layout.curve) {
            paint_curve_thumb(frame, bounds, spec, layout.scale, window);
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
    // The outline last, above the rail and the body, so the ring stays
    // whole on every side whatever the state paints underneath.
    window.paint_quad(PaintQuad {
        bounds: card,
        corner_radii: radius.into(),
        background: transparent_black().into(),
        border_widths: theme::card::OUTLINE_W.into(),
        border_color: outline,
        border_style: BorderStyle::default(),
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
