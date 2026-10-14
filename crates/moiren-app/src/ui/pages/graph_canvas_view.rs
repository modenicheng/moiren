//! Graph canvas page: camera-transformed node cards over a canvas-painted
//! grid and wire layer. Ported from graph-canvas.slint.
//!
//! Paint split: `canvas()` paints grid + fill-only bezier wire ribbons; the
//! node layer is absolutely positioned divs so text, icons and hit areas use
//! stock elements at every zoom. Pan = middle-drag or left-drag on empty
//! canvas; zoom = wheel anchored at the cursor.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::graph_canvas::{Camera, CanvasNode, CanvasWorld, wire_ribbon};
use crate::ui::theme;
use crate::ui::widgets::icon::icon_tinted;

/// Renders one node card at `origin` (canvas-local screen px).
fn node_card(node: &CanvasNode, origin: (f32, f32), zoom: f32, selected: bool) -> Stateful<Div> {
    let tint = node.tint();
    let (w, h) = (node.width() * zoom, node.height() * zoom);
    let pad = theme::card::PADDING * zoom;
    let chip = 26. * zoom;
    let title_size = (theme::TEXT_MD * zoom).max(9.);
    let detail_size = (theme::TEXT_XS * zoom).max(8.);
    let node_id: SharedString = node.id.clone().into();

    div()
        .id(node_id)
        .absolute()
        .left(px(origin.0))
        .top(px(origin.1))
        .w(px(w))
        .h(px(h))
        .rounded(px(6.))
        .bg(theme::SURFACE_2)
        .border_1()
        .border_color(if selected { tint } else { theme::BORDER_STRONG })
        .overflow_hidden()
        .p(px(pad))
        .flex()
        .flex_col()
        .gap(px(theme::card::GAP * zoom))
        .cursor_pointer()
        .hover(|s| s.bg(theme::SURFACE_3))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(8. * zoom))
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .size(px(chip))
                        .rounded(px(theme::RADIUS_SM))
                        .bg(theme::SURFACE_3)
                        .items_center()
                        .justify_center()
                        .child(icon_tinted(
                            node.icon.unwrap_or("sliders"),
                            tint,
                            14. * zoom,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .overflow_hidden()
                        .child(
                            div()
                                .text_size(px(title_size))
                                .text_color(theme::TEXT)
                                .font_family(theme::FONT_UI)
                                .line_height(px((title_size * 1.3).ceil()))
                                .truncate()
                                .child(node.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(detail_size))
                                .when(node.is_parameter, |s| {
                                    s.text_color(tint).font_family(theme::FONT_MONO)
                                })
                                .when(!node.is_parameter, |s| s.text_color(theme::TEXT_FAINT))
                                .line_height(px((detail_size * 1.3).ceil()))
                                .truncate()
                                .child(if node.is_parameter && !node.value.is_empty() {
                                    node.value.clone()
                                } else {
                                    node.detail.clone()
                                }),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .w_full()
                .items_center()
                .overflow_hidden()
                .child(if node.is_parameter {
                    // Parameter bar: track + tinted fill proportion.
                    div()
                        .relative()
                        .w_full()
                        .h(px(3. * zoom))
                        .rounded(px(2.))
                        .bg(theme::BORDER_STRONG)
                        .when_some(
                            node.level.map(|level| level * (w - 2. * pad)),
                            |s, fill_w| {
                                s.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .h_full()
                                        .w(px(fill_w))
                                        .rounded(px(2.))
                                        .bg(tint),
                                )
                            },
                        )
                } else {
                    // Meter track (live levels attach later).
                    div()
                        .w_full()
                        .h(px(theme::card::METER_HEIGHT * zoom))
                        .rounded(px(2.))
                        .bg(theme::SURFACE)
                }),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(theme::card::ROUTING_HEIGHT * zoom))
                .w_full()
                .items_center()
                .justify_between()
                .when(node.has_input, |s| {
                    s.child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px((9. * zoom).max(8.)))
                            .text_color(theme::TEXT_MUTED)
                            .child(format!("→ {}", node.input_label)),
                    )
                })
                .when(node.has_output, |s| {
                    s.child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px((9. * zoom).max(8.)))
                            .text_color(theme::TEXT_MUTED)
                            .child(format!("{} →", node.output_label)),
                    )
                }),
        )
}

/// Grid + wire paint layer inside the canvas bounds.
pub fn paint_canvas_layer(
    world: &CanvasWorld,
    camera: &Camera,
    bounds: Bounds<Pixels>,
    selected_wire: Option<&str>,
    window: &mut Window,
) {
    let zoom = camera.zoom;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));

    // Grid: keep 24-48 screen px between lines.
    let grid_world = theme::CANVAS_GRID
        * if zoom < 0.5 {
            4.
        } else if zoom < 1.0 {
            2.
        } else {
            1.
        };
    let step = grid_world * zoom;
    let mut x = ((camera.pan_x % step) + step) % step;
    while x < w {
        window.paint_quad(gpui_kit::fill(
            Bounds {
                origin: point(px(ox + x), px(oy)),
                size: size(px(1.), px(h)),
            },
            theme::GRID_LINE,
        ));
        x += step;
    }
    let mut y = ((camera.pan_y % step) + step) % step;
    while y < h {
        window.paint_quad(gpui_kit::fill(
            Bounds {
                origin: point(px(ox), px(oy + y)),
                size: size(px(w), px(1.)),
            },
            theme::GRID_LINE,
        ));
        y += step;
    }

    // Wire ribbons (fill-only Path) under the node layer.
    for wire in &world.wires {
        let selected = selected_wire == Some(wire.id.as_str());
        let tint = wire.tint.unwrap_or(theme::PRIMARY);
        let from_s = (
            ox + camera.pan_x + wire.from.0 * zoom,
            oy + camera.pan_y + wire.from.1 * zoom,
        );
        let to_s = (
            ox + camera.pan_x + wire.to.0 * zoom,
            oy + camera.pan_y + wire.to.1 * zoom,
        );
        let width = (if selected { 2.0 } else { 1.6 } * zoom).max(0.75);
        let path = wire_ribbon(from_s, to_s, width);
        window.paint_path(path, if selected { theme::PRIMARY } else { tint });
    }
}

/// The full graph canvas view. All interaction reports back through the two
/// callbacks; the parent owns camera + selection state.
/// Shared callback result type: window + app mutation hooks.
type WindowAppFn<A> = Box<dyn Fn(A, &mut Window, &mut App)>;
type RefWindowAppFn<A> = Box<dyn Fn(&A, &mut Window, &mut App)>;
type SelectNodeFn = dyn Fn(&str, &mut Window, &mut App);
type ZoomFn = dyn Fn((f32, Point<Pixels>, Bounds<Pixels>), &mut Window, &mut App);

/// Canvas interaction callbacks, owned by the parent view.
pub struct CanvasCallbacks {
    pub on_background_down: WindowAppFn<(MouseDownEvent, Bounds<Pixels>)>,
    pub on_canvas_mouse_move: WindowAppFn<(Point<Pixels>, Bounds<Pixels>)>,
    pub on_zoom: Rc<ZoomFn>,
    pub on_pan: WindowAppFn<(f32, f32)>,
    pub on_select_node: Rc<SelectNodeFn>,
    pub on_camera_fit: RefWindowAppFn<Camera>,
}

pub fn graph_canvas_view(
    world: Arc<CanvasWorld>,
    camera: Camera,
    selected_node: Option<&str>,
    cb: CanvasCallbacks,
) -> impl IntoElement {
    let CanvasCallbacks {
        on_background_down,
        on_canvas_mouse_move,
        on_zoom,
        on_pan,
        on_select_node,
        on_camera_fit,
    } = cb;
    let wire_paint_world = world.clone();
    let paint_camera = camera;
    let zoom = camera.zoom;

    let node_positions: Vec<(f32, f32)> = world
        .nodes
        .iter()
        .map(|n| (camera.pan_x + n.x * zoom, camera.pan_y + n.y * zoom))
        .collect();

    div()
        .id("graph-canvas")
        .relative()
        .size_full()
        .bg(theme::WINDOW_BG)
        .overflow_hidden()
        .child(canvas(
            {
                let fit_camera = paint_camera;
                let fit_world = world.clone();
                move |bounds, window, cx| {
                    // Auto-fit once (or after Fit clicked) on real layout.
                    if !fit_camera.adjusted {
                        let mut fitted = fit_camera;
                        fitted.fit(bounds.size, fit_world.graph_bounds);
                        on_camera_fit(&fitted, window, cx);
                    }
                    bounds
                }
            },
            move |bounds, _state, window, _cx| {
                paint_canvas_layer(&wire_paint_world, &paint_camera, bounds, None, window)
            },
        ))
        // Background interaction layer: covers the whole canvas area.
        .child(
            div()
                .id("graph-canvas-input")
                .absolute()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_background_down((event.clone(), Bounds::default()), window, cx)
                })
                .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                    on_canvas_mouse_move((event.position, Bounds::default()), window, cx)
                })
                .on_scroll_wheel({
                    let on_zoom = on_zoom.clone();
                    move |event: &ScrollWheelEvent, window, cx| {
                        // Pixel deltas mark precision touchpads; their
                        // two-finger scroll pans, wheel notches zoom.
                        #[cfg(windows)]
                        let control =
                            event.modifiers.control || crate::ui::touchpad::wheel_control();
                        #[cfg(not(windows))]
                        let control = event.modifiers.control;
                        let (dx, dy, continuous) = match event.delta {
                            ScrollDelta::Pixels(delta) => {
                                (f32::from(delta.x), f32::from(delta.y), true)
                            }
                            ScrollDelta::Lines(lines) => (lines.x * 60., lines.y * 60., false),
                        };
                        match crate::ui::scroll_policy::scroll_action(dx, dy, continuous, control) {
                            crate::ui::scroll_policy::ScrollAction::Pan(dx, dy) => {
                                on_pan((dx, dy), window, cx);
                            }
                            crate::ui::scroll_policy::ScrollAction::Zoom(factor) => {
                                on_zoom((factor, event.position, Bounds::default()), window, cx);
                            }
                        }
                    }
                })
                .on_pinch(move |event: &PinchEvent, window, cx| {
                    // Fractional zoom deltas; 0.1 == +10%, multiplicative.
                    let factor = crate::ui::scroll_policy::pinch_zoom_factor(event.delta);
                    on_zoom((factor, event.position, Bounds::default()), window, cx);
                }),
        )
        // Node cards sit above the input layer; each card's own click
        // selects it (selection state flows through the parent view).
        .children(
            world
                .nodes
                .iter()
                .zip(node_positions)
                .map(|(node, origin)| {
                    let selected = selected_node == Some(node.id.as_str());
                    let on_select = on_select_node.clone();
                    let node = node.clone();
                    node_card(&node, origin, zoom, selected)
                        .on_click(move |_, window, cx| on_select(node.id.as_str(), window, cx))
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
        )
}
