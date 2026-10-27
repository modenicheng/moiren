//! Graph painting and input consume the same prepaint frame and viewport clip.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use super::graph_canvas::{
    Camera, CanvasFrame, CanvasNode, CanvasWorld, HitTarget, PortDirection, PortTarget, WireCurve,
};
use crate::ui::theme;
use crate::ui::window::TestObserve;

mod nodes;

type WindowAppFn<Argument> = Box<dyn Fn(Argument, &mut Window, &mut App)>;
type WindowAppCallback<Argument> = dyn Fn(Argument, &mut Window, &mut App);
type CameraFitFn = dyn Fn(&Camera, Size<Pixels>, &mut Window, &mut App);
type SelectFn = dyn Fn(&str, &mut Window, &mut App);
type ZoomFn = dyn Fn((f32, Point<Pixels>), &mut Window, &mut App);
type BoxSelectFn = dyn Fn(Option<Bounds<f32>>, &mut Window, &mut App);
type PreparedFrame = Rc<RefCell<Option<Rc<CanvasFrame>>>>;

pub struct CanvasCallbacks {
    pub focus: FocusHandle,
    pub connection: Option<(PortTarget, (f32, f32))>,
    /// Active rubber-band selection in window coordinates (anchor, current).
    pub selection_box: Option<((f32, f32), (f32, f32))>,
    pub on_port_down: Rc<WindowAppCallback<(PortTarget, (f32, f32))>>,
    pub on_port_up: Rc<WindowAppCallback<Option<PortTarget>>>,
    pub on_connection_move: Rc<WindowAppCallback<(f32, f32)>>,
    pub on_delete: Rc<WindowAppCallback<()>>,
    pub on_cancel: Rc<WindowAppCallback<()>>,
    pub on_drop_node: Rc<WindowAppCallback<(super::node_catalog::NodeTemplate, (f32, f32))>>,
    pub on_background_down: Rc<WindowAppCallback<(MouseDownEvent, Bounds<Pixels>)>>,
    pub on_canvas_mouse_move: WindowAppFn<(Point<Pixels>, Option<MouseButton>)>,
    /// Receives the world-space rectangle of the active rubber band once per
    /// frame (None when no band is active), so the shell can live-select.
    pub on_box_select: Rc<BoxSelectFn>,
    pub on_zoom: Rc<ZoomFn>,
    pub on_pan: WindowAppFn<(f32, f32)>,
    pub on_select_node: Rc<SelectFn>,
    pub on_select_wire: Rc<SelectFn>,
    pub on_camera_fit: Rc<CameraFitFn>,
}

fn paint_grid(frame: &CanvasFrame, window: &mut Window) {
    let zoom = frame.camera.zoom;
    let grid = theme::CANVAS_GRID * (1. / zoom).log2().ceil().exp2().max(1.);
    let step = grid * zoom;
    let viewport = frame.viewport();
    let mut left = frame.camera.pan_x.rem_euclid(step);
    while left < viewport.size.width {
        window.paint_quad(fill(
            frame.window_rect(Bounds {
                origin: point(left, 0.),
                size: size(1., viewport.size.height),
            }),
            theme::GRID_LINE,
        ));
        left += step;
    }
    let mut top = frame.camera.pan_y.rem_euclid(step);
    while top < viewport.size.height {
        window.paint_quad(fill(
            frame.window_rect(Bounds {
                origin: point(0., top),
                size: size(viewport.size.width, 1.),
            }),
            theme::GRID_LINE,
        ));
        top += step;
    }
}

fn paint_canvas_layer(
    frame: &CanvasFrame,
    selected_nodes: &[String],
    selected_wire: Option<&str>,
    selection_box: Option<((f32, f32), (f32, f32))>,
    paint_nodes: bool,
    window: &mut Window,
    cx: &mut App,
) {
    window.with_content_mask(
        Some(ContentMask {
            bounds: frame.bounds,
        }),
        |window| {
            paint_grid(frame, window);
            let origin = (
                f32::from(frame.bounds.origin.x),
                f32::from(frame.bounds.origin.y),
            );
            for (wire, geometry) in frame.world.wires.iter().zip(&frame.wires) {
                let selected = selected_wire == Some(wire.id.as_str());
                let connected = selected_nodes
                    .iter()
                    .any(|node| node == &wire.source || node == &wire.destination);
                let width = ((if selected {
                    2.8
                } else if connected {
                    2.
                } else {
                    1.6
                }) * frame.camera.zoom)
                    .max(if selected { 1.8 } else { 0.75 });
                if selected && let Ok(halo) = geometry.stroke(width + 5., origin) {
                    window.paint_path(halo, theme::PRIMARY.opacity(0.28));
                }
                if let Ok(path) = geometry.stroke(width, origin) {
                    window.paint_path(
                        path,
                        if selected {
                            theme::TEXT
                        } else {
                            wire.tint.unwrap_or(theme::PRIMARY)
                        },
                    );
                }
            }
            for (node, layout) in frame.world.nodes.iter().zip(&frame.nodes) {
                if paint_nodes && layout.card.intersects(&frame.viewport()) {
                    nodes::paint_node(
                        frame,
                        node,
                        layout,
                        selected_nodes.iter().any(|selected| selected == &node.id),
                        window,
                        cx,
                    );
                }
            }
            if let Some((anchor, current)) = selection_box {
                let a = frame.local_position(anchor);
                let b = frame.local_position(current);
                let left = a.0.min(b.0);
                let top = a.1.min(b.1);
                let rect = frame.window_rect(Bounds {
                    origin: point(left, top),
                    size: size((a.0 - b.0).abs(), (a.1 - b.1).abs()),
                });
                window.paint_quad(fill(rect, theme::PRIMARY.opacity(0.10)));
                window.paint_quad(PaintQuad {
                    bounds: rect,
                    corner_radii: 2f32.into(),
                    background: transparent_black().into(),
                    border_widths: 1f32.into(),
                    border_color: theme::PRIMARY.opacity(0.65),
                    border_style: BorderStyle::default(),
                });
            }
        },
    );
}

/// Port dots straddle the card edge by their radius; pad the ghost so they
/// stay inside the element.
const PREVIEW_PORT_PAD: f32 = 4.;

/// Drag ghost for a library template: the exact default card the drop will
/// create, painted by the shared node painter instead of a bare placeholder.
pub(in crate::ui) fn node_drag_preview(
    template: super::node_catalog::NodeTemplate,
) -> impl IntoElement {
    use super::node_catalog::NodeTemplate;
    let (icon, tint) = match template {
        NodeTemplate::Input => ("figma-input", theme::NODE_SOURCE),
        NodeTemplate::Application => ("app", theme::NODE_SOURCE),
        NodeTemplate::Tone => ("wave", theme::NODE_SOURCE),
        NodeTemplate::Output => ("figma-output", theme::NODE_SINK),
        NodeTemplate::Bus => ("figma-bus", theme::NODE_UTILITY),
        NodeTemplate::Recorder => ("radio", theme::NODE_ANALYSIS),
        NodeTemplate::Meter => ("activity", theme::NODE_ANALYSIS),
        NodeTemplate::Spectrum => ("wave", theme::NODE_ANALYSIS),
        NodeTemplate::Delay => ("clock", theme::NODE_DSP),
        NodeTemplate::ChannelMapper | NodeTemplate::Rack => ("route", theme::NODE_UTILITY),
        _ => ("figma-fader", theme::NODE_DSP),
    };
    // A fresh card's port shape: sources only output, the sink only inputs.
    let (input_count, has_output) = match template {
        NodeTemplate::Input | NodeTemplate::Application | NodeTemplate::Tone => (0, true),
        NodeTemplate::Output => (1, false),
        _ => (1, true),
    };
    let node = CanvasNode {
        node_id: None,
        id: "drag-preview".into(),
        name: t!(template.title()).to_string(),
        detail: match template {
            NodeTemplate::Input => t!("Choose a source").to_string(),
            NodeTemplate::Output => t!("Choose an output").to_string(),
            _ => "L/R".into(),
        },
        icon: Some(icon),
        x: PREVIEW_PORT_PAD,
        y: PREVIEW_PORT_PAD,
        wide: false,
        input_count,
        input_ports: Vec::new(),
        output_port: None,
        has_output,
        tint: Some(tint),
        gain: 1.,
        balance: None,
        peaks: None,
        kind: template
            .kind()
            .unwrap_or(moiren_core::graph::NodeKind::Source),
        content_width: 270.,
        measured_height: Some(112.),
    };
    let size = (
        node.width() + PREVIEW_PORT_PAD * 2.,
        node.height() + PREVIEW_PORT_PAD * 2.,
    );
    let world = Arc::new(CanvasWorld::new(vec![node], Vec::new()));
    canvas(
        move |bounds, _, _| {
            CanvasFrame::new(
                world,
                Camera {
                    adjusted: true,
                    ..Default::default()
                },
                bounds,
            )
        },
        |_, frame, window, cx| {
            nodes::paint_node(
                &frame,
                &frame.world.nodes[0],
                &frame.nodes[0],
                true,
                window,
                cx,
            );
        },
    )
    .w(px(size.0))
    .h(px(size.1))
}

fn overview(prepared: PreparedFrame) -> impl IntoElement {
    div()
        .absolute()
        .bottom(px(12.))
        .right(px(12.))
        .w(px(164.))
        .p(px(8.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .overflow_hidden()
        .child(
            div()
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_MUTED)
                .child(t!("Overview")),
        )
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let prepared = prepared.borrow();
                    let Some(frame) = prepared.as_ref() else {
                        return;
                    };
                    let overview = frame.overview(size(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    ));
                    let project = |rect: Bounds<f32>| Bounds {
                        origin: bounds.origin + point(px(rect.origin.x), px(rect.origin.y)),
                        size: size(px(rect.size.width), px(rect.size.height)),
                    };
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for node in &frame.world.nodes {
                            let layout = super::graph_canvas::NodeLayout::new(node);
                            window.paint_quad(fill(
                                project(overview.camera.project_rect(layout.card)),
                                theme::soft(node.tint()),
                            ));
                        }
                        window.paint_quad(PaintQuad {
                            bounds: project(overview.viewport),
                            corner_radii: 0f32.into(),
                            background: transparent_black().into(),
                            border_widths: 1f32.into(),
                            border_color: theme::TEXT_MUTED,
                            border_style: BorderStyle::default(),
                        });
                    });
                },
            )
            .w_full()
            .h(px(64.)),
        )
}

pub fn graph_canvas_view(
    world: Arc<CanvasWorld>,
    camera: Camera,
    selected_nodes: &[String],
    selected_wire: Option<&str>,
    callbacks: CanvasCallbacks,
    node_controls: Vec<(String, AnyElement)>,
) -> impl IntoElement {
    let CanvasCallbacks {
        on_drop_node,
        on_background_down,
        on_canvas_mouse_move,
        on_box_select,
        on_zoom,
        on_pan,
        on_select_node,
        on_select_wire,
        on_camera_fit,
        focus,
        connection,
        selection_box,
        on_port_down,
        on_port_up,
        on_connection_move,
        on_delete,
        on_cancel,
    } = callbacks;
    let prepared: PreparedFrame = Rc::new(RefCell::new(None));
    let selected_nodes = selected_nodes.to_vec();
    let selected_wire = selected_wire.map(str::to_owned);
    // The left handler moves `focus`; keep one clone for the right handler,
    // which must also focus the canvas so right-button drags keep keys alive.
    let right_focus = focus.clone();
    let input_move: Rc<WindowAppCallback<(Point<Pixels>, Option<MouseButton>)>> =
        Rc::from(on_canvas_mouse_move);
    let on_pan: Rc<WindowAppCallback<(f32, f32)>> = Rc::from(on_pan);
    let separate_nodes = !node_controls.is_empty();
    let mut controls: std::collections::HashMap<_, _> = node_controls.into_iter().collect();
    let mut node_layers = Vec::new();
    if separate_nodes {
        for (index, node) in world.nodes.iter().enumerate() {
            let painted = prepared.clone();
            let selected = selected_nodes.iter().any(|id| id == &node.id);
            node_layers.push(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        if let Some(frame) = painted.borrow().as_ref() {
                            let layout = &frame.nodes[index];
                            if layout.card.intersects(&frame.viewport()) {
                                nodes::paint_node(
                                    frame,
                                    &frame.world.nodes[index],
                                    layout,
                                    selected,
                                    window,
                                    cx,
                                );
                            }
                        }
                    },
                )
                .absolute()
                .size_full()
                .into_any_element(),
            );
            let card = camera.project_rect(super::graph_canvas::NodeLayout::new(node).card);
            let hit_frame = prepared.clone();
            let card_focus = focus.clone();
            let select = on_select_node.clone();
            let port_down = on_port_down.clone();
            let pan_down = on_background_down.clone();
            let right_pan_down = on_background_down.clone();
            let middle_frame = prepared.clone();
            let right_frame = prepared.clone();
            let wheel_frame = prepared.clone();
            let zoom = on_zoom.clone();
            let pan = on_pan.clone();
            let pinch_frame = prepared.clone();
            let pinch_zoom = on_zoom.clone();
            let release_move = input_move.clone();
            let release_port = on_port_up.clone();
            let move_card = input_move.clone();
            // Occlusion also shields ancestor hitboxes in GPUI. Forward header
            // gestures explicitly through the same prepared-frame callbacks.
            node_layers.push(
                div()
                    .absolute()
                    .left(px(card.left()))
                    .top(px(card.top()))
                    .w(px(card.size.width))
                    .h(px(card.size.height))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        window.focus(&card_focus, cx);
                        if let Some(frame) = hit_frame.borrow().as_ref() {
                            match frame.hit_test((
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                            )) {
                                Some(HitTarget::Port(port)) => port_down(
                                    (
                                        port,
                                        frame.camera.canvas_to_world(frame.local_position((
                                            f32::from(event.position.x),
                                            f32::from(event.position.y),
                                        ))),
                                    ),
                                    window,
                                    cx,
                                ),
                                Some(HitTarget::Node(id)) => select(id, window, cx),
                                _ => {}
                            }
                        }
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Middle, move |event, window, cx| {
                        if let Some(frame) = middle_frame.borrow().as_ref() {
                            pan_down((event.clone(), frame.bounds), window, cx);
                        }
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        if let Some(frame) = right_frame.borrow().as_ref() {
                            right_pan_down((event.clone(), frame.bounds), window, cx);
                        }
                        cx.stop_propagation();
                    })
                    .on_scroll_wheel(move |event, window, cx| {
                        if let Some(frame) = wheel_frame.borrow().as_ref() {
                            scroll_canvas(event, frame, &zoom, &pan, window, cx);
                        }
                        cx.stop_propagation();
                    })
                    .on_pinch(move |event, window, cx| {
                        if let Some(frame) = pinch_frame.borrow().as_ref() {
                            let anchor = frame.local_position((
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                            ));
                            pinch_zoom(
                                (
                                    crate::ui::scroll_policy::pinch_zoom_factor(event.delta),
                                    point(px(anchor.0), px(anchor.1)),
                                ),
                                window,
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    })
                    .on_mouse_move(move |event, window, cx| {
                        move_card((event.position, event.pressed_button), window, cx);
                    })
                    .on_mouse_up(MouseButton::Left, move |event, window, cx| {
                        release_port(None, window, cx);
                        release_move((event.position, None), window, cx);
                        cx.stop_propagation();
                    })
                    .into_any_element(),
            );
            if let Some(body) = controls.remove(&node.id) {
                node_layers.push(body);
            }
        }
    }
    div()
        .id("graph-canvas")
        .relative()
        .size_full()
        .bg(theme::WINDOW_BG)
        .overflow_hidden()
        // The entire graph is a library drop surface, including native card
        // children layered above the canvas input hitbox.
        .on_drop({
            let prepared = prepared.clone();
            move |template: &super::node_catalog::NodeTemplate, window, cx| {
                if let Some(frame) = prepared.borrow().as_ref() {
                    let position = window.mouse_position();
                    let local =
                        frame.local_position((f32::from(position.x), f32::from(position.y)));
                    on_drop_node((*template, frame.camera.canvas_to_world(local)), window, cx);
                }
            }
        })
        .child(
            canvas(
                {
                    let prepared = prepared.clone();
                    move |bounds, window, cx| {
                        let frame = Rc::new(CanvasFrame::new(world.clone(), camera, bounds));
                        *prepared.borrow_mut() = Some(frame.clone());
                        on_camera_fit(&frame.camera, bounds.size, window, cx);
                        if let Some((anchor, current)) = selection_box {
                            let a = frame.local_position(anchor);
                            let b = frame.local_position(current);
                            let min = frame.camera.canvas_to_world((a.0.min(b.0), a.1.min(b.1)));
                            let max = frame.camera.canvas_to_world((a.0.max(b.0), a.1.max(b.1)));
                            on_box_select(
                                Some(Bounds {
                                    origin: point(min.0, min.1),
                                    size: size(max.0 - min.0, max.1 - min.1),
                                }),
                                window,
                                cx,
                            );
                        } else {
                            on_box_select(None, window, cx);
                        }
                        frame
                    }
                },
                move |_, frame, window, cx| {
                    paint_canvas_layer(
                        &frame,
                        &selected_nodes,
                        selected_wire.as_deref(),
                        selection_box,
                        !separate_nodes,
                        window,
                        cx,
                    );
                    if let Some((port, cursor)) = connection
                        && let Some(anchor) = frame.port_position(port)
                    {
                        let cursor = frame.camera.world_to_canvas(cursor);
                        let (from, to) = match port.direction {
                            PortDirection::Output => (anchor, cursor),
                            PortDirection::Input => (cursor, anchor),
                        };
                        let origin = (
                            f32::from(frame.bounds.origin.x),
                            f32::from(frame.bounds.origin.y),
                        );
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds: frame.bounds,
                            }),
                            |window| {
                                if let Ok(path) = WireCurve::new(from, to)
                                    .project(Camera {
                                        zoom: 1.,
                                        adjusted: true,
                                        ..Default::default()
                                    })
                                    .stroke(2., origin)
                                {
                                    window.paint_path(path, theme::PRIMARY);
                                }
                            },
                        );
                    }
                },
            )
            .size_full(),
        )
        .child(
            div()
                .id("graph-canvas-input")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .track_focus(&focus)
                .on_key_down(move |event, window, cx| {
                    match event.keystroke.key.as_str() {
                        "backspace" | "delete" => on_delete((), window, cx),
                        "escape" => on_cancel((), window, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                })
                .on_mouse_down(MouseButton::Left, {
                    let prepared = prepared.clone();
                    let on_background_down = on_background_down.clone();
                    move |event, window, cx| {
                        window.focus(&focus, cx);
                        let frame = prepared.borrow().clone();
                        let Some(frame) = frame else {
                            return;
                        };
                        match frame
                            .hit_test((f32::from(event.position.x), f32::from(event.position.y)))
                        {
                            Some(HitTarget::Port(port)) => on_port_down(
                                (
                                    port,
                                    frame.camera.canvas_to_world(frame.local_position((
                                        f32::from(event.position.x),
                                        f32::from(event.position.y),
                                    ))),
                                ),
                                window,
                                cx,
                            ),
                            Some(HitTarget::Node(id)) => on_select_node(id, window, cx),
                            Some(HitTarget::Wire(id)) => on_select_wire(id, window, cx),
                            None => on_background_down((event.clone(), frame.bounds), window, cx),
                        }
                    }
                })
                .on_mouse_down(MouseButton::Right, {
                    let prepared = prepared.clone();
                    let on_background_down = on_background_down.clone();
                    move |event, window, cx| {
                        window.focus(&right_focus, cx);
                        if let Some(frame) = prepared.borrow().as_ref() {
                            on_background_down((event.clone(), frame.bounds), window, cx);
                        }
                    }
                })
                .on_mouse_down(MouseButton::Middle, {
                    let prepared = prepared.clone();
                    move |event, window, cx| {
                        if let Some(frame) = prepared.borrow().as_ref() {
                            on_background_down((event.clone(), frame.bounds), window, cx);
                        }
                    }
                })
                .on_mouse_move({
                    let input_move = input_move.clone();
                    let prepared = prepared.clone();
                    move |event, window, cx| {
                        if let Some(frame) = prepared.borrow().as_ref() {
                            on_connection_move(
                                frame.camera.canvas_to_world(frame.local_position((
                                    f32::from(event.position.x),
                                    f32::from(event.position.y),
                                ))),
                                window,
                                cx,
                            );
                        }
                        input_move((event.position, event.pressed_button), window, cx)
                    }
                })
                .on_mouse_up(MouseButton::Left, {
                    let input_move = input_move.clone();
                    let prepared = prepared.clone();
                    let on_port_up = on_port_up.clone();
                    move |event, window, cx| {
                        let port = prepared.borrow().as_ref().and_then(|frame| {
                            match frame.hit_test((
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                            )) {
                                Some(HitTarget::Port(port)) => Some(port),
                                _ => None,
                            }
                        });
                        on_port_up(port, window, cx);
                        input_move((event.position, None), window, cx);
                    }
                })
                .on_mouse_up_out(MouseButton::Left, {
                    let input_move = input_move.clone();
                    move |event, window, cx| {
                        on_port_up(None, window, cx);
                        input_move((event.position, None), window, cx);
                    }
                })
                .on_mouse_up(MouseButton::Right, {
                    let input_move = input_move.clone();
                    move |event, window, cx| input_move((event.position, None), window, cx)
                })
                .on_mouse_up_out(MouseButton::Right, {
                    let input_move = input_move.clone();
                    move |event, window, cx| input_move((event.position, None), window, cx)
                })
                .on_mouse_up(MouseButton::Middle, {
                    let input_move = input_move.clone();
                    move |event, window, cx| input_move((event.position, None), window, cx)
                })
                .on_mouse_up_out(MouseButton::Middle, move |event, window, cx| {
                    input_move((event.position, None), window, cx)
                })
                .on_scroll_wheel({
                    let prepared = prepared.clone();
                    let on_zoom = on_zoom.clone();
                    move |event: &ScrollWheelEvent, window, cx| {
                        let frame = prepared.borrow().clone();
                        let Some(frame) = frame else {
                            return;
                        };
                        scroll_canvas(event, &frame, &on_zoom, &on_pan, window, cx);
                    }
                })
                .on_pinch({
                    let prepared = prepared.clone();
                    move |event: &PinchEvent, window, cx| {
                        let frame = prepared.borrow().clone();
                        if let Some(frame) = frame {
                            let factor = crate::ui::scroll_policy::pinch_zoom_factor(event.delta);
                            let anchor = frame.local_position((
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                            ));
                            on_zoom((factor, point(px(anchor.0), px(anchor.1))), window, cx);
                        }
                    }
                })
                .children(node_layers)
                .gpui_test_observe(),
        )
        .child(overview(prepared))
}

fn scroll_canvas(
    event: &ScrollWheelEvent,
    frame: &CanvasFrame,
    zoom: &Rc<ZoomFn>,
    pan: &Rc<WindowAppCallback<(f32, f32)>>,
    window: &mut Window,
    cx: &mut App,
) {
    #[cfg(windows)]
    let control = event.modifiers.control || crate::ui::touchpad::wheel_control();
    #[cfg(not(windows))]
    let control = event.modifiers.control;
    let (dx, dy, continuous) = match event.delta {
        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y), true),
        ScrollDelta::Lines(lines) => (lines.x * 60., lines.y * 60., false),
    };
    let anchor = frame.local_position((f32::from(event.position.x), f32::from(event.position.y)));
    match crate::ui::scroll_policy::scroll_action(dx, dy, continuous, control) {
        crate::ui::scroll_policy::ScrollAction::Pan(dx, dy) => pan((dx, dy), window, cx),
        crate::ui::scroll_policy::ScrollAction::Zoom(factor) => {
            zoom((factor, point(px(anchor.0), px(anchor.1))), window, cx)
        }
    }
}
