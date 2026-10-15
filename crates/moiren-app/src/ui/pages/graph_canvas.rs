//! Signal-graph canvas: grid, pan/zoom camera, node cards, bezier wires.
//!
//! Ported from graph-canvas.slint. gpui's `Path` primitive is fill-only (no
//! stroke width), so wires are built as filled polygon ribbons: sample the
//! cubic bezier, then offset each sample along the curve normal by ±w/2.

use gpui_kit::*;

use crate::service::AppSnapshot;
use crate::ui::theme;

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
    pub input_count: usize,
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
        theme::NODE_HEIGHT.max(self.port_y() + self.input_count as f32 * 18. + 8.)
    }
    pub fn input_y(&self, index: usize) -> f32 {
        self.port_y() + index as f32 * 18.
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
        input_count: 0,
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
    /// The service snapshot is the only topology authority. Positions are presentation only.
    pub fn positioned(
        snapshot: &AppSnapshot,
        positions: &std::collections::HashMap<String, (f32, f32)>,
    ) -> Self {
        use moiren_core::graph::NodeKind;
        let graph = &snapshot.graph;
        let order = graph.graph.topological_order().unwrap_or_default();
        let mut depth = std::collections::HashMap::new();
        let mut rows = std::collections::HashMap::new();
        let mut nodes = Vec::new();
        for id in order {
            let logical = graph.graph.get_node(id).expect("validated snapshot node");
            let state = graph.node(id).expect("service node state");
            let level = graph
                .graph
                .edges()
                .iter()
                .filter(|edge| edge.dst() == id)
                .filter_map(|edge| depth.get(&edge.src()))
                .copied()
                .max()
                .map_or(0, |n: usize| n + 1);
            depth.insert(id, level);
            let row = rows.entry(level).or_insert(0usize);
            let key = format!("{id:?}");
            let position = positions
                .get(&key)
                .copied()
                .unwrap_or((32. + level as f32 * 250., 80. + *row as f32 * 160.));
            *row += 1;
            let detail = match logical.kind() {
                NodeKind::Source => state
                    .source
                    .as_ref()
                    .map(|source| crate::ui::state::source_name(snapshot, source))
                    .unwrap_or_else(|| "Choose a source".into()),
                NodeKind::Sink => state
                    .output_endpoint_id
                    .as_ref()
                    .map(|id| crate::ui::state::output_name(snapshot, id))
                    .unwrap_or_else(|| "Choose an output".into()),
                NodeKind::Gain => {
                    if state.gain == 0. {
                        "−∞ dB".into()
                    } else {
                        format!("{:+.1} dB", 20. * state.gain.log10())
                    }
                }
                NodeKind::Pan => format!("Balance {:+.2}", state.pan),
                NodeKind::Bus => format!(
                    "{} inputs · {} channels",
                    logical.inputs().len(),
                    logical.channels()
                ),
                NodeKind::Compressor => "Linked peak compressor".into(),
            };
            let (icon, tint) = match logical.kind() {
                NodeKind::Source => ("wave", theme::SIGNAL_DEVICE),
                NodeKind::Sink => ("headphones", theme::SIGNAL_RECORDER),
                NodeKind::Bus => ("git-merge", theme::TEXT_MUTED),
                _ => ("sliders", theme::PRIMARY),
            };
            let mut card = node(&key, &state.label, detail)
                .at(position.0, position.1)
                .icon(icon)
                .ports(!logical.inputs().is_empty(), !logical.outputs().is_empty())
                .with_tint(tint);
            card.input_count = logical.inputs().len();
            if logical.kind() == NodeKind::Gain {
                card = card.parameter(state.gain as f32);
            }
            if logical.kind() == NodeKind::Pan {
                card = card.parameter((state.pan as f32 + 1.) / 2.);
            }
            nodes.push(card);
        }
        let wires = graph
            .graph
            .edges()
            .iter()
            .filter_map(|edge| {
                let a = nodes.iter().find(|n| n.id == format!("{:?}", edge.src()))?;
                let b = nodes.iter().find(|n| n.id == format!("{:?}", edge.dst()))?;
                let input = graph
                    .graph
                    .get_node(edge.dst())
                    .ok()?
                    .inputs()
                    .iter()
                    .position(|p| p.id() == edge.dst_port())?;
                Some(CanvasWire {
                    id: format!("{:?}", edge.id()),
                    from: (a.x + a.width(), a.y + a.port_y()),
                    to: (b.x, b.y + b.input_y(input)),
                    tint: Some(if edge.params().mute {
                        theme::TEXT_FAINT
                    } else {
                        a.tint()
                    }),
                })
            })
            .collect();
        let min_x = nodes.iter().map(|n| n.x).fold(0., f32::min);
        let min_y = nodes.iter().map(|n| n.y).fold(0., f32::min);
        let max_x = nodes.iter().map(|n| n.x + n.width()).fold(400., f32::max);
        let max_y = nodes.iter().map(|n| n.y + n.height()).fold(220., f32::max);
        Self {
            nodes,
            wires,
            graph_bounds: Bounds {
                origin: point(min_x - 24., min_y - 24.),
                size: size(max_x - min_x + 48., max_y - min_y + 48.),
            },
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
    #[allow(dead_code)]
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
