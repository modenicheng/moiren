use gpui_kit::{Bounds, Path, PathBuilder, Pixels, point, px, size};

use super::Camera;

type Position = (f32, f32);

#[derive(Debug, Clone, Copy)]
pub struct WireCurve {
    pub controls: [Position; 4],
}

impl WireCurve {
    pub fn new(from: Position, to: Position) -> Self {
        let handle = ((to.0 - from.0).abs() * 0.4).max(24.);
        Self {
            controls: [from, (from.0 + handle, from.1), (to.0 - handle, to.1), to],
        }
    }

    pub fn bounds(&self) -> Bounds<f32> {
        let left = self
            .controls
            .iter()
            .map(|position| position.0)
            .fold(f32::INFINITY, f32::min);
        let top = self
            .controls
            .iter()
            .map(|position| position.1)
            .fold(f32::INFINITY, f32::min);
        let right = self
            .controls
            .iter()
            .map(|position| position.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = self
            .controls
            .iter()
            .map(|position| position.1)
            .fold(f32::NEG_INFINITY, f32::max);
        Bounds {
            origin: point(left, top),
            size: size(right - left, bottom - top),
        }
    }

    pub fn project(&self, camera: Camera) -> WireGeometry {
        let controls = self
            .controls
            .map(|position| camera.world_to_canvas(position));
        let mut points = vec![controls[0]];
        if controls[0] != controls[3] {
            flatten(controls, 0, &mut points);
        }
        WireGeometry { points }
    }
}

#[derive(Debug, Clone)]
pub struct WireGeometry {
    pub points: Vec<Position>,
}

impl WireGeometry {
    pub fn distance(&self, position: Position) -> f32 {
        self.points
            .windows(2)
            .map(|segment| segment_distance(position, segment[0], segment[1]))
            .fold(f32::INFINITY, f32::min)
    }

    pub fn stroke(&self, width: f32, origin: Position) -> anyhow::Result<Path<Pixels>> {
        let mut builder = PathBuilder::stroke(px(width));
        if let Some(first) = self.points.first() {
            builder.move_to(point(px(origin.0 + first.0), px(origin.1 + first.1)));
            for position in self.points.iter().skip(1) {
                builder.line_to(point(px(origin.0 + position.0), px(origin.1 + position.1)));
            }
        }
        builder.build()
    }
}

fn flatten(controls: [Position; 4], depth: usize, points: &mut Vec<Position>) {
    let [start, first, second, end] = controls;
    if depth >= 12
        || (segment_distance(first, start, end).max(segment_distance(second, start, end)) <= 0.2)
    {
        points.push(end);
        return;
    }
    let first_half = midpoint(start, first);
    let middle_half = midpoint(first, second);
    let last_half = midpoint(second, end);
    let left = midpoint(first_half, middle_half);
    let right = midpoint(middle_half, last_half);
    let middle = midpoint(left, right);
    flatten([start, first_half, left, middle], depth + 1, points);
    flatten([middle, right, last_half, end], depth + 1, points);
}

fn midpoint(first: Position, second: Position) -> Position {
    ((first.0 + second.0) / 2., (first.1 + second.1) / 2.)
}

fn segment_distance(position: Position, start: Position, end: Position) -> f32 {
    let delta = (end.0 - start.0, end.1 - start.1);
    let length_squared = delta.0 * delta.0 + delta.1 * delta.1;
    let fraction = if length_squared > f32::EPSILON {
        ((position.0 - start.0) * delta.0 + (position.1 - start.1) * delta.1) / length_squared
    } else {
        0.
    }
    .clamp(0., 1.);
    (position.0 - start.0 - fraction * delta.0).hypot(position.1 - start.1 - fraction * delta.1)
}
