use std::sync::Arc;

use gpui_kit::{Bounds, Pixels, Size, point, px, size};

use super::{Camera, CanvasWorld, NodeLayout, WireCurve, WireGeometry};

#[derive(Debug, PartialEq, Eq)]
pub enum HitTarget<'a> {
    Port(PortTarget),
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
                        id: *port,
                        direction: PortDirection::Input,
                    }));
                }
            }
            if let Some(port) = node.output_port
                && layout.output.is_some_and(near)
            {
                return Some(HitTarget::Port(PortTarget {
                    id: port,
                    direction: PortDirection::Output,
                }));
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
                    .position(|id| *id == port.id)
                    .and_then(|index| layout.inputs.get(index).copied()),
                PortDirection::Output => (node.output_port == Some(port.id))
                    .then_some(layout.output)
                    .flatten(),
            })
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
