//! Signal-graph canvas: grid, pan/zoom camera, node cards, bezier wires.
//!
//! Ported from graph-canvas.slint. gpui's `Path` primitive is fill-only (no
//! stroke width), so wires are built as filled polygon ribbons: sample the
//! cubic bezier, then offset each sample along the curve normal by ±w/2.

use std::sync::Arc;

use gpui_kit::*;

use super::theme;
use crate::service::AppSnapshot;

/// Camera state: world origin at canvas top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
    /// User moved the camera manually; stop auto-fitting.
    pub adjusted: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan_x: 0.,
            pan_y: 0.,
            zoom: 1.,
            adjusted: false,
        }
    }
}

impl Camera {
    pub fn zoom_by(&mut self, factor: f32, anchor: (f32, f32)) {
        self.zoom_at((self.zoom * factor).clamp(0.25, 2.0), anchor);
    }
    pub fn zoom_at(&mut self, next_zoom: f32, anchor: (f32, f32)) {
        let (ax, ay) = anchor;
        self.pan_x = ax - (ax - self.pan_x) * next_zoom / self.zoom;
        self.pan_y = ay - (ay - self.pan_y) * next_zoom / self.zoom;
        self.zoom = next_zoom;
        self.adjusted = true;
    }
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan_x += dx;
        self.pan_y += dy;
        self.adjusted = true;
    }
    /// Toolbar zoom buttons have no cursor; keep the canvas center fixed.
    pub fn zoom_around_center(&mut self, canvas: Option<(f32, f32)>, factor: f32) {
        let (ax, ay) = canvas.map_or((0., 0.), |(w, h)| (w / 2., h / 2.));
        self.zoom_by(factor, (ax, ay));
    }
    /// Fit the graph into the canvas with gutters. Returns fit geometry.
    pub fn fit(&mut self, canvas: Size<Pixels>, graph: Bounds<f32>) {
        let (cw, ch) = (f32::from(canvas.width), f32::from(canvas.height));
        let gutter = theme::SPACE_5;
        let chrome_top = 96.;
        let chrome_bottom = 48.;
        let avail_w = (cw - 2. * gutter).max(1.);
        let avail_h = (ch - chrome_top - chrome_bottom).max(1.);
        let gw = graph.size.width.max(1.);
        let gh = graph.size.height.max(1.);
        self.zoom = (avail_w / gw).min(avail_h / gh).clamp(0.25, 1.0);
        self.pan_x = gutter + (avail_w - gw * self.zoom) / 2. - graph.origin.x * self.zoom;
        self.pan_y = chrome_top + (avail_h - gh * self.zoom) / 2. - graph.origin.y * self.zoom;
    }
}

/// World-space node descriptor for the candidate session graph.
#[derive(Debug, Clone)]
pub struct CanvasNode {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub value: String,
    pub icon: Option<&'static str>,
    pub x: f32,
    pub y: f32,
    pub wide: bool,
    pub has_input: bool,
    pub has_output: bool,
    pub input_label: String,
    pub output_label: String,
    pub tint: Option<Hsla>,
    /// 0..1 parameter level (GAIN/PAN bars).
    pub level: Option<f32>,
    pub is_parameter: bool,
}

impl CanvasNode {
    pub fn width(&self) -> f32 {
        if self.wide {
            theme::NODE_WIDE_WIDTH
        } else {
            theme::NODE_WIDTH
        }
    }
    pub fn height(&self) -> f32 {
        theme::NODE_HEIGHT
    }
    pub fn port_y(&self) -> f32 {
        theme::card::PORT_Y
    }
    pub fn tint(&self) -> Hsla {
        self.tint.unwrap_or(theme::PRIMARY)
    }
}

/// Fresh card with sensible defaults; callers refine via the builder.
fn node(id: &str, name: &str, detail: impl Into<String>) -> CanvasNode {
    CanvasNode {
        id: id.into(),
        name: name.into(),
        detail: detail.into(),
        value: String::new(),
        icon: None,
        x: 0.,
        y: 0.,
        wide: false,
        has_input: false,
        has_output: false,
        input_label: String::new(),
        output_label: String::new(),
        tint: Some(theme::PRIMARY),
        level: None,
        is_parameter: false,
    }
}

/// World-space wire between two port anchors.
#[derive(Debug, Clone)]
pub struct CanvasWire {
    pub id: String,
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub tint: Option<Hsla>,
}

/// The whole world payload the canvas renders.
pub struct CanvasWorld {
    pub nodes: Vec<CanvasNode>,
    pub wires: Vec<CanvasWire>,
    pub graph_bounds: Bounds<f32>,
}

impl CanvasWorld {
    /// Project the current fixed stereo session (input -> gain -> pan -> out).
    pub fn candidate(snapshot: &Arc<AppSnapshot>) -> Self {
        let gain = snapshot.control.pending_gain_pan.map(|(g, _)| g);
        let pan = snapshot.control.pending_gain_pan.map(|(_, p)| p);
        let spacing = 206.;
        let y = 160.;
        let nodes = vec![
            node("in", "Input", "Microphone")
                .icon("mic")
                .at(24., y)
                .ports(false, true)
                .with_tint(theme::SIGNAL_DEVICE),
            node(
                "gain",
                "Gain",
                gain.map(|g| format!("{g:.2}")).unwrap_or_default(),
            )
            .icon("sliders")
            .at(24. + spacing, y)
            .ports(true, true)
            .parameter(gain.unwrap_or(0.) as f32 / 16.),
            node(
                "pan",
                "Pan",
                pan.map(|p| format!("{p:+.2}")).unwrap_or_default(),
            )
            .icon("sliders")
            .at(24. + 2. * spacing, y)
            .ports(true, true)
            .parameter((pan.unwrap_or(0.) as f32 + 1.) / 2.),
            node("out", "Output", "Speakers")
                .icon("headphones")
                .at(24. + 3. * spacing, y)
                .ports(true, false)
                .with_tint(theme::SIGNAL_RECORDER)
                .wide(),
        ];
        let wires: Vec<CanvasWire> = nodes
            .windows(2)
            .map(|pair| {
                let (a, b) = (&pair[0], &pair[1]);
                CanvasWire {
                    id: format!("{}-{}", a.id, b.id),
                    from: (a.x + a.width(), a.y + a.port_y()),
                    to: (b.x, b.y + b.port_y()),
                    tint: Some(a.tint()),
                }
            })
            .collect();
        let graph_bounds = Bounds {
            origin: point(24., y - 24.),
            size: size(
                3. * spacing + theme::NODE_WIDE_WIDTH + 48.,
                theme::NODE_HEIGHT + 48.,
            ),
        };
        Self {
            nodes,
            wires,
            graph_bounds,
        }
    }
}

impl CanvasNode {
    fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }
    fn ports(mut self, input: bool, output: bool) -> Self {
        self.has_input = input;
        self.has_output = output;
        if input {
            self.input_label = "in".into();
        }
        if output {
            self.output_label = "out".into();
        }
        self
    }
    fn with_tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
    fn parameter(mut self, level: f32) -> Self {
        self.is_parameter = true;
        self.level = Some(level.clamp(0., 1.));
        self
    }
    fn wide(mut self) -> Self {
        self.wide = true;
        self
    }
}

/// Fill-only cubic bezier ribbon (stroke emulation via normal offsets).
pub fn wire_ribbon(from: (f32, f32), to: (f32, f32), width: f32) -> Path<Pixels> {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let dx = ((x2 - x1).abs() * 0.4).max(24.);
    // Cubic bezier with horizontal handles.
    let p = |t: f32| -> (f32, f32) {
        let mt = 1. - t;
        let a = mt * mt * mt;
        let b = 3. * mt * mt * t;
        let c = 3. * mt * t * t;
        let d = t * t * t;
        (
            a * x1 + b * (x1 + dx) + c * (x2 - dx) + d * x2,
            a * y1 + b * y1 + c * y2 + d * y2,
        )
    };
    let samples = 16;
    let half = width / 2.;
    let mut upper: Vec<(f32, f32)> = Vec::with_capacity(samples + 1);
    let mut lower: Vec<(f32, f32)> = Vec::with_capacity(samples + 1);
    for i in 0..=samples {
        let t = i as f32 / samples as f32;
        let (cx, cy) = p(t);
        let (nx, ny) = p((t + 1. / samples as f32).min(1.));
        // tangent
        let tx = nx - cx;
        let ty = ny - cy;
        let len = (tx * tx + ty * ty).sqrt().max(1e-6);
        // normal
        let (ux, uy) = (-ty / len * half, tx / len * half);
        upper.push((cx + ux, cy + uy));
        lower.push((cx - ux, cy - uy));
    }
    let mut path = Path::new(point(px(upper[0].0), px(upper[0].1)));
    for (x, y) in upper.iter().skip(1) {
        path.line_to(point(px(*x), px(*y)));
    }
    for (x, y) in lower.iter().rev() {
        path.line_to(point(px(*x), px(*y)));
    }
    path
}
