//! Graph painting and input consume the same prepaint frame and viewport clip.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use super::graph_canvas::{
    Camera, CanvasFrame, CanvasNode, CanvasWorld, ConnectionCandidate, HitTarget, KnobControl,
    OverlayRects, PortDirection, PortTarget, WireCurve,
};
use crate::ui::monitoring::{DisplayLevel, SpectrumCurve};
use crate::ui::observe::TestObserve;
use crate::ui::shared_frame::SharedFrame;
use crate::ui::theme;

mod nodes;

// The callback aliases ride `pub(in crate::ui)` so the window-side page
// builders can name them when they construct [`CanvasCallbacks`] fields.
pub(in crate::ui) type WindowAppFn<Argument> = Box<dyn Fn(Argument, &mut Window, &mut App)>;
pub(in crate::ui) type WindowAppCallback<Argument> = dyn Fn(Argument, &mut Window, &mut App);
pub(in crate::ui) type CameraFitFn = dyn Fn(&Camera, Size<Pixels>, &mut Window, &mut App);
pub(in crate::ui) type SelectFn = dyn Fn(&str, &mut Window, &mut App);
pub(in crate::ui) type ZoomFn = dyn Fn((f32, Point<Pixels>), &mut Window, &mut App);

/// One node's live meter reading: display ballistics plus the latched clip
/// state, read at paint time so meter motion repaints without re-projecting
/// the graph.
#[derive(Debug, Clone, Copy)]
pub(in crate::ui) struct NodeMeterReading {
    pub levels: [DisplayLevel; 2],
    pub clips: [bool; 2],
}

pub(in crate::ui) type NodeLevelsFn =
    dyn Fn(moiren_core::graph::NodeId, &App) -> Option<NodeMeterReading>;
/// One analyzer's curve for the cached canvas: an owned copy taken at
/// rebuild time, so the paint closure needs no borrow into the shell's
/// subscription table (the hot layer reads that table directly).
pub(in crate::ui) struct NodeSpectrumReading {
    pub points: Box<[f32]>,
    pub lo_hz: f64,
    pub hi_hz: f64,
}

impl NodeSpectrumReading {
    pub fn curve(&self) -> SpectrumCurve<'_> {
        SpectrumCurve {
            points: &self.points,
            lo_hz: self.lo_hz,
            hi_hz: self.hi_hz,
        }
    }
}

pub(in crate::ui) type NodeSpectrumFn =
    dyn Fn(moiren_core::graph::NodeId, &App) -> Option<NodeSpectrumReading>;

/// What a card's live surfaces show in one paint: the meter reading and
/// the analyzer curve, both read at paint time, neither part of the
/// projection.
#[derive(Clone, Copy, Default)]
pub(in crate::ui) struct NodeLive<'a> {
    pub meters: Option<&'a NodeMeterReading>,
    pub spectrum: Option<SpectrumCurve<'a>>,
}

/// The shell's readers for those surfaces, consulted per visible card.
#[derive(Clone, Copy)]
struct LiveReaders<'a> {
    levels: &'a NodeLevelsFn,
    spectrum: &'a NodeSpectrumFn,
}
/// The frame the canvas prepaint projected most recently. Shell-owned so the
/// page's hot layer can repaint bars over the cached canvas between cache
/// rebuilds, exactly like the canvas painted them.
pub(in crate::ui) type PreparedFrame = SharedFrame<CanvasFrame>;

pub(in crate::ui) use nodes::{
    paint_fault_banner, paint_fault_tooltip, paint_meter_bars, paint_spectrum_body,
};

pub struct CanvasCallbacks {
    pub focus: FocusHandle,
    /// The connection drag in flight: the port it pulls from, the cursor in
    /// world units, and — when the drag picked an edge up off its input —
    /// that input, which the caller has already dropped from the world so
    /// the head paints free.
    pub connection: Option<(PortTarget, (f32, f32), Option<moiren_core::graph::PortId>)>,
    /// Live ballistics-smoothed bar readings per node, consulted at paint
    /// time so meter motion repaints without rebuilding this element tree.
    pub node_levels: Rc<NodeLevelsFn>,
    /// Live spectrum curves per analyzer node, for the same reason.
    pub node_spectrum: Rc<NodeSpectrumFn>,
    pub on_port_down: Rc<WindowAppCallback<(PortTarget, (f32, f32))>>,
    pub on_port_up: Rc<WindowAppCallback<Option<ConnectionCandidate>>>,
    /// A knob dial press: canvas node id, control, window y. Starts a
    /// value drag (or a double-click reset).
    pub on_knob_down: Rc<WindowAppCallback<(String, KnobControl, f32)>>,
    /// Wheel notches over a knob dial: positive = up = increase.
    pub on_knob_wheel: Rc<WindowAppCallback<(String, KnobControl, f32)>>,
    /// The expander glyph was pressed; opens the detail window.
    pub on_expand: Rc<WindowAppCallback<String>>,
    /// A binding chip was pressed: canvas node id plus the chip's window
    /// rect, which anchors the endpoint menu.
    pub on_chip: Rc<WindowAppCallback<(String, Bounds<Pixels>)>>,
    /// A faulted card's "!" badge was pressed: canvas node id plus the
    /// badge's window rect, which anchors the fault detail popover.
    pub on_badge: Rc<WindowAppCallback<(String, Bounds<Pixels>)>>,
    pub on_connection_move: Rc<WindowAppCallback<(f32, f32)>>,
    pub on_delete: Rc<WindowAppCallback<()>>,
    pub on_cancel: Rc<WindowAppCallback<()>>,
    pub on_drop_node: Rc<WindowAppCallback<(super::node_catalog::NodeTemplate, (f32, f32))>>,
    pub on_background_down: Rc<WindowAppCallback<(MouseDownEvent, Bounds<Pixels>)>>,
    pub on_canvas_mouse_move: WindowAppFn<(Point<Pixels>, Option<MouseButton>)>,

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

/// The snap alignment guides (item F): world-space vertical/horizontal
/// lines painted above the grid while a card drag is aligned.
fn paint_snap_guides(frame: &CanvasFrame, window: &mut Window) {
    let Some(guides) = frame.world.guides else {
        return;
    };
    let viewport = frame.viewport();
    if let Some(x) = guides.x {
        let left = frame.camera.world_to_canvas((x, 0.)).0;
        if (0. ..=viewport.size.width).contains(&left) {
            window.paint_quad(fill(
                frame.window_rect(Bounds {
                    origin: point(left, 0.),
                    size: size(1., viewport.size.height),
                }),
                theme::PRIMARY,
            ));
        }
    }
    if let Some(y) = guides.y {
        let top = frame.camera.world_to_canvas((0., y)).1;
        if (0. ..=viewport.size.height).contains(&top) {
            window.paint_quad(fill(
                frame.window_rect(Bounds {
                    origin: point(0., top),
                    size: size(viewport.size.width, 1.),
                }),
                theme::PRIMARY,
            ));
        }
    }
}

fn paint_canvas_layer(
    frame: &CanvasFrame,
    selected_nodes: &[String],
    selected_wire: Option<&str>,
    readers: LiveReaders<'_>,
    connection: Option<(PortTarget, (f32, f32))>,
    window: &mut Window,
    cx: &mut App,
) {
    window.with_content_mask(
        Some(ContentMask {
            bounds: frame.bounds,
        }),
        |window| {
            paint_grid(frame, window);
            paint_snap_guides(frame, window);
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
                if layout.card.intersects(&frame.viewport()) {
                    let spectrum = node
                        .spectrum
                        .then(|| node.node_id.and_then(|id| (readers.spectrum)(id, cx)))
                        .flatten();
                    let meters = node.node_id.and_then(|id| (readers.levels)(id, cx));
                    nodes::paint_node(
                        frame,
                        node,
                        layout,
                        selected_nodes.iter().any(|selected| selected == &node.id),
                        NodeLive {
                            meters: meters.as_ref(),
                            spectrum: spectrum.as_ref().map(NodeSpectrumReading::curve),
                        },
                        window,
                        cx,
                    );
                }
            }
            if let Some((port, cursor)) = connection {
                paint_connection_drag(frame, port, cursor, window);
            }
        },
    );
}

/// The connection drag's ghost line and hover verdict: the line snaps to
/// the resolved candidate port when the drop would land (port or body),
/// colors itself by the verdict, and rings the target so the release is
/// never a surprise (design 18 A3).
fn paint_connection_drag(
    frame: &CanvasFrame,
    port: PortTarget,
    cursor: (f32, f32),
    window: &mut Window,
) {
    use super::graph_canvas::CandidateStatus;
    let Some(anchor) = frame.port_position(port) else {
        return;
    };
    let cursor = frame.camera.world_to_canvas(cursor);
    let window_cursor = (
        cursor.0 + f32::from(frame.bounds.origin.x),
        cursor.1 + f32::from(frame.bounds.origin.y),
    );
    let candidate = frame.connection_candidate(window_cursor, port);
    let snap = candidate
        .as_ref()
        .filter(|candidate| {
            matches!(
                candidate.status,
                CandidateStatus::Valid | CandidateStatus::Displacing
            )
        })
        .and_then(|candidate| frame.port_position(candidate.port));
    let (from, to) = match port.direction {
        PortDirection::Output => (anchor, snap.unwrap_or(cursor)),
        PortDirection::Input => (snap.unwrap_or(cursor), anchor),
    };
    let color = match candidate.as_ref().map(|candidate| candidate.status) {
        Some(CandidateStatus::Incompatible | CandidateStatus::Cycles) => theme::DANGER,
        Some(CandidateStatus::Displacing) => theme::WARNING,
        _ => theme::PRIMARY,
    };
    let origin = (
        f32::from(frame.bounds.origin.x),
        f32::from(frame.bounds.origin.y),
    );
    if let Ok(path) = WireCurve::new(from, to)
        .project(Camera {
            zoom: 1.,
            adjusted: true,
            ..Default::default()
        })
        .stroke(2., origin)
    {
        window.paint_path(path, color);
    }
    let Some(candidate) = candidate else {
        return;
    };
    let status_color = match candidate.status {
        CandidateStatus::Valid => theme::PRIMARY,
        CandidateStatus::Displacing => theme::WARNING,
        CandidateStatus::Incompatible | CandidateStatus::Cycles => theme::DANGER,
    };
    if let Some(anchor) = frame.port_position(candidate.port) {
        let radius = 9.;
        window.paint_quad(fill(
            frame.window_rect(Bounds {
                origin: point(anchor.0 - radius, anchor.1 - radius),
                size: size(radius * 2., radius * 2.),
            }),
            theme::soft(status_color),
        ));
    }
    if candidate.from_body
        && let Some((_, layout)) = frame
            .world
            .nodes
            .iter()
            .zip(&frame.nodes)
            .find(|(node, _)| node.node_id == Some(candidate.node))
    {
        let card = frame.window_rect(layout.card);
        window.paint_quad(PaintQuad {
            bounds: card,
            corner_radii: (theme::card::RADIUS * layout.scale).into(),
            background: transparent_black().into(),
            border_widths: theme::card::OUTLINE_W.into(),
            border_color: status_color,
            border_style: BorderStyle::default(),
        });
    }
    // A displacing drop fades the wire it would replace before the
    // release commits it.
    if candidate.status == CandidateStatus::Displacing
        && let Some(index) = frame
            .world
            .wires
            .iter()
            .position(|wire| wire.dst_port == candidate.port.id)
        && let Ok(path) = frame.wires[index].stroke(3., origin)
    {
        window.paint_path(path, theme::WINDOW_BG.opacity(0.55));
    }
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
    // The ghost paints the library's own glyph for the kind, so the card
    // that lands looks exactly like the card that was dragged.
    let icon = template.icon();
    let tint = template.tint();
    // A fresh card's port shape: sources only output, the sink only inputs.
    let (input_count, has_output) = match template {
        NodeTemplate::Input | NodeTemplate::Application | NodeTemplate::Tone => (0, true),
        NodeTemplate::Output => (1, false),
        _ => (1, true),
    };
    // The analyzer's ghost is the analyzer's shape: the wide 4:3 card with
    // its plot furniture, so the drop lands what the drag showed.
    let spectrum = template == NodeTemplate::Spectrum;
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
        input_count,
        input_ports: Vec::new(),
        output_port: None,
        has_output,
        tint: Some(tint),
        faulted: false,
        preparing: false,
        faults: Vec::new(),
        body_height: if spectrum {
            theme::card::SPECTRUM_BODY_H
        } else {
            theme::card::BODY_H
        },
        meter_vertical: spectrum,
        channels: 2,
        knobs: Vec::new(),
        binding: None,
        matrix: None,
        rack_stages: Vec::new(),
        curve: None,
        spectrum,
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
                false,
                NodeLive::default(),
                window,
                cx,
            );
        },
    )
    .w(px(size.0))
    .h(px(size.1))
}

fn overview(prepared: PreparedFrame, overlays: &OverlayRects) -> impl IntoElement {
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
        .child(overlays.probe("overview"))
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
                    let Some(frame) = prepared.read() else {
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
    prepared: PreparedFrame,
    overlays: &OverlayRects,
) -> impl IntoElement {
    let CanvasCallbacks {
        on_drop_node,
        on_background_down,
        on_canvas_mouse_move,
        on_zoom,
        on_pan,
        on_select_node,
        on_select_wire,
        on_camera_fit,
        node_levels,
        node_spectrum,
        focus,
        connection,
        on_port_down,
        on_port_up,
        on_knob_down,
        on_knob_wheel,
        on_expand,
        on_chip,
        on_badge,
        on_connection_move,
        on_delete,
        on_cancel,
    } = callbacks;
    let selected_nodes = selected_nodes.to_vec();
    let selected_wire = selected_wire.map(str::to_owned);
    // The painter and the release path only need the head and the port it
    // pulls from; the picked-up input already shaped the world.
    let connection = connection.map(|(port, cursor, _)| (port, cursor));
    // The left handler moves `focus`; keep one clone for the right handler,
    // which must also focus the canvas so right-button drags keep keys alive.
    let right_focus = focus.clone();
    let input_move: Rc<WindowAppCallback<(Point<Pixels>, Option<MouseButton>)>> =
        Rc::from(on_canvas_mouse_move);
    div()
        .id("graph-canvas")
        .relative()
        .size_full()
        .bg(theme::WINDOW_BG)
        .overflow_hidden()
        .child(
            canvas(
                {
                    let prepared = prepared.clone();
                    move |bounds, window, cx| {
                        let frame = Rc::new(CanvasFrame::new(world.clone(), camera, bounds));
                        prepared.publish(Rc::clone(&frame));
                        on_camera_fit(&frame.camera, bounds.size, window, cx);
                        frame
                    }
                },
                move |_, frame, window, cx| {
                    paint_canvas_layer(
                        &frame,
                        &selected_nodes,
                        selected_wire.as_deref(),
                        LiveReaders {
                            levels: node_levels.as_ref(),
                            spectrum: node_spectrum.as_ref(),
                        },
                        connection,
                        window,
                        cx,
                    );
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
                .on_drop({
                    let prepared = prepared.clone();
                    move |template: &super::node_catalog::NodeTemplate, window, cx| {
                        if let Some(frame) = prepared.read() {
                            let position = window.mouse_position();
                            let local = frame
                                .local_position((f32::from(position.x), f32::from(position.y)));
                            on_drop_node(
                                (*template, frame.camera.canvas_to_world(local)),
                                window,
                                cx,
                            );
                        }
                    }
                })
                .on_mouse_down(MouseButton::Left, {
                    let prepared = prepared.clone();
                    let on_background_down = on_background_down.clone();
                    let on_knob_down = on_knob_down.clone();
                    let on_expand = on_expand.clone();
                    let on_chip = on_chip.clone();
                    let on_badge = on_badge.clone();
                    move |event, window, cx| {
                        window.focus(&focus, cx);
                        let Some(frame) = prepared.read() else {
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
                            Some(HitTarget::Knob { node, control }) => on_knob_down(
                                (node.to_string(), control, f32::from(event.position.y)),
                                window,
                                cx,
                            ),
                            Some(HitTarget::Expand { node }) => {
                                on_expand(node.to_string(), window, cx)
                            }
                            Some(HitTarget::Chip { node }) => {
                                let anchor = frame
                                    .world
                                    .nodes
                                    .iter()
                                    .position(|candidate| candidate.id == node)
                                    .and_then(|index| frame.nodes[index].chip.as_ref())
                                    .map(|chip| frame.window_rect(chip.bounds));
                                if let Some(anchor) = anchor {
                                    on_chip((node.to_string(), anchor), window, cx)
                                }
                            }
                            Some(HitTarget::Badge { node }) => {
                                let anchor = frame
                                    .world
                                    .nodes
                                    .iter()
                                    .position(|candidate| candidate.id == node)
                                    .map(|index| frame.window_rect(frame.nodes[index].badge));
                                if let Some(anchor) = anchor {
                                    on_badge((node.to_string(), anchor), window, cx)
                                }
                            }
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
                        if let Some(frame) = prepared.read() {
                            on_background_down((event.clone(), frame.bounds), window, cx);
                        }
                    }
                })
                .on_mouse_down(MouseButton::Middle, {
                    let prepared = prepared.clone();
                    move |event, window, cx| {
                        if let Some(frame) = prepared.read() {
                            on_background_down((event.clone(), frame.bounds), window, cx);
                        }
                    }
                })
                .on_mouse_move({
                    let input_move = input_move.clone();
                    let prepared = prepared.clone();
                    move |event, window, cx| {
                        if let Some(frame) = prepared.read() {
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
                    let drag_port = connection.map(|(port, _)| port);
                    move |event, window, cx| {
                        let candidate = prepared.read().and_then(|frame| {
                            drag_port.and_then(|drag| {
                                frame.connection_candidate(
                                    (f32::from(event.position.x), f32::from(event.position.y)),
                                    drag,
                                )
                            })
                        });
                        on_port_up(candidate, window, cx);
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
                    let on_knob_wheel = on_knob_wheel.clone();
                    move |event: &ScrollWheelEvent, window, cx| {
                        let Some(frame) = prepared.read() else {
                            return;
                        };
                        // A knob under the cursor consumes the wheel before
                        // the canvas zoom/pan policy sees it (design 18 B3);
                        // positive notches are wheel-up = increase.
                        if let Some(HitTarget::Knob { node, control }) = frame
                            .hit_test((f32::from(event.position.x), f32::from(event.position.y)))
                        {
                            let notches = match event.delta {
                                ScrollDelta::Lines(lines) => lines.y,
                                ScrollDelta::Pixels(delta) => f32::from(delta.y) / 60.,
                            };
                            on_knob_wheel((node.to_string(), control, notches), window, cx);
                            cx.stop_propagation();
                            return;
                        }
                        #[cfg(windows)]
                        let control =
                            event.modifiers.control || crate::ui::touchpad::wheel_control();
                        #[cfg(not(windows))]
                        let control = event.modifiers.control;
                        let (delta_x, delta_y, continuous) = match event.delta {
                            ScrollDelta::Pixels(delta) => {
                                (f32::from(delta.x), f32::from(delta.y), true)
                            }
                            ScrollDelta::Lines(lines) => (lines.x * 60., lines.y * 60., false),
                        };
                        let anchor = frame.local_position((
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                        ));
                        match crate::ui::scroll_policy::scroll_action(
                            delta_x, delta_y, continuous, control,
                        ) {
                            crate::ui::scroll_policy::ScrollAction::Pan(delta_x, delta_y) => {
                                on_pan((delta_x, delta_y), window, cx)
                            }
                            crate::ui::scroll_policy::ScrollAction::Zoom(factor) => {
                                on_zoom((factor, point(px(anchor.0), px(anchor.1))), window, cx)
                            }
                        }
                    }
                })
                .on_pinch({
                    let prepared = prepared.clone();
                    move |event: &PinchEvent, window, cx| {
                        if let Some(frame) = prepared.read() {
                            let factor = crate::ui::scroll_policy::pinch_zoom_factor(event.delta);
                            let anchor = frame.local_position((
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                            ));
                            on_zoom((factor, point(px(anchor.0), px(anchor.1))), window, cx);
                        }
                    }
                })
                .gpui_test_observe(),
        )
        .child(overview(prepared, overlays))
}
