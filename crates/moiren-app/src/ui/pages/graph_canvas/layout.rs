use gpui_kit::{Bounds, point, size};

use super::{Camera, CanvasNode};
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct TextLayout {
    pub bounds: Bounds<f32>,
    pub font_size: f32,
    pub line_height: f32,
}

impl TextLayout {
    fn new(left: f32, top: f32, width: f32, font_size: f32) -> Self {
        Self {
            bounds: Bounds {
                origin: point(left, top),
                size: size(width.max(0.), font_size + 2.),
            },
            font_size,
            line_height: font_size + 2.,
        }
    }

    fn project(&self, camera: Camera) -> Self {
        Self {
            bounds: camera.project_rect(self.bounds),
            font_size: self.font_size * camera.zoom,
            line_height: self.line_height * camera.zoom,
        }
    }
}

/// Where a card's level meter strip sits and which way it runs. The hot
/// layer repaints these from the shared frame, so the geometry must round
/// trip through [`Camera::project_rect`] like every other card region.
#[derive(Debug, Clone, Copy)]
pub enum MeterLayout {
    Horizontal { bounds: Bounds<f32> },
    Vertical { bounds: Bounds<f32> },
}

impl MeterLayout {
    pub fn bounds(&self) -> Bounds<f32> {
        match self {
            Self::Horizontal { bounds } | Self::Vertical { bounds } => *bounds,
        }
    }

    fn project(&self, camera: Camera) -> Self {
        match *self {
            Self::Horizontal { bounds } => Self::Horizontal {
                bounds: camera.project_rect(bounds),
            },
            Self::Vertical { bounds } => Self::Vertical {
                bounds: camera.project_rect(bounds),
            },
        }
    }
}

/// One knob on the body: the dial circle plus its label and value text
/// beside it.
#[derive(Debug, Clone)]
pub struct KnobLayout {
    pub dial: Bounds<f32>,
    pub label: TextLayout,
    pub value: TextLayout,
}

/// Design 18 card anatomy: header (icon, name, status glyphs), tint rail,
/// per-kind body, meter strip, port rails with channel hints. World-space
/// units; [`NodeLayout::project`] scales everything with the camera.
#[derive(Debug, Clone)]
pub struct NodeLayout {
    pub card: Bounds<f32>,
    pub rail: Bounds<f32>,
    pub icon: Bounds<f32>,
    pub title: TextLayout,
    pub badge: Bounds<f32>,
    pub expand: Bounds<f32>,
    pub body: Bounds<f32>,
    pub detail: TextLayout,
    pub meter: MeterLayout,
    pub knobs: Vec<KnobLayout>,
    pub inputs: Vec<(f32, f32)>,
    pub input_hints: Vec<TextLayout>,
    pub output: Option<(f32, f32)>,
    pub output_hint: Option<TextLayout>,
    pub scale: f32,
}

impl NodeLayout {
    pub fn new(node: &CanvasNode) -> Self {
        let width = node.width();
        let body_top = node.y + theme::card::BODY_TOP;
        let body_bottom = body_top + node.body_height;
        let meter_total = theme::card::METER_CHANNELS as f32 * theme::card::METER_H
            + theme::card::METER_CHANNEL_GAP;
        let ports_at =
            (body_bottom + meter_total + theme::card::PORT_BOTTOM).max(theme::card::PORT_Y);
        let height = node.height();
        let port_y = node.y + ports_at;
        let padding = theme::card::PADDING;
        let inputs = (0..node.input_count)
            .map(|index| (node.x, port_y + index as f32 * theme::card::PORT_SPACING))
            .collect::<Vec<_>>();
        let input_hints = (0..node.input_count)
            .map(|index| {
                TextLayout::new(
                    node.x + theme::card::RAIL_W + padding,
                    port_y + index as f32 * theme::card::PORT_SPACING
                        - theme::card::HINT_SIZE / 2.
                        - 1.,
                    48.,
                    theme::card::HINT_SIZE,
                )
            })
            .collect();
        let output_hint = node.has_output.then(|| {
            TextLayout::new(
                node.x + width - theme::card::RAIL_W - padding - 48.,
                port_y - theme::card::HINT_SIZE / 2. - 1.,
                48.,
                theme::card::HINT_SIZE,
            )
        });
        let meter = if node.meter_vertical {
            MeterLayout::Vertical {
                bounds: Bounds {
                    origin: point(
                        node.x + width - theme::card::RAIL_W - padding - theme::card::METER_H,
                        body_top,
                    ),
                    size: size(theme::card::METER_H, body_bottom - body_top),
                },
            }
        } else {
            MeterLayout::Horizontal {
                bounds: Bounds {
                    origin: point(
                        node.x + theme::card::RAIL_W + padding,
                        body_bottom + theme::card::METER_CHANNEL_GAP,
                    ),
                    size: size(
                        width - theme::card::RAIL_W - 2. * padding,
                        theme::card::METER_H,
                    ),
                },
            }
        };
        let knobs = node
            .knobs
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let dial_x = node.x
                    + theme::card::RAIL_W
                    + padding
                    + index as f32 * (theme::card::DIAL_SIZE + 16.);
                let dial_y = body_top + (node.body_height - theme::card::DIAL_SIZE) / 2.;
                let text_left = dial_x + theme::card::DIAL_SIZE + 8.;
                let text_width = node.x + width - padding - text_left;
                KnobLayout {
                    dial: Bounds {
                        origin: point(dial_x, dial_y),
                        size: size(theme::card::DIAL_SIZE, theme::card::DIAL_SIZE),
                    },
                    label: TextLayout::new(
                        text_left,
                        dial_y + 3.,
                        text_width,
                        theme::card::HINT_SIZE,
                    ),
                    value: TextLayout::new(
                        text_left,
                        dial_y + theme::card::DIAL_SIZE - 14.,
                        text_width,
                        11.,
                    ),
                }
            })
            .collect();
        Self {
            card: Bounds {
                origin: point(node.x, node.y),
                size: size(width, height),
            },
            rail: Bounds {
                origin: point(node.x, node.y),
                size: size(theme::card::RAIL_W, height),
            },
            icon: Bounds {
                origin: point(
                    node.x + theme::card::RAIL_W + 8.,
                    node.y + (theme::card::HEADER_H - theme::card::ICON_SIZE) / 2.,
                ),
                size: size(theme::card::ICON_SIZE, theme::card::ICON_SIZE),
            },
            title: TextLayout::new(
                node.x + theme::card::RAIL_W + 8. + theme::card::ICON_SIZE + 6.,
                node.y + (theme::card::HEADER_H - theme::card::TITLE_SIZE - 2.) / 2.,
                theme::card::TITLE_WIDTH,
                theme::card::TITLE_SIZE,
            ),
            badge: Bounds {
                origin: point(
                    node.x + width - padding - theme::card::BADGE_SIZE - 18.,
                    node.y + (theme::card::HEADER_H - theme::card::BADGE_SIZE) / 2.,
                ),
                size: size(theme::card::BADGE_SIZE, theme::card::BADGE_SIZE),
            },
            expand: Bounds {
                origin: point(
                    node.x + width - padding - theme::card::BADGE_SIZE,
                    node.y + (theme::card::HEADER_H - theme::card::BADGE_SIZE) / 2.,
                ),
                size: size(theme::card::BADGE_SIZE, theme::card::BADGE_SIZE),
            },
            body: Bounds {
                origin: point(node.x + theme::card::RAIL_W + padding, body_top),
                size: size(width - theme::card::RAIL_W - 2. * padding, node.body_height),
            },
            detail: TextLayout::new(
                node.x + theme::card::RAIL_W + padding,
                body_top,
                width - theme::card::RAIL_W - 2. * padding,
                theme::TEXT_XS,
            ),
            meter,
            knobs,
            inputs,
            input_hints,
            output: node.has_output.then_some((node.x + width, port_y)),
            output_hint,
            scale: 1.,
        }
    }

    pub fn project(&self, camera: Camera) -> Self {
        Self {
            card: camera.project_rect(self.card),
            rail: camera.project_rect(self.rail),
            icon: camera.project_rect(self.icon),
            title: self.title.project(camera),
            badge: camera.project_rect(self.badge),
            expand: camera.project_rect(self.expand),
            body: camera.project_rect(self.body),
            detail: self.detail.project(camera),
            meter: self.meter.project(camera),
            knobs: self
                .knobs
                .iter()
                .map(|knob| KnobLayout {
                    dial: camera.project_rect(knob.dial),
                    label: knob.label.project(camera),
                    value: knob.value.project(camera),
                })
                .collect(),
            inputs: self
                .inputs
                .iter()
                .map(|anchor| camera.world_to_canvas(*anchor))
                .collect(),
            input_hints: self
                .input_hints
                .iter()
                .map(|hint| hint.project(camera))
                .collect(),
            output: self.output.map(|anchor| camera.world_to_canvas(anchor)),
            output_hint: self.output_hint.as_ref().map(|hint| hint.project(camera)),
            scale: camera.zoom,
        }
    }
}
