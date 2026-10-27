//! Presentation-only graph projection with shared camera, layout and wire geometry.

use gpui_kit::*;
use rust_i18n::t;

use crate::service::AppSnapshot;
use crate::ui::theme;

mod camera;
mod frame;
mod layout;
mod wire;

pub use camera::Camera;
pub use frame::{CanvasFrame, HitTarget, PortDirection, PortTarget};
pub use layout::{NodeLayout, TextLayout};
pub use wire::{WireCurve, WireGeometry};

#[cfg(test)]
mod tests;

/// World-space node descriptor for the candidate session graph.
#[derive(Debug, Clone)]
pub struct CanvasNode {
    pub node_id: Option<moiren_core::graph::NodeId>,
    pub id: String,
    pub name: String,
    pub detail: String,
    pub icon: Option<&'static str>,
    pub x: f32,
    pub y: f32,
    pub wide: bool,
    pub input_count: usize,
    pub input_ports: Vec<moiren_core::graph::PortId>,
    pub output_port: Option<moiren_core::graph::PortId>,
    pub has_output: bool,
    pub tint: Option<Hsla>,
    pub gain: f64,
    pub balance: Option<f64>,
    pub peaks: Option<[f32; 2]>,
    pub kind: moiren_core::graph::NodeKind,
    /// Native body layout is presentation state, never graph topology.
    pub measured_height: Option<f32>,
    pub content_width: f32,
}

impl CanvasNode {
    pub fn width(&self) -> f32 {
        self.content_width
    }
    pub fn height(&self) -> f32 {
        let height = self.measured_height.unwrap_or(200.);
        height.max(80. + self.input_count.saturating_sub(1) as f32 * 18. + theme::card::PADDING)
    }
    pub fn tint(&self) -> Hsla {
        self.tint.unwrap_or(theme::PRIMARY)
    }
}

/// Fresh card with sensible defaults; callers refine via the builder.
fn node(id: &str, name: &str, detail: impl Into<String>) -> CanvasNode {
    CanvasNode {
        node_id: None,
        id: id.into(),
        name: name.into(),
        detail: detail.into(),
        icon: None,
        x: 0.,
        y: 0.,
        wide: false,
        input_count: 0,
        input_ports: Vec::new(),
        output_port: None,
        has_output: false,
        tint: Some(theme::PRIMARY),
        gain: 1.,
        balance: None,
        peaks: None,
        kind: moiren_core::graph::NodeKind::Source,
        measured_height: None,
        content_width: 320.,
    }
}

/// World-space wire between two port anchors.
#[derive(Debug, Clone)]
pub struct CanvasWire {
    pub id: String,
    pub source: String,
    pub destination: String,
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
    #[cfg(feature = "gpui-test")]
    pub fn positioned(
        snapshot: &AppSnapshot,
        positions: &std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    ) -> Self {
        Self::measured(snapshot, positions, &std::collections::BTreeMap::new())
    }

    pub fn measured(
        snapshot: &AppSnapshot,
        positions: &std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
        heights: &std::collections::BTreeMap<moiren_core::graph::NodeId, f32>,
    ) -> Self {
        use moiren_core::graph::NodeKind;
        let graph = &snapshot.graph;
        let order = graph.graph.topological_order().unwrap_or_default();
        let mut depth = std::collections::HashMap::new();
        let mut column_bottoms = std::collections::HashMap::new();
        let mut column_widths = std::collections::BTreeMap::<usize, f32>::new();
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
            let column_bottom = column_bottoms.entry(level).or_insert(80.);
            let key = format!("{id:?}");
            let position = positions
                .get(&id)
                .copied()
                .unwrap_or((32. + level as f32 * 380., *column_bottom));
            let channels = if logical.channels() == 2 {
                "L/R".into()
            } else {
                format!("{} ch", logical.channels())
            };
            let detail = match logical.kind() {
                NodeKind::Source if state.source.is_none() => t!("Choose a source").to_string(),
                NodeKind::Sink if state.output_endpoint_id.is_none() => {
                    t!("Choose an output").to_string()
                }
                NodeKind::Eq => state.builtin.eq().map_or_else(
                    || channels.clone(),
                    |settings| {
                        format!(
                            "{:.0} Hz · {:+.1} dB",
                            settings.frequency_hz, settings.gain_db
                        )
                    },
                ),
                NodeKind::Delay => state.builtin.delay().map_or_else(
                    || channels.clone(),
                    |settings| {
                        t!(
                            "%{ms} ms · %{percent}% wet",
                            ms = format!("{:.0}", settings.delay_ms),
                            percent = format!("{:.0}", settings.mix * 100.)
                        )
                        .to_string()
                    },
                ),
                NodeKind::Recorder => state.builtin.recorder().map_or_else(
                    || channels.clone(),
                    |settings| {
                        t!("%{frames} frame buffer", frames = settings.capacity_frames).to_string()
                    },
                ),
                NodeKind::Meter => state.builtin.meter().map_or_else(
                    || channels.clone(),
                    |settings| {
                        t!("%{frames} frame window", frames = settings.window_frames).to_string()
                    },
                ),
                NodeKind::SpectrumAnalyzer => state.builtin.spectrum().map_or_else(
                    || channels.clone(),
                    |settings| {
                        t!(
                            "FFT %{fft} · hop %{hop}",
                            fft = settings.fft_size,
                            hop = settings.hop_frames
                        )
                        .to_string()
                    },
                ),
                NodeKind::Rack => state.builtin.rack().map_or_else(
                    || t!("Empty rack").to_string(),
                    |settings| {
                        // English keeps its plural; zh-CN shares one form for both keys.
                        if settings.slots.len() == 1 {
                            t!("%{count} stage", count = settings.slots.len()).to_string()
                        } else {
                            t!("%{count} stages", count = settings.slots.len()).to_string()
                        }
                    },
                ),
                _ => channels,
            };
            let (icon, tint) = match logical.kind() {
                NodeKind::Source => (
                    match state.source {
                        Some(crate::service::InputSelection::Physical { .. }) | None => {
                            "figma-input"
                        }
                        Some(crate::service::InputSelection::Process { .. }) => "app",
                        Some(crate::service::InputSelection::Tone { .. }) => "wave",
                    },
                    theme::NODE_SOURCE,
                ),
                NodeKind::Sink => ("figma-output", theme::NODE_SINK),
                NodeKind::Bus => ("figma-bus", theme::NODE_UTILITY),
                NodeKind::Recorder => ("radio", theme::NODE_ANALYSIS),
                NodeKind::Meter => ("activity", theme::NODE_ANALYSIS),
                NodeKind::SpectrumAnalyzer => ("wave", theme::NODE_ANALYSIS),
                NodeKind::Delay => ("clock", theme::NODE_DSP),
                NodeKind::ChannelMapper | NodeKind::Rack => ("route", theme::NODE_UTILITY),
                _ => ("figma-fader", theme::NODE_DSP),
            };
            let mut card = node(
                &key,
                &crate::ui::state::display_label(snapshot, state, logical.kind()),
                detail,
            )
            .at(position.0, position.1)
            .icon(icon)
            .ports(!logical.inputs().is_empty(), !logical.outputs().is_empty())
            .with_tint(tint);
            card.input_count = logical.inputs().len();
            card.node_id = Some(id);
            card.kind = logical.kind();
            card.measured_height = heights.get(&id).copied();
            // Width follows the number of control columns or routing channels;
            // height comes from native layout, including wrapping and errors.
            card.content_width = match logical.kind() {
                NodeKind::Source
                    if matches!(
                        state.source,
                        Some(crate::service::InputSelection::Tone { .. })
                    ) =>
                {
                    244.
                }
                NodeKind::Source | NodeKind::Sink => {
                    268. + crate::ui::widgets::meters::node_meter_width(logical.channels())
                }
                NodeKind::Gain => 244.,
                NodeKind::Pan => {
                    148. + crate::ui::widgets::meters::node_meter_width(logical.channels()) + 20.
                }
                NodeKind::Eq | NodeKind::Compressor | NodeKind::Gate => {
                    3. * 94.
                        + 16.
                        + 20.
                        + crate::ui::widgets::meters::node_meter_width(logical.channels())
                }
                NodeKind::Limiter | NodeKind::Delay => 3. * 92. + 20.,
                NodeKind::ChannelMapper => (logical.channels().min(8) as f32 * 68. + 76.).max(296.),
                NodeKind::Rack => {
                    rack_content_width(state.builtin.rack())
                        + if state
                            .builtin
                            .rack()
                            .is_some_and(|rack| !rack.slots.is_empty())
                        {
                            crate::ui::widgets::meters::node_meter_width(logical.channels()) + 16.
                        } else {
                            0.
                        }
                }
                NodeKind::Bus => {
                    248. + crate::ui::widgets::meters::node_meter_width(logical.channels()) + 20.
                }
                NodeKind::Recorder => {
                    368. + crate::ui::widgets::meters::node_meter_width(logical.channels()) + 16.
                }
                NodeKind::Meter => {
                    184. + crate::ui::widgets::meters::node_meter_width(logical.channels()) + 20.
                }
                NodeKind::SpectrumAnalyzer => {
                    336. + crate::ui::widgets::meters::node_meter_width(logical.channels()) + 16.
                }
            };
            column_widths
                .entry(level)
                .and_modify(|w| *w = w.max(card.width()))
                .or_insert(card.width());
            card.gain = if logical.kind() == NodeKind::Gain {
                state.gain
            } else {
                state.mixer.gain
            };
            card.balance = (logical.kind() == NodeKind::Pan).then_some(state.pan);
            card.input_ports = logical.inputs().iter().map(|port| port.id()).collect();
            card.output_port = logical.outputs().first().map(|port| port.id());
            // Different workstations have different heights; fixed row strides
            // would place a second processor inside a tall compressor card.
            *column_bottom = (*column_bottom).max(card.y + card.height() + 40.);
            nodes.push(card);
        }
        let mut column_x = std::collections::HashMap::new();
        let mut x = 32.;
        for (level, width) in column_widths {
            column_x.insert(level, x);
            x += width + 60.;
        }
        for card in &mut nodes {
            if let Some(id) = card.node_id
                && !positions.contains_key(&id)
            {
                card.x = column_x[&depth[&id]];
            }
        }
        let wires = graph
            .graph
            .edges()
            .iter()
            .filter_map(|edge| {
                let a = nodes.iter().find(|n| n.node_id == Some(edge.src()))?;
                let b = nodes.iter().find(|n| n.node_id == Some(edge.dst()))?;
                let input = graph
                    .graph
                    .get_node(edge.dst())
                    .ok()?
                    .inputs()
                    .iter()
                    .position(|p| p.id() == edge.dst_port())?;
                Some(CanvasWire {
                    id: format!("{:?}", edge.id()),
                    source: a.id.clone(),
                    destination: b.id.clone(),
                    from: NodeLayout::new(a).output?,
                    to: *NodeLayout::new(b).inputs.get(input)?,
                    tint: Some(if edge.params().mute {
                        theme::TEXT_FAINT
                    } else {
                        a.tint()
                    }),
                })
            })
            .collect();
        Self::new(nodes, wires)
    }

    pub fn new(nodes: Vec<CanvasNode>, wires: Vec<CanvasWire>) -> Self {
        let graph_bounds = nodes
            .iter()
            .map(|node| NodeLayout::new(node).card)
            .chain(
                wires
                    .iter()
                    .map(|wire| WireCurve::new(wire.from, wire.to).bounds()),
            )
            .reduce(|bounds, next| bounds.union(&next))
            .unwrap_or(Bounds {
                origin: point(0., 0.),
                size: size(400., 220.),
            });
        Self {
            nodes,
            wires,
            graph_bounds: Bounds {
                origin: graph_bounds.origin - point(8., 8.),
                size: graph_bounds.size + size(16., 16.),
            },
        }
    }
}

/// Use the widest configured stage, rather than the selected stage: switching
/// a selector must not move its own hit target or the Rack's connection ports.
fn rack_content_width(settings: Option<&moiren_engine::rack::RackSettings>) -> f32 {
    use moiren_engine::rack::RackProcessor;
    settings.map_or(248., |rack| {
        rack.slots
            .iter()
            .map(|slot| match slot.processor {
                RackProcessor::Gain { .. } | RackProcessor::Pan { .. } => 268.,
                RackProcessor::ChannelMapper(_) => 324.,
                _ => 304.,
            })
            .fold(248., f32::max)
    })
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
        self.input_count = usize::from(input);
        self.has_output = output;
        self
    }
    fn with_tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
    #[allow(dead_code)]
    fn wide(mut self) -> Self {
        self.wide = true;
        self
    }
}
