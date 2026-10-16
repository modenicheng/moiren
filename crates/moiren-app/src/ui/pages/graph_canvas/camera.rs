use gpui_kit::{Bounds, Pixels, Size, point, size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
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
    pub fn world_to_canvas(&self, world: (f32, f32)) -> (f32, f32) {
        (
            world.0 * self.zoom + self.pan_x,
            world.1 * self.zoom + self.pan_y,
        )
    }

    pub fn canvas_to_world(&self, canvas: (f32, f32)) -> (f32, f32) {
        self.canvas_delta_to_world((canvas.0 - self.pan_x, canvas.1 - self.pan_y))
    }

    pub fn canvas_delta_to_world(&self, delta: (f32, f32)) -> (f32, f32) {
        (delta.0 / self.zoom, delta.1 / self.zoom)
    }

    pub fn project_rect(&self, world: Bounds<f32>) -> Bounds<f32> {
        let origin = self.world_to_canvas((world.origin.x, world.origin.y));
        Bounds {
            origin: point(origin.0, origin.1),
            size: world.size * self.zoom,
        }
    }

    pub fn visible_world(&self, viewport: Size<f32>) -> Bounds<f32> {
        let origin = self.canvas_to_world((0., 0.));
        Bounds {
            origin: point(origin.0, origin.1),
            size: viewport / self.zoom,
        }
    }

    pub fn zoom_by(&mut self, factor: f32, anchor: (f32, f32)) {
        if factor.is_finite() && factor > 0. {
            if (0.25..=2.).contains(&self.zoom) {
                self.zoom_at(self.zoom * factor, anchor);
            } else {
                self.set_zoom_at(
                    (self.zoom * factor).clamp(f32::EPSILON, self.zoom.max(2.)),
                    anchor,
                );
            }
        }
    }

    pub fn zoom_at(&mut self, next_zoom: f32, anchor: (f32, f32)) {
        if next_zoom.is_finite() {
            self.set_zoom_at(next_zoom.clamp(0.25, 2.), anchor);
        }
    }

    fn set_zoom_at(&mut self, next_zoom: f32, anchor: (f32, f32)) {
        if !next_zoom.is_finite() || !anchor.0.is_finite() || !anchor.1.is_finite() {
            return;
        }
        let world = self.canvas_to_world(anchor);
        self.zoom = next_zoom;
        self.pan_x = anchor.0 - world.0 * self.zoom;
        self.pan_y = anchor.1 - world.1 * self.zoom;
        self.adjusted = true;
    }

    pub fn pan_by(&mut self, delta_x: f32, delta_y: f32) {
        if delta_x.is_finite() && delta_y.is_finite() {
            self.pan_x += delta_x;
            self.pan_y += delta_y;
            self.adjusted = true;
        }
    }

    pub fn zoom_around_center(&mut self, canvas: Option<(f32, f32)>, factor: f32) {
        let anchor = canvas.map_or((0., 0.), |(width, height)| (width / 2., height / 2.));
        self.zoom_by(factor, anchor);
    }

    pub fn fit(&mut self, canvas: Size<Pixels>, graph: Bounds<f32>) {
        let width = f32::from(canvas.width).max(1.);
        let height = f32::from(canvas.height).max(1.);
        let gutter = 24f32.min(width / 8.).min(height / 8.);
        let top = 72f32.min(height / 3.);
        let available = size(
            (width - 2. * gutter).max(1.),
            (height - top - gutter).max(1.),
        );
        self.zoom = (available.width / graph.size.width.max(1.))
            .min(available.height / graph.size.height.max(1.))
            .clamp(f32::EPSILON, 1.);
        self.pan_x = gutter + (available.width - graph.size.width * self.zoom) / 2.
            - graph.origin.x * self.zoom;
        self.pan_y = top + (available.height - graph.size.height * self.zoom) / 2.
            - graph.origin.y * self.zoom;
    }
}
