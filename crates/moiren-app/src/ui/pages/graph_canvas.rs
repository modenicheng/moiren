//! Presentation-only graph projection with shared camera, layout and wire geometry.

use gpui_kit::*;
use rust_i18n::t;

use crate::service::AppSnapshot;
use crate::ui::theme;

mod camera;
mod frame;
mod knob;
mod layout;
mod wire;

pub use camera::Camera;
pub use frame::{CanvasFrame, HitTarget, PortDirection, PortTarget};
pub use knob::{CardKnob, KnobControl};
pub use layout::{KnobLayout, MeterLayout, NodeLayout, TextLayout};
pub use wire::{WireCurve, WireGeometry};

#[cfg(test)]
mod tests;

/// One port on a canvas node: identity plus the channel count connection
/// compatibility is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasPort {
    pub id: moiren_core::graph::PortId,
    pub channels: usize,
}

/// Where a connection drag would land, with the app-side compatibility
/// verdict that drives hover feedback and the release path. Commit-side
/// validation stays authoritative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConnectionCandidate {
    pub port: PortTarget,
    /// The drop resolved through the node body rather than the port dot.
    pub from_body: bool,
    /// Node the candidate port belongs to, for the body halo.
    pub node: moiren_core::graph::NodeId,
    pub status: CandidateStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateStatus {
    /// Free compatible port: the drop connects.
    Valid,
    /// The destination input already has an edge: the drop displaces it.
    Displacing,
    /// Same-direction target or channel mismatch.
    Incompatible,
    /// The edge would close a cycle, including self-connection.
    Cycles,
}

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
    pub input_count: usize,
    pub input_ports: Vec<CanvasPort>,
    pub output_port: Option<CanvasPort>,
    pub has_output: bool,
    pub tint: Option<Hsla>,
    /// Card states beyond selection: a fault landed on this node, or the
    /// engine is re-preparing it (design 18 B1).
    pub faulted: bool,
    pub preparing: bool,
    /// Body region height in world units; kinds with richer bodies extend
    /// this and the port rail follows.
    pub body_height: f32,
    /// Curve-style nodes run their meter vertically along the body's
    /// right edge; everything else uses the horizontal strip.
    pub meter_vertical: bool,
    pub channels: usize,
    /// Knob controls exposed on the body (design 18 B3); empty for kinds
    /// whose body is informational.
    pub knobs: Vec<CardKnob>,
}

impl CanvasNode {
    pub fn width(&self) -> f32 {
        theme::NODE_WIDTH
    }
    pub fn height(&self) -> f32 {
        let meter_total = theme::card::METER_CHANNELS as f32 * theme::card::METER_H
            + theme::card::METER_CHANNEL_GAP;
        let body_bottom = theme::card::BODY_TOP + self.body_height;
        let ports_at =
            (body_bottom + meter_total + theme::card::PORT_BOTTOM).max(theme::card::PORT_Y);
        ports_at
            + self.input_count.saturating_sub(1) as f32 * theme::card::PORT_SPACING
            + theme::card::PORT_BOTTOM
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
        input_count: 0,
        input_ports: Vec::new(),
        output_port: None,
        has_output: false,
        tint: Some(theme::PRIMARY),
        faulted: false,
        preparing: false,
        body_height: theme::card::BODY_H,
        meter_vertical: false,
        channels: 2,
        knobs: Vec::new(),
    }
}

/// World-space wire between two port anchors.
#[derive(Debug, Clone)]
pub struct CanvasWire {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub src: moiren_core::graph::NodeId,
    pub dst: moiren_core::graph::NodeId,
    pub dst_port: moiren_core::graph::PortId,
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
        positions: &std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    ) -> Self {
        use moiren_core::graph::NodeKind;
        let graph = &snapshot.graph;
        // Card states (design 18 B1): a fault marks the node it names, or
        // both endpoint cards of a route; the preparing set comes straight
        // from the service's structural-edit tracking.
        let node_of_port =
            |port: moiren_core::graph::PortId| -> Option<moiren_core::graph::NodeId> {
                graph
                    .graph
                    .nodes()
                    .iter()
                    .find(|node| {
                        node.inputs().iter().any(|p| p.id() == port)
                            || node.outputs().iter().any(|p| p.id() == port)
                    })
                    .map(|node| node.id())
            };
        let mut faulted = std::collections::BTreeSet::new();
        for fault in &graph.faults {
            match fault.subject {
                crate::service::GraphSubject::Node(id) => {
                    faulted.insert(id);
                }
                crate::service::GraphSubject::Route(source, destination) => {
                    faulted.extend(node_of_port(source));
                    faulted.extend(node_of_port(destination));
                }
                crate::service::GraphSubject::Graph => {}
            }
        }
        let preparing: std::collections::BTreeSet<moiren_core::graph::NodeId> =
            graph.preparing_nodes.iter().copied().collect();
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
                .get(&id)
                .copied()
                .unwrap_or((32. + level as f32 * 330., 80. + *row as f32 * 170.));
            *row += 1;
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
                &crate::ui::presentation::display_label(snapshot, state, logical.kind()),
                detail,
            )
            .at(position.0, position.1)
            .icon(icon)
            .ports(!logical.inputs().is_empty(), !logical.outputs().is_empty())
            .with_tint(state.tint.and_then(theme::node_tint).unwrap_or(tint));
            card.input_count = logical.inputs().len();
            card.node_id = Some(id);
            card.faulted = faulted.contains(&id);
            card.preparing = preparing.contains(&id);
            card.channels = logical.channels();
            let knob = |control: KnobControl, label: String| {
                let value = control.current(state);
                CardKnob {
                    control,
                    label,
                    value_text: control.value_text(value),
                    position: control.position(value),
                }
            };
            match logical.kind() {
                NodeKind::Gain => {
                    card.knobs
                        .push(knob(KnobControl::GainLevel, t!("Gain").to_string()));
                }
                NodeKind::Pan => {
                    card.knobs
                        .push(knob(KnobControl::PanPosition, t!("Balance").to_string()));
                }
                NodeKind::Delay => {
                    card.knobs
                        .push(knob(KnobControl::DelayTime, t!("Time").to_string()));
                }
                NodeKind::Source
                    if matches!(
                        state.source,
                        Some(crate::service::InputSelection::Tone { .. })
                    ) =>
                {
                    card.knobs.push(knob(
                        KnobControl::ToneFrequency,
                        t!("Frequency").to_string(),
                    ));
                }
                _ => {}
            }
            if !card.knobs.is_empty() {
                card.body_height = theme::card::KNOB_BODY_H;
            }
            card.input_ports = logical
                .inputs()
                .iter()
                .map(|port| CanvasPort {
                    id: port.id(),
                    channels: port.channels(),
                })
                .collect();
            card.output_port = logical.outputs().first().map(|port| CanvasPort {
                id: port.id(),
                channels: port.channels(),
            });
            nodes.push(card);
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
                    src: edge.src(),
                    dst: edge.dst(),
                    dst_port: edge.dst_port(),
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

    /// The node a port belongs to, by direction-aware identity.
    pub fn node_of_port(&self, port: PortTarget) -> Option<moiren_core::graph::NodeId> {
        self.nodes
            .iter()
            .find(|node| match port.direction {
                PortDirection::Input => node.input_ports.iter().any(|p| p.id == port.id),
                PortDirection::Output => node.output_port.is_some_and(|p| p.id == port.id),
            })
            .and_then(|node| node.node_id)
    }

    /// The channel count of a port on this world's nodes.
    fn channels_of(&self, port: PortTarget) -> Option<usize> {
        self.nodes.iter().find_map(|node| match port.direction {
            PortDirection::Input => node
                .input_ports
                .iter()
                .find(|p| p.id == port.id)
                .map(|p| p.channels),
            PortDirection::Output => node
                .output_port
                .filter(|p| p.id == port.id)
                .map(|p| p.channels),
        })
    }

    /// Whether an edge `src -> dst` would close a cycle over this world's
    /// wires: it does when `src` is already reachable from `dst`. Mirrors
    /// the core's `would_cycle` (private there) for hover feedback only;
    /// commit-side validation stays authoritative.
    pub fn would_cycle(
        &self,
        src: moiren_core::graph::NodeId,
        dst: moiren_core::graph::NodeId,
    ) -> bool {
        let mut stack = vec![dst];
        let mut visited = std::collections::BTreeSet::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current == src {
                return true;
            }
            for wire in &self.wires {
                if wire.src == current {
                    stack.push(wire.dst);
                }
            }
        }
        false
    }

    /// Compatibility verdict for a drag `drag` released onto `target`
    /// (a port of `node`). The displaced edge, when one exists, is the one
    /// feeding the destination input whichever end was dragged.
    pub fn resolve_candidate(
        &self,
        drag: PortTarget,
        target: PortTarget,
        node: moiren_core::graph::NodeId,
        from_body: bool,
    ) -> ConnectionCandidate {
        let status = if target.direction == drag.direction {
            CandidateStatus::Incompatible
        } else {
            let (source, destination) = match drag.direction {
                PortDirection::Output => (drag.id, target.id),
                PortDirection::Input => (target.id, drag.id),
            };
            let source_node = self.node_of_port(PortTarget {
                id: source,
                direction: PortDirection::Output,
            });
            let destination_node = self.node_of_port(PortTarget {
                id: destination,
                direction: PortDirection::Input,
            });
            match (source_node, destination_node) {
                (Some(src), Some(dst)) if src == dst => CandidateStatus::Cycles,
                (Some(src), Some(dst)) if self.would_cycle(src, dst) => CandidateStatus::Cycles,
                _ if self.channels_of(drag).is_none()
                    || self.channels_of(target).is_none()
                    || self.channels_of(drag) != self.channels_of(target) =>
                {
                    CandidateStatus::Incompatible
                }
                _ if self.wires.iter().any(|wire| wire.dst_port == destination) => {
                    CandidateStatus::Displacing
                }
                _ => CandidateStatus::Valid,
            }
        };
        ConnectionCandidate {
            port: target,
            from_body,
            node,
            status,
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
        self.input_count = usize::from(input);
        self.has_output = output;
        self
    }
    fn with_tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
}
