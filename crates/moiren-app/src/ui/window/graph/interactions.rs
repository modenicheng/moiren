//! The canvas interaction callbacks, one builder per [`CanvasCallbacks`]
//! field: each closure is a verbatim move out of the former `page()` body,
//! so the page stays a projection plus an assembly and every gesture keeps
//! its own reviewable unit. Builders take the shell entity and (where the
//! gesture resolves canvas ids back to `NodeId`s) the projected world.

use super::*;
use crate::ui::pages::node_catalog::NodeTemplate;
use pages::graph_canvas::{CanvasFrame, CanvasWorld};
use pages::graph_canvas_view::{CameraFitFn, SelectFn, WindowAppCallback, WindowAppFn, ZoomFn};
use std::rc::Rc;

pub(super) fn zoom(this: &Entity<StudioShell>) -> Rc<ZoomFn> {
    let this = this.clone();
    Rc::new(
        move |(factor, anchor): (f32, Point<Pixels>), _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                let anchor = (f32::from(anchor.x), f32::from(anchor.y));
                shell.camera.zoom_by(factor, anchor);
                cx.notify();
            });
        },
    )
}

pub(super) fn select_node(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<SelectFn> {
    let nodes = world.clone();
    let this = this.clone();
    Rc::new(move |id: &str, window: &mut Window, cx: &mut App| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            // Reverse of the render-path conversion: resolve the canvas id
            // back to its NodeId through the same projected world.
            let Some(node) = nodes.nodes.iter().find(|node| node.id == id) else {
                return;
            };
            let Some(node_id) = node.node_id else {
                return;
            };
            let modifiers = window.modifiers();
            if modifiers.control || modifiers.shift {
                if shell.selected_nodes.contains(&node_id) {
                    shell.selected_nodes.remove(&node_id);
                    if shell.selected_node == Some(node_id) {
                        shell.selected_node = None;
                    }
                } else {
                    shell.selected_nodes.insert(node_id);
                    shell.selected_node = Some(node_id);
                }
            } else if !shell.selected_nodes.contains(&node_id) {
                shell.selected_nodes.clear();
                shell.selected_nodes.insert(node_id);
                shell.selected_node = Some(node_id);
            } else {
                shell.selected_node = Some(node_id);
            }
            shell.selected_route = None;
            // Dragging moves the whole selection; seed every member so the
            // world positions survive a move of untouched nodes.
            for selected in shell.selected_nodes.clone() {
                if let Some(projected) = nodes
                    .nodes
                    .iter()
                    .find(|node| node.node_id == Some(selected))
                    .map(|node| (node.x, node.y))
                {
                    shell.node_positions.entry(selected).or_insert(projected);
                }
            }
            let pos = window.mouse_position();
            shell.camera.adjusted = true;
            shell.node_drag = Some((node_id, f32::from(pos.x), f32::from(pos.y)));
            shell.node_drag_snap = (0., 0.);
            shell.sync_drag_demand();
            // Single click selects; a double click on the same node also
            // opens the inspector.
            if shell.element_double_clicked(StudioShell::element_key("node", node_id)) {
                shell.open_inspector();
            }
            cx.notify();
        });
    })
}

pub(super) fn background_move(
    this: &Entity<StudioShell>,
) -> WindowAppFn<(Point<Pixels>, Option<MouseButton>)> {
    let this = this.clone();
    Box::new(
        move |(position, pressed): (Point<Pixels>, Option<MouseButton>),
              window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                let (x, y) = (f32::from(position.x), f32::from(position.y));
                // Release the pan grab as soon as no pan button is held, even
                // when the release happened outside the canvas (or was a
                // touchpad tap with no physical button at all).
                let held = matches!(
                    pressed,
                    Some(MouseButton::Left | MouseButton::Middle | MouseButton::Right)
                );
                if !held {
                    shell.pan_grab = None;
                    shell.node_drag = None;
                    shell.node_drag_snap = (0., 0.);
                    shell.knob_drag = None;
                    shell.drag_guides = None;
                    // Hover for the fault tooltip: tracked here, painted by
                    // the meter frame, so a change costs one frame and never
                    // a page rebuild. Anything on a card counts, port dots
                    // included.
                    let hovered = shell.graph_canvas_frame.read().and_then(|frame| {
                        use pages::graph_canvas::HitTarget;
                        let id = match frame.hit_test((x, y))? {
                            HitTarget::Node(id)
                            | HitTarget::Knob { node: id, .. }
                            | HitTarget::Expand { node: id }
                            | HitTarget::Chip { node: id }
                            | HitTarget::Badge { node: id } => id.to_owned(),
                            HitTarget::Port(port) => return frame.world.node_of_port(port),
                            HitTarget::Wire(_) => return None,
                        };
                        frame
                            .world
                            .nodes
                            .iter()
                            .find(|node| node.id == id)
                            .and_then(|node| node.node_id)
                    });
                    if shell.hovered_node != hovered {
                        shell.hovered_node = hovered;
                        cx.notify();
                    }
                    // A right press released within the drag threshold opens
                    // the context menu over the node under the cursor, or
                    // the background menu over the canvas or a wire; a
                    // longer stroke was a pan.
                    if let Some(pressed) = shell.right_press.take()
                        && (x - pressed.0).hypot(y - pressed.1) <= 3.
                    {
                        shell.context_menu = Some(match hovered {
                            Some(node) => ContextMenu::Node {
                                node,
                                at: (x, y),
                                color_open: false,
                            },
                            None => ContextMenu::Background { at: (x, y) },
                        });
                        cx.notify();
                    }
                    let clicked = shell
                        .background_click
                        .take()
                        .is_some_and(|(bx, by)| (x - bx).hypot(y - by) <= 3.);
                    if let Some(drag) = shell.selection_drag.take() {
                        // A plain background click clears the selection; a
                        // rubber band already applied its live selection.
                        if !drag.moved && clicked {
                            shell.clear_canvas_selection();
                        }
                        cx.notify();
                    } else if clicked {
                        shell.clear_canvas_selection();
                        cx.notify();
                    }
                    if shell.connection_drag.take().is_some() {
                        cx.notify();
                    }
                    shell.sync_drag_demand();
                    return;
                }
                if let Some((id, control, start_y, start_value)) = shell.knob_drag {
                    // Vertical drag on the dial; up increases. The value
                    // math is anchored to the press so mid-drag camera or
                    // concurrent edits cannot feedback-loop the knob.
                    let value = control.drag(start_value, (y - start_y) as f64);
                    let edit = {
                        let state = shell.state.read(cx);
                        state
                            .snapshot
                            .graph
                            .node(id)
                            .and_then(|entry| control.edit(id, entry, value))
                    };
                    if let Some(edit) = edit {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                        cx.notify();
                    }
                }
                if let Some(drag) = shell.selection_drag.as_mut() {
                    drag.current = (x, y);
                    if !drag.moved && (x - drag.anchor.0).hypot(y - drag.anchor.1) > 3. {
                        drag.moved = true;
                        shell.background_click = None;
                    }
                    // Apply the live selection here, in the event phase: the
                    // cached page rebuilds on this notify, and a notify raised
                    // from the hot layer's paint would never reach it.
                    let (moved, anchor, current, additive) =
                        (drag.moved, drag.anchor, drag.current, drag.additive);
                    let frame = shell.graph_canvas_frame.read();
                    if moved && let Some(frame) = frame {
                        let a = frame.local_position(anchor);
                        let b = frame.local_position(current);
                        let min = frame.camera.canvas_to_world((a.0.min(b.0), a.1.min(b.1)));
                        let max = frame.camera.canvas_to_world((a.0.max(b.0), a.1.max(b.1)));
                        let world_rect = Bounds {
                            origin: point(min.0, min.1),
                            size: size(max.0 - min.0, max.1 - min.1),
                        };
                        if shell.apply_box_select(world_rect, &frame.world.nodes, additive, cx) {
                            cx.notify();
                        }
                    }
                }
                if let Some((id, gx, gy)) = shell.node_drag {
                    shell.camera.adjusted = true;
                    let raw = shell.camera.canvas_delta_to_world((x - gx, y - gy));
                    // Alignment snapping (item F): the dragged selection's
                    // bounding box seeks the other cards' edges and the
                    // horizontal axis within 6 screen px. The threshold
                    // reads the drag's free path: stored positions carry
                    // the offset previous frames baked in, so it is undone
                    // first — measuring the per-frame step instead would
                    // re-match the same target every sub-threshold move and
                    // pin the card forever. Alt bypasses it and releases a
                    // baked one.
                    let baked = shell.node_drag_snap;
                    shell.node_drag_snap = (0., 0.);
                    if window.modifiers().alt {
                        shell.drag_guides = None;
                    } else if let Some(frame) = shell.graph_canvas_frame.read() {
                        let (snapped, guides) = snap_delta(
                            &frame,
                            &shell.selected_nodes,
                            &shell.node_positions,
                            baked,
                            raw,
                        );
                        shell.node_drag_snap = snapped;
                        shell.drag_guides = Some(guides);
                    }
                    // The stored positions bake in the previous offset: this
                    // frame's move lands them on the new one, or releases
                    // them back onto the free path when the snap lets go.
                    let applied = (
                        raw.0 + shell.node_drag_snap.0 - baked.0,
                        raw.1 + shell.node_drag_snap.1 - baked.1,
                    );
                    let selection: Vec<_> = shell.selected_nodes.iter().copied().collect();
                    for id in selection {
                        if let Some(pos) = shell.node_positions.get_mut(&id) {
                            pos.0 += applied.0;
                            pos.1 += applied.1;
                        }
                    }
                    shell.node_drag = Some((id, x, y));
                    cx.notify();
                }
                if let Some((gx, gy)) = shell.pan_grab {
                    shell.camera.pan_by(x - gx, y - gy);
                    shell.node_drag = None;
                    shell.node_drag_snap = (0., 0.);
                    shell.pan_grab = Some((x, y));
                    shell.sync_drag_demand();
                    cx.notify();
                }
            });
        },
    )
}

pub(super) fn background_down(
    this: &Entity<StudioShell>,
) -> Rc<WindowAppCallback<(MouseDownEvent, Bounds<Pixels>)>> {
    let this = this.clone();
    Rc::new(
        move |(event, _bounds): (MouseDownEvent, Bounds<Pixels>),
              window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                let anchor = (
                    f32::from(window.mouse_position().x),
                    f32::from(window.mouse_position().y),
                );
                match event.button {
                    // Left on the background: plain click clears the selection
                    // on release; dragging past the threshold rubber-bands.
                    MouseButton::Left if !shell.hand_tool => {
                        shell.background_click = Some(anchor);
                        let modifiers = window.modifiers();
                        shell.selection_drag = Some(SelectionDrag {
                            anchor,
                            current: anchor,
                            additive: modifiers.control || modifiers.shift,
                            base: shell.selected_nodes.clone(),
                            moved: false,
                            hits: Default::default(),
                        });
                    }
                    // Right and middle button (and the hand tool): pan grab.
                    // The right press also arms the context menu: released
                    // within the drag threshold it opens the menu, beyond it
                    // the stroke was a pan.
                    _ => {
                        shell.pan_grab = Some(anchor);
                        shell.background_click = None;
                        shell.selection_drag = None;
                        shell.right_press = (event.button == MouseButton::Right).then_some(anchor);
                    }
                }
                shell.sync_drag_demand();
                cx.notify();
            });
        },
    )
}

pub(super) fn scroll_pan(this: &Entity<StudioShell>) -> WindowAppFn<(f32, f32)> {
    let this = this.clone();
    Box::new(
        move |(dx, dy): (f32, f32), _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.camera.pan_by(dx, dy);
                cx.notify();
            });
        },
    )
}

pub(super) fn camera_fit(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<CameraFitFn> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(
        move |fitted: &Camera, canvas: Size<Pixels>, _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                let viewport = Bounds {
                    origin: point(0., 0.),
                    size: size(f32::from(canvas.width), f32::from(canvas.height)),
                };
                // Cull against the camera this frame actually rendered, not
                // the shell's fit bookkeeping: `fitted` is the shell camera
                // verbatim once the user has zoomed or panned, and only
                // differs from it in fit mode — where the fitted view is
                // what is on screen. The set is replaced, never extended:
                // demand must shrink when the camera leaves nodes off
                // screen, or their level windows keep being drained and
                // their ballistics advancing for a view nobody sees
                // (audit HC-9).
                let mut points = shell.node_meter_points.borrow_mut();
                points.clear();
                points.extend(
                    nodes
                        .nodes
                        .iter()
                        .filter(|node| {
                            fitted
                                .project_rect(pages::graph_canvas::NodeLayout::new(node).card)
                                .intersects(&viewport)
                        })
                        .filter_map(|node| node.node_id),
                );
                drop(points);
                let canvas_size = Some((f32::from(canvas.width), f32::from(canvas.height)));
                let changed = shell.canvas_size != canvas_size
                    || (!shell.camera.adjusted && shell.camera != *fitted);
                shell.canvas_size = canvas_size;
                if !shell.camera.adjusted {
                    shell.camera = *fitted;
                }
                if changed {
                    cx.notify();
                }
            });
        },
    )
}

/// Resolves a canvas node id back to its `NodeId` through the projected
/// world — the reverse of the render-path id conversion, shared by every
/// card-anchored gesture.
fn resolve_node(world: &CanvasWorld, id: &str) -> Option<moiren_core::graph::NodeId> {
    world
        .nodes
        .iter()
        .find(|node| node.id == id)
        .and_then(|node| node.node_id)
}

/// The 6 screen-px snap threshold, in world units at the frame's zoom.
const SNAP_THRESHOLD_SCREEN: f32 = 6.;

/// Alignment snapping for the in-flight card drag (item F): the dragged
/// selection's bounding box seeks the other cards' left/center/right and
/// top/middle/bottom edges plus the horizontal axis, nearest candidate
/// within the threshold wins per axis, and the aligned world coordinate
/// comes back as the guide lines to paint. `positions` carry the offset
/// previous frames baked in (`baked`): the candidates are measured on the
/// drag's free path — stored minus baked plus this frame's raw move — so a
/// snapped card lets go once the cursor leaves the threshold. Returns the
/// offset to bake in going forward.
fn snap_delta(
    frame: &CanvasFrame,
    dragged: &std::collections::BTreeSet<moiren_core::graph::NodeId>,
    positions: &std::collections::HashMap<moiren_core::graph::NodeId, (f32, f32)>,
    baked: (f32, f32),
    raw: (f32, f32),
) -> ((f32, f32), pages::graph_canvas::DragGuides) {
    let threshold = SNAP_THRESHOLD_SCREEN / frame.camera.zoom;
    // The dragged selection's candidate bounding box: the free path's
    // positions this frame.
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for id in dragged {
        let Some(&(x, y)) = positions.get(id) else {
            continue;
        };
        let Some(node) = frame
            .world
            .nodes
            .iter()
            .find(|node| node.node_id == Some(*id))
        else {
            continue;
        };
        let card = pages::graph_canvas::NodeLayout::new(node).card;
        min_x = min_x.min(x + raw.0 - baked.0);
        min_y = min_y.min(y + raw.1 - baked.1);
        max_x = max_x.max(x + raw.0 - baked.0 + card.size.width);
        max_y = max_y.max(y + raw.1 - baked.1 + card.size.height);
    }
    if min_x > max_x {
        return ((0., 0.), pages::graph_canvas::DragGuides::default());
    }
    let candidate_x = [min_x, (min_x + max_x) / 2., max_x];
    let candidate_y = [min_y, (min_y + max_y) / 2., max_y];
    // (|offset|, offset, aligned world coordinate) per axis.
    let mut best_x: Option<(f32, f32, f32)> = None;
    let mut best_y: Option<(f32, f32, f32)> = None;
    for node in &frame.world.nodes {
        let Some(other) = node.node_id else {
            continue;
        };
        if dragged.contains(&other) {
            continue;
        }
        let card = pages::graph_canvas::NodeLayout::new(node).card;
        for target in [
            card.origin.x,
            card.origin.x + card.size.width / 2.,
            card.origin.x + card.size.width,
        ] {
            for &candidate in &candidate_x {
                let offset = target - candidate;
                if offset.abs() <= threshold
                    && best_x.is_none_or(|(best, _, _)| offset.abs() < best)
                {
                    best_x = Some((offset.abs(), offset, target));
                }
            }
        }
        for target in [
            card.origin.y,
            card.origin.y + card.size.height / 2.,
            card.origin.y + card.size.height,
            // The horizontal axis is a target of its own.
            0.,
        ] {
            for &candidate in &candidate_y {
                let offset = target - candidate;
                if offset.abs() <= threshold
                    && best_y.is_none_or(|(best, _, _)| offset.abs() < best)
                {
                    best_y = Some((offset.abs(), offset, target));
                }
            }
        }
    }
    let mut guides = pages::graph_canvas::DragGuides::default();
    let mut snapped = (0., 0.);
    if let Some((_, offset, target)) = best_x {
        snapped.0 = offset;
        // The guide paints on the edge the drag snapped to.
        guides.x = Some(target);
    }
    if let Some((_, offset, target)) = best_y {
        snapped.1 = offset;
        guides.y = Some(target);
    }
    (snapped, guides)
}

pub(super) fn port_down(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<(pages::graph_canvas::PortTarget, (f32, f32))>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |(port, cursor), window, cx| {
        this.update(cx, |shell, cx| {
            if !shell.begin_pointer_session(window) {
                use pages::graph_canvas::PortDirection;
                shell.camera.adjusted = true;
                for node in &nodes.nodes {
                    if let Some(node_id) = node.node_id {
                        shell
                            .node_positions
                            .entry(node_id)
                            .or_insert((node.x, node.y));
                    }
                }
                // Dragging an occupied input picks its wire end up instead of
                // starting a fresh reverse connection: the drag moves that
                // edge's source output (design 18 A4). An empty input keeps
                // the reverse drag.
                let picked = (port.direction == PortDirection::Input)
                    .then(|| {
                        shell
                            .state
                            .read(cx)
                            .snapshot
                            .graph
                            .graph
                            .edges()
                            .iter()
                            .find(|edge| edge.dst_port() == port.id)
                            .map(|edge| DetachedEdge {
                                edge: edge.id(),
                                destination: port.id,
                            })
                    })
                    .flatten();
                let port = picked
                    .map(|picked| {
                        let source = shell
                            .state
                            .read(cx)
                            .snapshot
                            .graph
                            .graph
                            .get_edge(picked.edge)
                            .map(|edge| edge.src_port());
                        pages::graph_canvas::PortTarget {
                            id: source.unwrap_or(port.id),
                            direction: PortDirection::Output,
                        }
                    })
                    .unwrap_or(port);
                shell.connection_drag = Some(ConnectionDrag {
                    port,
                    cursor,
                    detached: picked,
                });
                cx.notify();
            }
        })
    })
}

pub(super) fn connection_move(this: &Entity<StudioShell>) -> Rc<WindowAppCallback<(f32, f32)>> {
    let this = this.clone();
    Rc::new(move |cursor, _, cx| {
        this.update(cx, |shell, cx| {
            if let Some(drag) = &mut shell.connection_drag {
                drag.cursor = cursor;
                cx.notify();
            }
        })
    })
}

pub(super) fn port_up(
    this: &Entity<StudioShell>,
) -> Rc<WindowAppCallback<Option<pages::graph_canvas::ConnectionCandidate>>> {
    let this = this.clone();
    Rc::new(move |candidate, _, cx| {
        this.update(cx, |shell, cx| {
            use pages::graph_canvas::{CandidateStatus, PortDirection};
            let Some(drag) = shell.connection_drag.take() else {
                return;
            };
            if let Some(picked) = drag.detached {
                match candidate {
                    // Back on a connectable input: its own port restores the
                    // edge untouched, any other reroutes it in one atomic
                    // edit.
                    Some(candidate)
                        if matches!(
                            candidate.status,
                            CandidateStatus::Valid | CandidateStatus::Displacing
                        ) =>
                    {
                        if candidate.port.id != picked.destination {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(
                                    crate::service::GraphEdit::Reconnect {
                                        edge: picked.edge,
                                        destination: candidate.port.id,
                                    },
                                ))
                            });
                        }
                    }
                    // A picked head released in the void — or on an
                    // incompatible target — disconnects the edge: that is
                    // the drag-to-disconnect gesture. Escape restores
                    // instead.
                    _ => shell.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::EditGraph(
                            crate::service::GraphEdit::Disconnect(picked.edge),
                        ))
                    }),
                }
                cx.notify();
                return;
            }
            if let Some(candidate) = candidate
                && matches!(
                    candidate.status,
                    CandidateStatus::Valid | CandidateStatus::Displacing
                )
            {
                let (source, destination) = match drag.port.direction {
                    PortDirection::Output => (drag.port.id, candidate.port.id),
                    PortDirection::Input => (candidate.port.id, drag.port.id),
                };
                // Both port drops and node-body drops go through the
                // displacing connect so an occupied input reroutes instead
                // of faulting (design 18 A1/A2).
                shell.state.update(cx, |state, _| {
                    state.dispatch(AppRequest::EditGraph(
                        crate::service::GraphEdit::ConnectReplacing {
                            source,
                            destination,
                        },
                    ))
                });
            }
            cx.notify();
        })
    })
}

pub(super) fn knob_down(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<(String, pages::graph_canvas::KnobControl, f32)>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |(id, control, y), window, cx| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            let Some(node_id) = resolve_node(&nodes, &id) else {
                return;
            };
            let current = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(node_id)
                .map(|entry| control.current(entry));
            let Some(current) = current else {
                return;
            };
            // Double-click resets to the control's default, mirroring the
            // element sliders' behavior.
            if shell.element_double_clicked(StudioShell::element_key("knob", (node_id, control))) {
                let edit = {
                    let state = shell.state.read(cx);
                    state
                        .snapshot
                        .graph
                        .node(node_id)
                        .and_then(|entry| control.edit(node_id, entry, control.spec().default))
                };
                if let Some(edit) = edit {
                    shell
                        .state
                        .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                }
            } else {
                shell.knob_drag = Some((node_id, control, y, current));
                shell.sync_drag_demand();
            }
            cx.notify();
        })
    })
}

pub(super) fn chip(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<(String, Bounds<Pixels>)>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |(id, anchor), window, cx| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            let Some(node_id) = resolve_node(&nodes, &id) else {
                return;
            };
            // The chip selects its card like a press anywhere on it, then
            // hangs the endpoint menu off the chip.
            shell.selected_nodes.clear();
            shell.selected_nodes.insert(node_id);
            shell.selected_node = Some(node_id);
            shell.selected_route = None;
            shell.binding_menu = Some(BindingMenu {
                node: node_id,
                anchor,
            });
            cx.notify();
        })
    })
}

pub(super) fn badge(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<(String, Bounds<Pixels>)>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |(id, anchor), window, cx| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            let Some(node_id) = resolve_node(&nodes, &id) else {
                return;
            };
            // The badge opens the fault popover on the card's fault group;
            // it selects nothing, mirroring the hover tooltip that also
            // presumes nothing.
            shell.fault_popover = Some(super::super::fault_popover::FaultPopover {
                node: Some(node_id),
                anchor,
            });
            cx.notify();
        })
    })
}

pub(super) fn knob_wheel(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<(String, pages::graph_canvas::KnobControl, f32)>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |(id, control, notches), _, cx| {
        this.update(cx, |shell, cx| {
            let Some(node_id) = resolve_node(&nodes, &id) else {
                return;
            };
            let edit = {
                let state = shell.state.read(cx);
                state.snapshot.graph.node(node_id).and_then(|entry| {
                    control.edit(
                        node_id,
                        entry,
                        control.wheel(control.current(entry), notches as f64),
                    )
                })
            };
            if let Some(edit) = edit {
                shell
                    .state
                    .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                cx.notify();
            }
        })
    })
}

pub(super) fn expand(
    this: &Entity<StudioShell>,
    world: &std::sync::Arc<CanvasWorld>,
) -> Rc<WindowAppCallback<String>> {
    let this = this.clone();
    let nodes = world.clone();
    Rc::new(move |id, window, cx| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            let Some(node_id) = resolve_node(&nodes, &id) else {
                return;
            };
            // The card's three-dot glyph opens the same node context menu a
            // right click does, anchored at the click.
            let pos = window.mouse_position();
            shell.select_single_node(node_id);
            shell.context_menu = Some(ContextMenu::Node {
                node: node_id,
                at: (f32::from(pos.x), f32::from(pos.y)),
                color_open: false,
            });
            cx.notify();
        })
    })
}

pub(super) fn delete(this: &Entity<StudioShell>) -> Rc<WindowAppCallback<()>> {
    let this = this.clone();
    Rc::new(move |(), _, cx| this.update(cx, |shell, cx| shell.delete_graph_selection(cx)))
}

pub(super) fn cancel(this: &Entity<StudioShell>) -> Rc<WindowAppCallback<()>> {
    let this = this.clone();
    Rc::new(move |(), _, cx| this.update(cx, |shell, cx| shell.cancel_active_gesture(cx)))
}

pub(super) fn drop_node(
    this: &Entity<StudioShell>,
) -> Rc<WindowAppCallback<(NodeTemplate, (f32, f32))>> {
    let this = this.clone();
    Rc::new(move |(template, position), _, cx| {
        this.update(cx, |shell, cx| shell.spawn_node(template, position, cx));
    })
}

pub(super) fn select_wire(this: &Entity<StudioShell>) -> Rc<SelectFn> {
    let this = this.clone();
    Rc::new(move |id, window, cx| {
        this.update(cx, |shell, cx| {
            if shell.begin_pointer_session(window) {
                return;
            }
            shell.selected_route = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .edges()
                .iter()
                .find(|e| format!("{:?}", e.id()) == id)
                .map(|e| (e.src_port(), e.dst_port()));
            shell.selected_node = None;
            shell.selected_nodes.clear();
            if let Some(route) = shell.selected_route
                && shell.element_double_clicked(StudioShell::element_key("route", route))
            {
                shell.open_inspector();
            }
            cx.notify();
        })
    })
}
