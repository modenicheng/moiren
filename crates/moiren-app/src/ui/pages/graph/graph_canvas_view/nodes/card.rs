use gpui_kit::{
    App, Bounds, ContentMask, FontWeight, Hsla, PaintQuad, TransformationMatrix, point, px, size,
};

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, CanvasNode, NodeLayout, TextLayout};
use crate::ui::theme;

use super::super::NodeLive;
use super::chips::paint_binding_chip;
use super::curve_thumb::paint_curve_thumb;
use super::dial::paint_knob;
use super::meters::paint_meter_bars;
use super::previews::{paint_matrix_preview, paint_rack_chain};
use super::recording::paint_recording_strip;
use super::shared::{Label, Paint, ShapeStyle, paint_label, rounded};
use super::spectrum::paint_spectrum_body;

/// The faulted card's "!" badge in the reserved header slot: a danger-red
/// disc with a white glyph. The rect is camera-projected, so the badge
/// reads at every zoom like the outline does. Hit-tested as
/// `HitTarget::Badge`; clicking opens the fault detail popover.
fn paint_fault_badge(frame: &CanvasFrame, badge: &Bounds<f32>, paint: Paint<'_>, cx: &mut App) {
    let Paint { shapes, window } = paint;
    const GLYPH: f32 = theme::card::BADGE_SIZE * 0.72;
    let bounds = frame.window_rect(*badge);
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: (f32::from(bounds.size.width) / 2.).into(),
        background: theme::DANGER.into(),
        border_widths: 0f32.into(),
        border_color: theme::DANGER,
        border_style: gpui_kit::BorderStyle::default(),
    });
    let glyph = shapes.fitted(
        window,
        "!",
        ShapeStyle::new(
            GLYPH,
            theme::FONT_UI,
            FontWeight::SEMIBOLD,
            theme::controls::ON_ACCENT,
        ),
        GLYPH,
    );
    let left =
        f32::from(bounds.origin.x) + (f32::from(bounds.size.width) - f32::from(glyph.width())) / 2.;
    let top = f32::from(bounds.origin.y) + (f32::from(bounds.size.height) - GLYPH) / 2.;
    let _ = glyph.paint(
        point(px(left), px(top)),
        px(GLYPH),
        gpui_kit::TextAlign::Left,
        None,
        window,
        cx,
    );
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
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
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
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
}

/// The design 18 node card: header with icon, display name and the
/// expander glyph; full-height tint rail; per-kind body (the detail line
/// until the body widgets land); the meter strip; and port rails with
/// channel hints. Every card wears a gray outline so stacked cards keep
/// their edges, and the outline carries state — danger while a fault names
/// the node, warning while the engine re-prepares it, the primary highlight
/// while selected — so a card's body never changes with its state except
/// for a fault, which additionally tints the surface with the faint danger
/// wash and pins the "!" badge. Fault details surface in
/// [`paint_fault_tooltip`] on hover and in the fault popover via the badge.
pub(in crate::ui) fn paint_node(
    frame: &CanvasFrame,
    node: &CanvasNode,
    layout: &NodeLayout,
    selected: bool,
    live: NodeLive<'_>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let tint = node.tint();
    let card = frame.window_rect(layout.card);
    let radius = theme::card::RADIUS * layout.scale;
    let (outline, outline_width) = if node.faulted {
        (theme::DANGER, theme::card::OUTLINE_W)
    } else if node.preparing {
        (theme::WARNING, theme::card::OUTLINE_REST_W)
    } else if selected {
        (theme::controls::FOCUS, theme::card::OUTLINE_W)
    } else {
        (theme::GRIP, theme::card::OUTLINE_REST_W)
    };
    rounded(card, theme::controls::RAISED, radius, window);
    if node.faulted {
        // The faint red wash over the card's own surface, under the rail
        // and body that the masked paint lays on top of it.
        rounded(card, theme::DANGER_WASH, radius, window);
    }
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
        // Every kind icon is a Tabler glyph (24-unit box, 2-unit padding,
        // stroke 2), so all cards read at the same visual size.
        if let Some(name) = node.icon {
            let _ = window.paint_svg(
                frame.window_rect(layout.icon),
                format!("icons/{name}.svg").into(),
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
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
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
        if node.faulted {
            paint_fault_badge(
                frame,
                &layout.badge,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        }
        // Body: knobs where the kind exposes its main control, else the
        // detail line. States never touch the body — a faulted or
        // preparing card keeps every control usable under its outline.
        if !node.knobs.is_empty() {
            for (knob, knob_layout) in node.knobs.iter().zip(&layout.knobs) {
                paint_knob(
                    frame,
                    knob_layout,
                    knob,
                    layout.scale,
                    Paint {
                        shapes: &mut *shapes,
                        window: &mut *window,
                    },
                    cx,
                );
            }
        } else if let Some(chip) = node.binding.as_ref().zip(layout.chip.as_ref()) {
            let (chip, chip_layout) = chip;
            paint_binding_chip(
                frame,
                chip_layout,
                chip,
                tint,
                layout.scale,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if let (Some(matrix), Some(bounds)) = (&node.matrix, layout.matrix) {
            paint_matrix_preview(frame, bounds, matrix, tint, layout.scale, window);
        } else if let (Some(bounds), false) = (layout.rack, node.rack_stages.is_empty()) {
            paint_rack_chain(
                frame,
                bounds,
                &node.rack_stages,
                tint,
                layout.scale,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if layout.spectrum.is_some() {
            paint_spectrum_body(
                frame,
                layout,
                &node.detail,
                tint,
                live.spectrum,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if let (Some(spec), Some(bounds)) = (&node.curve, layout.curve) {
            paint_curve_thumb(frame, bounds, spec, layout.scale, window);
        } else if let (Some(recording), Some(strip)) = (&node.recording, layout.recording.as_ref())
        {
            paint_recording_strip(
                frame,
                recording,
                &node.detail,
                strip,
                layout.scale,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
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
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        }
        paint_meter_bars(frame, layout, live.meters, window);
    });
    // The outline last, above the rail and the body, so the ring stays
    // whole on every side whatever the state paints underneath.
    window.paint_quad(PaintQuad {
        bounds: card,
        corner_radii: radius.into(),
        background: theme::controls::TRANSPARENT.into(),
        border_widths: outline_width.into(),
        border_color: outline,
        border_style: gpui_kit::BorderStyle::default(),
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
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
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
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
}
