use std::sync::Arc;

use gpui_kit::{Bounds, Pixels, Size, point, px, size};

use super::{
    Camera, CanvasWorld, ConnectionCandidate, NodeLayout, RecordAction, WireCurve, WireGeometry,
};

#[derive(Debug, PartialEq, Eq)]
pub enum HitTarget<'a> {
    Port(PortTarget),
    /// A knob dial on a card body; knobs win over the card so drags
    /// adjust the value instead of moving the node.
    Knob {
        node: &'a str,
        control: super::KnobControl,
    },
    /// The header's expander glyph; opens the node detail window.
    Expand {
        node: &'a str,
    },
    /// A source/sink card's binding chip; opens the endpoint menu.
    Chip {
        node: &'a str,
    },
    /// A recorder card's transport slot; the action is the affordance the
    /// phase offers at this position, so the click and the paint agree.
    CardAction {
        node: &'a str,
        action: RecordAction,
    },
    /// A faulted card's "!" badge; opens the fault detail popover.
    Badge {
        node: &'a str,
    },
    Node(&'a str),
    Wire(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortTarget {
    pub id: moiren_core::graph::PortId,
    pub direction: PortDirection,
}

pub struct CanvasFrame {
    pub world: Arc<CanvasWorld>,
    pub camera: Camera,
    pub bounds: Bounds<Pixels>,
    pub nodes: Vec<NodeLayout>,
    pub wires: Vec<WireGeometry>,
}

impl CanvasFrame {
    pub fn new(world: Arc<CanvasWorld>, mut camera: Camera, bounds: Bounds<Pixels>) -> Self {
        if !camera.adjusted {
            camera.fit(bounds.size, world.graph_bounds);
        }
        Self {
            nodes: world
                .nodes
                .iter()
                .map(|node| NodeLayout::new(node).project(camera))
                .collect(),
            wires: world
                .wires
                .iter()
                .map(|wire| WireCurve::new(wire.from, wire.to).project(camera))
                .collect(),
            world,
            camera,
            bounds,
        }
    }

    pub fn local_position(&self, window: (f32, f32)) -> (f32, f32) {
        (
            window.0 - f32::from(self.bounds.origin.x),
            window.1 - f32::from(self.bounds.origin.y),
        )
    }

    pub fn viewport(&self) -> Bounds<f32> {
        Bounds {
            origin: point(0., 0.),
            size: size(
                f32::from(self.bounds.size.width),
                f32::from(self.bounds.size.height),
            ),
        }
    }

    pub fn hit_test(&self, window: (f32, f32)) -> Option<HitTarget<'_>> {
        let local = self.local_position(window);
        if !self.viewport().contains(&point(local.0, local.1)) {
            return None;
        }
        for (index, layout) in self.nodes.iter().enumerate().rev() {
            let node = &self.world.nodes[index];
            // Ports straddle the card edge. Keep their hit area usable at low zoom.
            let near = |anchor: (f32, f32)| (anchor.0 - local.0).hypot(anchor.1 - local.1) <= 8.;
            for (port, anchor) in node.input_ports.iter().zip(&layout.inputs) {
                if near(*anchor) {
                    return Some(HitTarget::Port(PortTarget {
                        id: port.id,
                        direction: PortDirection::Input,
                    }));
                }
            }
            if let Some(port) = node.output_port
                && layout.output.is_some_and(near)
            {
                return Some(HitTarget::Port(PortTarget {
                    id: port.id,
                    direction: PortDirection::Output,
                }));
            }
            if layout.expand.contains(&point(local.0, local.1)) {
                return Some(HitTarget::Expand { node: &node.id });
            }
            for (knob, knob_layout) in node.knobs.iter().zip(&layout.knobs) {
                if knob_layout.dial.contains(&point(local.0, local.1)) {
                    return Some(HitTarget::Knob {
                        node: &node.id,
                        control: knob.control,
                    });
                }
            }
            if layout
                .chip
                .as_ref()
                .is_some_and(|chip| chip.bounds.contains(&point(local.0, local.1)))
            {
                return Some(HitTarget::Chip { node: &node.id });
            }
            // Transport slots only hit while the phase offers them, exactly
            // as the strip paints them; they win over the card beneath.
            if let Some(strip) = layout.recording.as_ref()
                && let Some(recording) = node.recording.as_ref()
            {
                let slot = |bounds: &Bounds<f32>, action: RecordAction| {
                    bounds
                        .contains(&point(local.0, local.1))
                        .then(|| action.available(recording.phase))
                        .flatten()
                        .map(|action| HitTarget::CardAction {
                            node: &node.id,
                            action,
                        })
                };
                if let Some(hit) = slot(&strip.record, RecordAction::Record) {
                    return Some(hit);
                }
                if let Some(hit) = slot(&strip.pause, RecordAction::Pause) {
                    return Some(hit);
                }
                if let Some(hit) = slot(&strip.stop, RecordAction::Stop) {
                    return Some(hit);
                }
            }
            // The "!" badge only exists on a faulted card, so the gate and
            // the paint agree; it wins over the card beneath it.
            if node.faulted && layout.badge.contains(&point(local.0, local.1)) {
                return Some(HitTarget::Badge { node: &node.id });
            }
            if layout.card.contains(&point(local.0, local.1)) {
                return Some(HitTarget::Node(&node.id));
            }
        }
        self.wires
            .iter()
            .enumerate()
            .rev()
            .map(|(index, geometry)| (index, geometry.distance(local)))
            .filter(|(_, distance)| *distance <= 6.)
            .min_by(|first, second| first.1.total_cmp(&second.1))
            .map(|(index, _)| HitTarget::Wire(&self.world.wires[index].id))
    }

    pub fn port_position(&self, port: PortTarget) -> Option<(f32, f32)> {
        self.world
            .nodes
            .iter()
            .zip(&self.nodes)
            .find_map(|(node, layout)| match port.direction {
                PortDirection::Input => node
                    .input_ports
                    .iter()
                    .position(|candidate| candidate.id == port.id)
                    .and_then(|index| layout.inputs.get(index).copied()),
                PortDirection::Output => node
                    .output_port
                    .filter(|candidate| candidate.id == port.id)
                    .and(layout.output),
            })
    }

    /// Where a connection drag from `drag` would land at `window`: the
    /// port under the cursor when one is hit (any direction —
    /// same-direction targets come back `Incompatible` so the verdict can
    /// be painted), else the topmost card under the cursor resolves to its
    /// compatible port: the input nearest the cursor for an output drag,
    /// the output for an input drag (design 18 A2). `None` when nothing
    /// compatible is under the cursor; the drag simply cancels, as before.
    pub fn connection_candidate(
        &self,
        window: (f32, f32),
        drag: PortTarget,
    ) -> Option<ConnectionCandidate> {
        let local = self.local_position(window);
        if !self.viewport().contains(&point(local.0, local.1)) {
            return None;
        }
        let near = |anchor: (f32, f32)| (anchor.0 - local.0).hypot(anchor.1 - local.1) <= 8.;
        for (index, layout) in self.nodes.iter().enumerate().rev() {
            let node = &self.world.nodes[index];
            let port_target = node
                .input_ports
                .iter()
                .zip(&layout.inputs)
                .map(|(port, anchor)| {
                    (
                        PortTarget {
                            id: port.id,
                            direction: PortDirection::Input,
                        },
                        *anchor,
                    )
                })
                .chain(node.output_port.zip(layout.output).map(|(port, anchor)| {
                    (
                        PortTarget {
                            id: port.id,
                            direction: PortDirection::Output,
                        },
                        anchor,
                    )
                }))
                .find(|(_, anchor)| near(*anchor))
                .map(|(target, _)| target);
            if let Some(target) = port_target {
                let node_id = node.node_id?;
                return Some(self.world.resolve_candidate(drag, target, node_id, false));
            }
            if layout.card.contains(&point(local.0, local.1)) {
                let node_id = node.node_id?;
                let target = match drag.direction {
                    PortDirection::Output => node
                        .input_ports
                        .iter()
                        .zip(&layout.inputs)
                        .min_by(|(_, a), (_, b)| {
                            (a.0 - local.0)
                                .hypot(a.1 - local.1)
                                .total_cmp(&(b.0 - local.0).hypot(b.1 - local.1))
                        })
                        .map(|(port, _)| PortTarget {
                            id: port.id,
                            direction: PortDirection::Input,
                        }),
                    PortDirection::Input => node.output_port.map(|port| PortTarget {
                        id: port.id,
                        direction: PortDirection::Output,
                    }),
                }?;
                return Some(self.world.resolve_candidate(drag, target, node_id, true));
            }
        }
        None
    }

    pub fn overview(&self, viewport: Size<f32>) -> Overview {
        let visible = self.camera.visible_world(self.viewport().size);
        let content = self.world.graph_bounds.union(&visible);
        let padding = 3.;
        let zoom = ((viewport.width - padding * 2.).max(1.) / content.size.width.max(1.))
            .min((viewport.height - padding * 2.).max(1.) / content.size.height.max(1.));
        let camera = Camera {
            pan_x: (viewport.width - content.size.width * zoom) / 2. - content.origin.x * zoom,
            pan_y: (viewport.height - content.size.height * zoom) / 2. - content.origin.y * zoom,
            zoom,
            adjusted: true,
        };
        Overview {
            viewport: camera.project_rect(visible),
            camera,
        }
    }

    pub fn window_rect(&self, local: Bounds<f32>) -> Bounds<Pixels> {
        Bounds {
            origin: self.bounds.origin + point(px(local.origin.x), px(local.origin.y)),
            size: size(px(local.size.width), px(local.size.height)),
        }
    }
}

pub struct Overview {
    pub camera: Camera,
    pub viewport: Bounds<f32>,
}
