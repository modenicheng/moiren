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

    /// Canvas-space shift. Projection is affine, so translating a projected
    /// layout equals projecting the moved node — this is how the hot layer
    /// paints a dragged card without re-projecting anything (audit A-1).
    fn translated(mut self, dx: f32, dy: f32) -> Self {
        self.bounds.origin += point(dx, dy);
        self
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

/// A binding chip: the pill's rect with its icon slot, label, and the
/// dropdown affordance at the right edge.
#[derive(Debug, Clone)]
pub struct ChipLayout {
    pub bounds: Bounds<f32>,
    pub icon: Bounds<f32>,
    pub label: TextLayout,
    pub chevron: Bounds<f32>,
}

/// A recorder card's session strip: the status line (dot, phase word, and
/// the hot-layer duration slot on its right), the output file name, and the
/// three transport slots. Slot rects always exist; which carry affordances
/// follows `RecordAction::available` in both the paint and the hit test.
#[derive(Debug, Clone)]
pub struct RecordingLayout {
    pub dot: Bounds<f32>,
    pub status: TextLayout,
    pub duration: TextLayout,
    pub file_name: TextLayout,
    pub record: Bounds<f32>,
    pub pause: Bounds<f32>,
    pub stop: Bounds<f32>,
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
    pub chip: Option<ChipLayout>,
    pub recording: Option<RecordingLayout>,
    pub matrix: Option<Bounds<f32>>,
    pub rack: Option<Bounds<f32>>,
    pub curve: Option<Bounds<f32>>,
    /// The spectrum analyzer's plot, laid out like a curve thumbnail: the
    /// body less the vertical meter's strip on the right.
    pub spectrum: Option<Bounds<f32>>,
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
        let height = node.height();
        let port_y = node.y + node.port_offset();
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
        let chip = node.binding.as_ref().map(|_| {
            let bounds = Bounds {
                origin: point(
                    node.x + theme::card::RAIL_W + padding,
                    body_top + (node.body_height - theme::card::CHIP_H) / 2.,
                ),
                size: size(
                    width - theme::card::RAIL_W - 2. * padding,
                    theme::card::CHIP_H,
                ),
            };
            ChipLayout {
                icon: Bounds {
                    origin: point(
                        bounds.origin.x + 5.,
                        bounds.origin.y + (theme::card::CHIP_H - theme::card::CHIP_ICON) / 2.,
                    ),
                    size: size(theme::card::CHIP_ICON, theme::card::CHIP_ICON),
                },
                label: TextLayout::new(
                    bounds.origin.x + theme::card::CHIP_ICON + 9.,
                    bounds.origin.y + (theme::card::CHIP_H - 12.) / 2.,
                    bounds.size.width - theme::card::CHIP_ICON - 9. - 18.,
                    11.,
                ),
                chevron: Bounds {
                    origin: point(
                        bounds.origin.x + bounds.size.width - 15.,
                        bounds.origin.y + (theme::card::CHIP_H - 12.) / 2.,
                    ),
                    size: size(12., 12.),
                },
                bounds,
            }
        });
        let matrix = node.matrix.as_ref().map(|_| Bounds {
            origin: point(node.x + theme::card::RAIL_W + padding, body_top),
            size: size(width - theme::card::RAIL_W - 2. * padding, node.body_height),
        });
        let recording = node.recording.as_ref().map(|_| {
            let left = node.x + theme::card::RAIL_W + padding;
            let inner = width - theme::card::RAIL_W - 2. * padding;
            let status_w = (inner * 0.42).max(48.);
            let button_y = body_top + 32.;
            let button = |offset: f32| Bounds {
                origin: point(left + offset, button_y),
                size: size(theme::card::REC_BTN, theme::card::REC_BTN),
            };
            RecordingLayout {
                dot: Bounds {
                    origin: point(left, body_top + 3.),
                    size: size(theme::card::REC_DOT, theme::card::REC_DOT),
                },
                status: TextLayout::new(
                    left + theme::card::REC_DOT + 6.,
                    body_top,
                    status_w,
                    theme::card::HINT_SIZE,
                ),
                duration: TextLayout::new(
                    left + theme::card::REC_DOT + 6. + status_w,
                    body_top,
                    inner - status_w - theme::card::REC_DOT - 6.,
                    theme::card::HINT_SIZE,
                ),
                file_name: TextLayout::new(left, body_top + 16., inner, 11.),
                record: button(0.),
                pause: button(theme::card::REC_BTN + 6.),
                stop: button(2. * (theme::card::REC_BTN + 6.)),
            }
        });
        let rack = (!node.rack_stages.is_empty()).then(|| Bounds {
            origin: point(node.x + theme::card::RAIL_W + padding, body_top),
            size: size(width - theme::card::RAIL_W - 2. * padding, node.body_height),
        });
        let meter_stride = theme::card::METER_H + theme::card::METER_CHANNEL_GAP;
        let plot = Bounds {
            origin: point(node.x + theme::card::RAIL_W + padding, body_top),
            size: size(
                width - theme::card::RAIL_W - 2. * padding - 2. * meter_stride - 4.,
                node.body_height,
            ),
        };
        let curve = node.curve.is_some().then_some(plot);
        let spectrum = node.spectrum.then_some(plot);
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
            chip,
            recording,
            matrix,
            rack,
            curve,
            spectrum,
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
            chip: self.chip.as_ref().map(|chip| ChipLayout {
                bounds: camera.project_rect(chip.bounds),
                icon: camera.project_rect(chip.icon),
                label: chip.label.project(camera),
                chevron: camera.project_rect(chip.chevron),
            }),
            recording: self.recording.as_ref().map(|strip| RecordingLayout {
                dot: camera.project_rect(strip.dot),
                status: strip.status.project(camera),
                duration: strip.duration.project(camera),
                file_name: strip.file_name.project(camera),
                record: camera.project_rect(strip.record),
                pause: camera.project_rect(strip.pause),
                stop: camera.project_rect(strip.stop),
            }),
            matrix: self.matrix.map(|bounds| camera.project_rect(bounds)),
            rack: self.rack.map(|bounds| camera.project_rect(bounds)),
            curve: self.curve.map(|bounds| camera.project_rect(bounds)),
            spectrum: self.spectrum.map(|bounds| camera.project_rect(bounds)),
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

    /// The projected layout of the same card shifted by a canvas-space
    /// offset — the hot layer's view of a dragged card. Projection is
    /// affine, so this equals [`Self::project`] of the node at its moved
    /// world position; every field is listed explicitly so a layout field
    /// added later cannot silently escape the drag offset.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let shift = |bounds: &mut Bounds<f32>| bounds.origin += point(dx, dy);
        let text = |text: &TextLayout| text.clone().translated(dx, dy);
        let anchor = |anchor: &(f32, f32)| (anchor.0 + dx, anchor.1 + dy);
        Self {
            card: {
                let mut card = self.card;
                shift(&mut card);
                card
            },
            rail: {
                let mut rail = self.rail;
                shift(&mut rail);
                rail
            },
            icon: {
                let mut icon = self.icon;
                shift(&mut icon);
                icon
            },
            title: text(&self.title),
            badge: {
                let mut badge = self.badge;
                shift(&mut badge);
                badge
            },
            expand: {
                let mut expand = self.expand;
                shift(&mut expand);
                expand
            },
            body: {
                let mut body = self.body;
                shift(&mut body);
                body
            },
            detail: text(&self.detail),
            meter: match self.meter {
                MeterLayout::Horizontal { bounds } => MeterLayout::Horizontal {
                    bounds: {
                        let mut bounds = bounds;
                        shift(&mut bounds);
                        bounds
                    },
                },
                MeterLayout::Vertical { bounds } => MeterLayout::Vertical {
                    bounds: {
                        let mut bounds = bounds;
                        shift(&mut bounds);
                        bounds
                    },
                },
            },
            knobs: self
                .knobs
                .iter()
                .map(|knob| KnobLayout {
                    dial: {
                        let mut dial = knob.dial;
                        shift(&mut dial);
                        dial
                    },
                    label: text(&knob.label),
                    value: text(&knob.value),
                })
                .collect(),
            chip: self.chip.as_ref().map(|chip| ChipLayout {
                bounds: {
                    let mut bounds = chip.bounds;
                    shift(&mut bounds);
                    bounds
                },
                icon: {
                    let mut icon = chip.icon;
                    shift(&mut icon);
                    icon
                },
                label: text(&chip.label),
                chevron: {
                    let mut chevron = chip.chevron;
                    shift(&mut chevron);
                    chevron
                },
            }),
            recording: self.recording.as_ref().map(|strip| RecordingLayout {
                dot: {
                    let mut dot = strip.dot;
                    shift(&mut dot);
                    dot
                },
                status: text(&strip.status),
                duration: text(&strip.duration),
                file_name: text(&strip.file_name),
                record: {
                    let mut record = strip.record;
                    shift(&mut record);
                    record
                },
                pause: {
                    let mut pause = strip.pause;
                    shift(&mut pause);
                    pause
                },
                stop: {
                    let mut stop = strip.stop;
                    shift(&mut stop);
                    stop
                },
            }),
            matrix: self.matrix.map(|mut bounds| {
                shift(&mut bounds);
                bounds
            }),
            rack: self.rack.map(|mut bounds| {
                shift(&mut bounds);
                bounds
            }),
            curve: self.curve.map(|mut bounds| {
                shift(&mut bounds);
                bounds
            }),
            spectrum: self.spectrum.map(|mut bounds| {
                shift(&mut bounds);
                bounds
            }),
            inputs: self.inputs.iter().map(anchor).collect(),
            input_hints: self.input_hints.iter().map(text).collect(),
            output: self.output.map(|a| anchor(&a)),
            output_hint: self.output_hint.as_ref().map(text),
            scale: self.scale,
        }
    }
}
