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

#[derive(Debug, Clone)]
pub struct NodeLayout {
    pub card: Bounds<f32>,
    pub icon: Bounds<f32>,
    pub title: TextLayout,
    pub detail: TextLayout,
    pub more: Bounds<f32>,
    pub meter: Bounds<f32>,
    pub fader: Bounds<f32>,
    pub balance: Bounds<f32>,
    pub gain_value: TextLayout,
    pub balance_value: TextLayout,
    pub inputs: Vec<(f32, f32)>,
    pub output: Option<(f32, f32)>,
    pub scale: f32,
}

impl NodeLayout {
    pub fn new(node: &CanvasNode) -> Self {
        let padding = theme::card::PADDING;
        let width = node.width();
        let icon_size = theme::card::ICON_SIZE;
        let text_left = node.x + padding + icon_size + 4.;
        let text_width = width - padding * 2. - icon_size - 40.;
        let title_top = node.y + theme::card::DECO_HEIGHT + padding + 2.;
        let port_y = node.y + 80.;
        let inputs = (0..node.input_count)
            .map(|index| (node.x, port_y + index as f32 * 18.))
            .collect::<Vec<_>>();
        Self {
            card: Bounds {
                origin: point(node.x, node.y),
                size: size(width, node.height()),
            },
            icon: Bounds {
                origin: point(node.x + padding, node.y + padding + 6.),
                size: size(icon_size, icon_size),
            },
            title: TextLayout::new(text_left, title_top, text_width, 12.),
            detail: TextLayout::new(text_left, title_top + 15., text_width, theme::TEXT_XS),
            more: Bounds {
                origin: point(node.x + width - padding - 16., node.y + 24.),
                size: size(16., 16.),
            },
            meter: Bounds {
                origin: point(node.x + padding, node.y + theme::card::METER_Y),
                size: size(theme::card::TRACK_WIDTH, theme::card::METER_HEIGHT),
            },
            fader: Bounds {
                origin: point(node.x + padding, node.y + theme::card::FADER_Y),
                size: size(theme::card::TRACK_WIDTH, theme::card::TRACK_HEIGHT),
            },
            balance: Bounds {
                origin: point(node.x + padding, node.y + theme::card::BALANCE_Y),
                size: size(theme::card::TRACK_WIDTH, theme::card::TRACK_HEIGHT),
            },
            gain_value: TextLayout::new(
                node.x + width - padding - theme::card::VALUE_WIDTH,
                node.y + 60.,
                theme::card::VALUE_WIDTH,
                10.,
            ),
            balance_value: TextLayout::new(
                node.x + width - padding - theme::card::VALUE_WIDTH,
                node.y + 84.,
                theme::card::VALUE_WIDTH,
                10.,
            ),
            inputs,
            output: node.has_output.then_some((node.x + width, port_y)),
            scale: 1.,
        }
    }

    pub fn project(&self, camera: Camera) -> Self {
        Self {
            card: camera.project_rect(self.card),
            icon: camera.project_rect(self.icon),
            title: self.title.project(camera),
            detail: self.detail.project(camera),
            more: camera.project_rect(self.more),
            meter: camera.project_rect(self.meter),
            fader: camera.project_rect(self.fader),
            balance: camera.project_rect(self.balance),
            gain_value: self.gain_value.project(camera),
            balance_value: self.balance_value.project(camera),
            inputs: self
                .inputs
                .iter()
                .map(|anchor| camera.world_to_canvas(*anchor))
                .collect(),
            output: self.output.map(|anchor| camera.world_to_canvas(anchor)),
            scale: camera.zoom,
        }
    }
}
