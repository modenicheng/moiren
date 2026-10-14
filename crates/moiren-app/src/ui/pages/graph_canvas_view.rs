//! Graph canvas page: camera-transformed grid, wire ribbons and node cards,
//! all painted inside one `canvas()` element so text re-rasterizes at every
//! zoom level (gpui `Path`/`Quad` are fill-only; div scaling would blur text
//! and misalign borders at fractional zoom).
//!
//! Interaction is a transparent overlay div; node hit-testing uses the same
//! camera math as painting against the canvas bounds captured at prepaint.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::graph_canvas::{Camera, CanvasNode, CanvasWorld, wire_ribbon};
use crate::ui::theme;

/// Screen-space rect of a node card under the given camera and canvas origin.
pub fn node_screen_rect(node: &CanvasNode, camera: &Camera, origin: (f32, f32)) -> Bounds<f32> {
    Bounds {
        origin: gpui_kit::point(
            origin.0 + camera.pan_x + node.x * camera.zoom,
            origin.1 + camera.pan_y + node.y * camera.zoom,
        ),
        size: gpui_kit::size(node.width() * camera.zoom, node.height() * camera.zoom),
    }
}

/// Shape one text line; truncates with an ellipsis when it exceeds `avail`.
fn shape_text(
    window: &Window,
    text: &str,
    font_size: f32,
    family: &str,
    color: Hsla,
    avail: f32,
) -> (ShapedLine, f32) {
    let font = || gpui_kit::font(family);
    let mut current = text.to_string();
    let mut line = window.text_system().shape_line(
        current.clone().into(),
        px(font_size),
        &run(&current, font(), color),
        None,
    );
    let mut width = f32::from(line.width());
    while width > avail && current.chars().count() > 2 {
        let cut = current
            .char_indices()
            .nth(current.chars().count().saturating_sub(2))
            .map(|(i, _)| i)
            .unwrap_or(0);
        current.truncate(cut);
        current.push('…');
        line = window.text_system().shape_line(
            current.clone().into(),
            px(font_size),
            &run(&current, font(), color),
            None,
        );
        width = f32::from(line.width());
    }
    (line, width)
}

fn run(text: &str, font: Font, color: Hsla) -> [TextRun; 1] {
    [TextRun {
        len: text.len(),
        font,
        color,
        ..TextRun::default()
    }]
}

/// Paint one node card at canvas-local screen coordinates.
///
/// All card metrics come from the world-space design sizes scaled by the
/// camera zoom; text is shaped at the scaled font size so it stays crisp.
fn paint_node_card(
    node: &CanvasNode,
    camera: &Camera,
    origin: (f32, f32),
    selected: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let z = camera.zoom;
    let tint = node.tint();
    let rect = node_screen_rect(node, camera, origin);
    let (ox, oy) = (rect.origin.x, rect.origin.y);
    let (w, h) = (rect.size.width, rect.size.height);
    let pad = theme::card::PADDING * z;
    let chip = 26. * z;

    // Card base: rounded quad with a one-pixel border (selection tinted).
    window.paint_quad(PaintQuad {
        bounds: Bounds {
            origin: point(px(ox), px(oy)),
            size: size(px(w), px(h)),
        },
        corner_radii: (6. * z).into(),
        background: theme::SURFACE_2.into(),
        border_widths: (1.).into(),
        border_color: if selected { tint } else { theme::BORDER_STRONG },
        border_style: BorderStyle::default(),
    });

    // Header: icon chip + title/detail text block, vertically centered.
    let chip_rect = Bounds {
        origin: point(px(ox + pad), px(oy + pad)),
        size: size(px(chip), px(chip)),
    };
    window.paint_quad(PaintQuad {
        bounds: chip_rect,
        corner_radii: (theme::RADIUS_SM * z).into(),
        background: theme::SURFACE_3.into(),
        border_widths: (0.).into(),
        border_color: gpui_kit::transparent_black(),
        border_style: BorderStyle::default(),
    });
    if let Some(icon) = node.icon {
        let _ = window.paint_svg(
            chip_rect,
            format!("icons/{icon}.svg").into(),
            None,
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }

    let title_size = (theme::TEXT_MD * z).max(9.);
    let detail_size = (theme::TEXT_XS * z).max(8.);
    let text_x = ox + pad + chip + 8. * z;
    let avail = ox + w - pad - text_x;
    let block_h = (title_size + detail_size) * 1.3;
    let text_y = oy + pad + (chip - block_h) / 2.;
    let (title, _) = shape_text(
        window,
        &node.name,
        title_size,
        theme::FONT_UI,
        theme::TEXT,
        avail,
    );
    let _ = title.paint(
        point(px(text_x), px(text_y)),
        px(title_size * 1.3),
        TextAlign::Left,
        None,
        window,
        cx,
    );
    let detail_color = if node.is_parameter {
        tint
    } else {
        theme::TEXT_FAINT
    };
    let detail_text = if node.is_parameter && !node.value.is_empty() {
        &node.value
    } else {
        &node.detail
    };
    let detail_family = if node.is_parameter {
        theme::FONT_MONO
    } else {
        theme::FONT_UI
    };
    let (detail, _) = shape_text(
        window,
        detail_text,
        detail_size,
        detail_family,
        detail_color,
        avail,
    );
    let _ = detail.paint(
        point(px(text_x), px(text_y + title_size * 1.3)),
        px(detail_size * 1.3),
        TextAlign::Left,
        None,
        window,
        cx,
    );

    // Parameter bar or meter track.
    let bar_y = oy + theme::card::METER_Y * z;
    let bar_w = w - 2. * pad;
    if node.is_parameter {
        let bar_h = 3. * z;
        window.paint_quad(gpui_kit::fill(
            Bounds {
                origin: point(px(ox + pad), px(bar_y)),
                size: size(px(bar_w), px(bar_h)),
            },
            theme::BORDER_STRONG,
        ));
        if let Some(level) = node.level {
            let fill_w = (level * bar_w).clamp(0., bar_w);
            window.paint_quad(gpui_kit::fill(
                Bounds {
                    origin: point(px(ox + pad), px(bar_y)),
                    size: size(px(fill_w), px(bar_h)),
                },
                tint,
            ));
        }
    } else {
        let bar_h = theme::card::METER_HEIGHT * z;
        window.paint_quad(gpui_kit::fill(
            Bounds {
                origin: point(px(ox + pad), px(bar_y)),
                size: size(px(bar_w), px(bar_h)),
            },
            theme::SURFACE,
        ));
    }

    // Routing labels.
    let label_size = (9. * z).max(7.);
    let label_y = oy + (theme::card::METER_Y + theme::card::METER_HEIGHT + theme::card::GAP) * z;
    if node.has_input {
        let (line, width) = shape_text(
            window,
            &format!("→ {}", node.input_label),
            label_size,
            theme::FONT_MONO,
            theme::TEXT_MUTED,
            bar_w / 2.,
        );
        let _ = line.paint(
            point(px(ox + pad), px(label_y)),
            px(label_size * 1.3),
            TextAlign::Left,
            None,
            window,
            cx,
        );
        let _ = width;
    }
    if node.has_output {
        let (line, width) = shape_text(
            window,
            &format!("{} →", node.output_label),
            label_size,
            theme::FONT_MONO,
            theme::TEXT_MUTED,
            bar_w / 2.,
        );
        let _ = line.paint(
            point(px(ox + w - pad - width), px(label_y)),
            px(label_size * 1.3),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// Grid + wires + node cards inside the canvas bounds.
pub fn paint_canvas_layer(
    world: &CanvasWorld,
    camera: &Camera,
    bounds: Bounds<Pixels>,
    selected_node: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) {
    let zoom = camera.zoom;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));

    // Grid: keep roughly 24-48 screen px between lines.
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
        let selected =
            selected_node.is_some_and(|id| id == wire.id.split('-').next().unwrap_or_default());
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

    // Node cards on top of the wires.
    for node in &world.nodes {
        let selected = selected_node == Some(node.id.as_str());
        paint_node_card(node, camera, (ox, oy), selected, window, cx);
    }
}

/// Shared callback result type: window + app mutation hooks.
type WindowAppFn<A> = Box<dyn Fn(A, &mut Window, &mut App)>;
type WindowAppFn2<A> = dyn Fn(A, &mut Window, &mut App);
type CameraFitFn = dyn Fn(&Camera, Size<Pixels>, &mut Window, &mut App);
type SelectNodeFn = dyn Fn(&str, &mut Window, &mut App);
type ZoomFn = dyn Fn((f32, Point<Pixels>), &mut Window, &mut App);

/// Canvas interaction callbacks, owned by the parent view.
pub struct CanvasCallbacks {
    pub on_background_down: Rc<WindowAppFn2<(MouseDownEvent, Bounds<Pixels>)>>,
    pub on_canvas_mouse_move: WindowAppFn<(Point<Pixels>, Option<MouseButton>)>,
    /// Zoom by `factor` anchored at the canvas-local point.
    pub on_zoom: Rc<ZoomFn>,
    pub on_pan: WindowAppFn<(f32, f32)>,
    pub on_select_node: Rc<SelectNodeFn>,
    pub on_camera_fit: Rc<CameraFitFn>,
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
    let selected_node = selected_node.map(str::to_string);
    // Canvas layout bounds, captured at prepaint so overlay handlers can
    // convert window coords to canvas-local coords and hit-test node cards.
    let canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));

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
                let seen_bounds = canvas_bounds.clone();
                move |bounds, window, cx| {
                    seen_bounds.set(Some(bounds));
                    // Auto-fit once (or after Fit clicked) on real layout.
                    if !fit_camera.adjusted {
                        let mut fitted = fit_camera;
                        fitted.fit(bounds.size, fit_world.graph_bounds);
                        on_camera_fit(&fitted, bounds.size, window, cx);
                    }
                    bounds
                }
            },
            move |bounds, _state, window, cx| {
                paint_canvas_layer(
                    &wire_paint_world,
                    &paint_camera,
                    bounds,
                    selected_node.as_deref(),
                    window,
                    cx,
                )
            },
        ))
        // Interaction overlay: hit-tests painted node cards via the camera.
        .child(
            div()
                .id("graph-canvas-input")
                .absolute()
                .size_full()
                .on_mouse_down(MouseButton::Left, {
                    let on_background_down = on_background_down.clone();
                    let on_select_node = on_select_node.clone();
                    let hit_camera = paint_camera;
                    let hit_world = world.clone();
                    let seen_bounds = canvas_bounds.clone();
                    move |event, window, cx| {
                        // Node hit-test: topmost card under the cursor wins.
                        if let Some(bounds) = seen_bounds.get() {
                            let local = (
                                f32::from(event.position.x) - f32::from(bounds.origin.x),
                                f32::from(event.position.y) - f32::from(bounds.origin.y),
                            );
                            let hit = hit_world.nodes.iter().rev().find(|node| {
                                let r = node_screen_rect(node, &hit_camera, (0., 0.));
                                local.0 >= r.origin.x
                                    && local.0 <= r.origin.x + r.size.width
                                    && local.1 >= r.origin.y
                                    && local.1 <= r.origin.y + r.size.height
                            });
                            if let Some(node) = hit {
                                on_select_node(node.id.as_str(), window, cx);
                                return;
                            }
                        }
                        on_background_down((event.clone(), Bounds::default()), window, cx);
                    }
                })
                .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                    on_canvas_mouse_move((event.position, event.pressed_button), window, cx)
                })
                .on_scroll_wheel({
                    let on_zoom = on_zoom.clone();
                    let seen_bounds = canvas_bounds.clone();
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
                        // Anchor zooms at the cursor, canvas-local.
                        let anchor = seen_bounds.get().map_or(point(px(0.), px(0.)), |b| {
                            point(event.position.x - b.origin.x, event.position.y - b.origin.y)
                        });
                        match crate::ui::scroll_policy::scroll_action(dx, dy, continuous, control) {
                            crate::ui::scroll_policy::ScrollAction::Pan(dx, dy) => {
                                on_pan((dx, dy), window, cx);
                            }
                            crate::ui::scroll_policy::ScrollAction::Zoom(factor) => {
                                on_zoom((factor, anchor), window, cx);
                            }
                        }
                    }
                })
                .on_pinch({
                    let on_zoom = on_zoom.clone();
                    let seen_bounds = canvas_bounds.clone();
                    move |event: &PinchEvent, window, cx| {
                        // Fractional zoom deltas; 0.1 == +10%, multiplicative.
                        let factor = crate::ui::scroll_policy::pinch_zoom_factor(event.delta);
                        let anchor = seen_bounds.get().map_or(point(px(0.), px(0.)), |b| {
                            point(event.position.x - b.origin.x, event.position.y - b.origin.y)
                        });
                        on_zoom((factor, anchor), window, cx);
                    }
                }),
        )
}
