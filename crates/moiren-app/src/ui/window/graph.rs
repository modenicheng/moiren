use rust_i18n::t;

use super::*;

mod compressor;
mod library;
pub(super) use compressor::CompressorControls;
pub(super) mod meter_frame;
mod node_inspector;
pub(super) use library::PendingNodeSpawn;
pub(in crate::ui::window) use library::{library, library_rail};
pub(in crate::ui::window) use node_inspector::inspector;

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct ConnectionDrag {
    port: pages::graph_canvas::PortTarget,
    cursor: (f32, f32),
}

/// A rubber-band selection drag on the canvas background. Starts as a plain
/// background click; past the 3 px threshold it becomes a live band that
/// replaces the selection (or unions with `base` when Ctrl/Shift held).
pub(super) struct SelectionDrag {
    anchor: (f32, f32),
    current: (f32, f32),
    additive: bool,
    base: std::collections::BTreeSet<moiren_core::graph::NodeId>,
    moved: bool,
    hits: std::collections::BTreeSet<moiren_core::graph::NodeId>,
}

impl StudioShell {
    /// The rubber band's live application, driven by the meter hot layer
    /// once per frame while a box select is in flight. Returns whether the
    /// selection changed (the caller notifies once per change, not per
    /// frame).
    pub(in crate::ui::window) fn apply_box_select(
        &mut self,
        rect: Bounds<f32>,
        nodes: &[pages::graph_canvas::CanvasNode],
        additive: bool,
        _cx: &mut Context<Self>,
    ) -> bool {
        let hits: std::collections::BTreeSet<moiren_core::graph::NodeId> = nodes
            .iter()
            .filter(|node| {
                pages::graph_canvas::NodeLayout::new(node)
                    .card
                    .intersects(&rect)
            })
            .filter_map(|node| node.node_id)
            .collect();
        let Some(drag) = self.selection_drag.as_mut() else {
            return false;
        };
        if !drag.moved {
            return false;
        }
        drag.hits = hits;
        let selection = if additive {
            drag.base.union(&drag.hits).copied().collect()
        } else {
            drag.hits.clone()
        };
        if self.selected_nodes == selection {
            return false;
        }
        self.selected_nodes = selection;
        if self.selected_nodes.len() == 1 {
            self.selected_node = self.selected_nodes.iter().next().copied();
        } else if self
            .selected_node
            .is_none_or(|id| !self.selected_nodes.contains(&id))
        {
            self.selected_node = None;
        }
        if !self.selected_nodes.is_empty() {
            self.selected_route = None;
        }
        true
    }

    /// Shared pointer-session preamble for canvas hit targets: cancel any
    /// in-flight drag or click first; under the hand tool the pointer instead
    /// becomes a pan grab. Returns true when the hand tool consumed the event.
    fn begin_pointer_session(&mut self, window: &mut Window) -> bool {
        self.node_drag = None;
        self.pan_grab = None;
        self.background_click = None;
        self.selection_drag = None;
        self.sync_drag_demand();
        if self.hand_tool {
            let position = window.mouse_position();
            self.pan_grab = Some((f32::from(position.x), f32::from(position.y)));
            self.sync_drag_demand();
            true
        } else {
            false
        }
    }

    fn clear_canvas_selection(&mut self) {
        self.selected_nodes.clear();
        self.selected_node = None;
        self.selected_route = None;
    }

    fn delete_graph_selection(&mut self, cx: &mut Context<Self>) {
        // The primary selection may have been set from outside the canvas
        // (mixer strip, library locator); it always deletes with the band.
        if let Some(id) = self.selected_node {
            self.selected_nodes.insert(id);
        }
        let mut removed = false;
        for id in self.selected_nodes.iter().copied().collect::<Vec<_>>() {
            let removed_here = self.state.update(cx, |state, _| {
                state
                    .try_dispatch(AppRequest::EditGraph(
                        crate::service::GraphEdit::RemoveNode(id),
                    ))
                    .is_ok()
            });
            removed |= removed_here;
        }
        if !removed {
            let edge_id = {
                let graph = &self.state.read(cx).snapshot.graph.graph;
                self.selected_route.and_then(|ports| {
                    graph
                        .edges()
                        .iter()
                        .find(|edge| (edge.src_port(), edge.dst_port()) == ports)
                        .map(|edge| edge.id())
                })
            };
            if let Some(edge_id) = edge_id
                && self.state.update(cx, |state, _| {
                    state
                        .try_dispatch(AppRequest::EditGraph(
                            crate::service::GraphEdit::Disconnect(edge_id),
                        ))
                        .is_ok()
                })
            {
                self.selected_route = None;
            }
        } else {
            self.selected_nodes.clear();
            self.selected_node = None;
            self.selected_route = None;
        }
        self.connection_drag = None;
        self.node_drag = None;
        cx.notify();
    }
}

/// The graph page's cached content: canvas, library, editing toolbar. The
/// shell renders this through [`super::meter_frame::page`], which caches it
/// behind [`meter_frame::ControlsKey`] and adds the animation-owner meter
/// frame as a sibling.
pub(super) fn page(shell: &StudioShell, window: &Window, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    // Meter bar readings are not baked in: the canvas paint consults the
    // live ballistics per draw, so meter motion repaints without the page
    // cache being invalidated or the world being re-projected.
    let world = std::sync::Arc::new(pages::graph_canvas::CanvasWorld::positioned(
        &state.snapshot,
        &shell.node_positions,
    ));
    let camera = shell.camera;
    // The canvas view identifies nodes by their Debug id string; this is the
    // single NodeId -> canvas-id conversion on the render path.
    let selected_ids: Vec<String> = shell
        .selected_nodes
        .iter()
        .map(|id| format!("{id:?}"))
        .collect();
    let selected_wire = shell.selected_route.and_then(|ports| {
        state
            .snapshot
            .graph
            .graph
            .edges()
            .iter()
            .find(|edge| (edge.src_port(), edge.dst_port()) == ports)
            .map(|edge| format!("{:?}", edge.id()))
    });
    let this = cx.entity();
    let node_levels = meter_frame::node_levels(&this);

    // Canvas view with interaction plumbed to the shell entity.
    let on_zoom = {
        let this = this.clone();
        move |(factor, anchor): (f32, Point<Pixels>), _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                let anchor = (f32::from(anchor.x), f32::from(anchor.y));
                shell.camera.zoom_by(factor, anchor);
                cx.notify();
            });
        }
    };
    let on_select_node = {
        let nodes = world.clone();
        let this = this.clone();
        move |id: &str, window: &mut Window, cx: &mut App| {
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
                shell.sync_drag_demand();
                // Single click selects; a double click on the same node also
                // opens the inspector.
                if shell.element_double_clicked(StudioShell::element_key("node", node_id)) {
                    shell.open_inspector();
                }
                cx.notify();
            });
        }
    };
    let on_bg_move = {
        let this = this.clone();
        move |(position, pressed): (Point<Pixels>, Option<MouseButton>),
              _window: &mut Window,
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
                    let delta = shell.camera.canvas_delta_to_world((x - gx, y - gy));
                    let selection: Vec<_> = shell.selected_nodes.iter().copied().collect();
                    for id in selection {
                        if let Some(pos) = shell.node_positions.get_mut(&id) {
                            pos.0 += delta.0;
                            pos.1 += delta.1;
                        }
                    }
                    shell.node_drag = Some((id, x, y));
                    cx.notify();
                }
                if let Some((gx, gy)) = shell.pan_grab {
                    shell.camera.pan_by(x - gx, y - gy);
                    shell.node_drag = None;
                    shell.pan_grab = Some((x, y));
                    shell.sync_drag_demand();
                    cx.notify();
                }
            });
        }
    };
    let on_bg_down = {
        let this = this.clone();
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
                    _ => {
                        shell.pan_grab = Some(anchor);
                        shell.background_click = None;
                        shell.selection_drag = None;
                    }
                }
                shell.sync_drag_demand();
                cx.notify();
            });
        }
    };
    let on_scroll_pan = {
        let this = this.clone();
        move |(dx, dy): (f32, f32), _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.camera.pan_by(dx, dy);
                cx.notify();
            });
        }
    };
    let on_camera_fit = {
        let this = this.clone();
        let nodes = world.clone();
        move |fitted: &Camera, canvas: Size<Pixels>, _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
                let viewport = Bounds {
                    origin: point(0., 0.),
                    size: size(f32::from(canvas.width), f32::from(canvas.height)),
                };
                shell.node_meter_points.borrow_mut().extend(
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
        }
    };
    let canvas_view = pages::graph_canvas_view::graph_canvas_view(
        world.clone(),
        camera,
        &selected_ids,
        selected_wire.as_deref(),
        pages::graph_canvas_view::CanvasCallbacks {
            focus: shell.graph_focus.clone(),
            connection: shell.connection_drag.map(|drag| (drag.port, drag.cursor)),
            node_levels,
            on_port_down: std::rc::Rc::new({
                let this = this.clone();
                let nodes = world.clone();
                move |(port, cursor), window, cx| {
                    this.update(cx, |shell, cx| {
                        if !shell.begin_pointer_session(window) {
                            shell.camera.adjusted = true;
                            for node in &nodes.nodes {
                                if let Some(node_id) = node.node_id {
                                    shell
                                        .node_positions
                                        .entry(node_id)
                                        .or_insert((node.x, node.y));
                                }
                            }
                            shell.connection_drag = Some(ConnectionDrag { port, cursor });
                            cx.notify();
                        }
                    })
                }
            }),
            on_connection_move: std::rc::Rc::new({
                let this = this.clone();
                move |cursor, _, cx| {
                    this.update(cx, |shell, cx| {
                        if let Some(drag) = &mut shell.connection_drag {
                            drag.cursor = cursor;
                            cx.notify();
                        }
                    })
                }
            }),
            on_port_up: std::rc::Rc::new({
                let this = this.clone();
                move |target, _, cx| {
                    this.update(cx, |shell, cx| {
                        use pages::graph_canvas::PortDirection;
                        let Some(drag) = shell.connection_drag.take() else {
                            return;
                        };
                        if let Some(target) = target
                            && target.direction != drag.port.direction
                        {
                            let (source, destination) = match drag.port.direction {
                                PortDirection::Output => (drag.port.id, target.id),
                                PortDirection::Input => (target.id, drag.port.id),
                            };
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(
                                    crate::service::GraphEdit::Connect {
                                        source,
                                        destination,
                                    },
                                ))
                            });
                        }
                        cx.notify();
                    })
                }
            }),
            on_delete: std::rc::Rc::new({
                let this = this.clone();
                move |(), _, cx| this.update(cx, |shell, cx| shell.delete_graph_selection(cx))
            }),
            on_cancel: std::rc::Rc::new({
                let this = this.clone();
                move |(), _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.connection_drag = None;
                        shell.node_drag = None;
                        shell.pan_grab = None;
                        shell.background_click = None;
                        shell.selection_drag = None;
                        cx.notify();
                    })
                }
            }),
            on_drop_node: std::rc::Rc::new({
                let this = this.clone();
                move |(template, position), _, cx| {
                    this.update(cx, |shell, cx| shell.spawn_node(template, position, cx));
                }
            }),
            on_background_down: std::rc::Rc::new(on_bg_down),
            on_canvas_mouse_move: Box::new(on_bg_move),
            on_zoom: std::rc::Rc::new(on_zoom),
            on_pan: Box::new(on_scroll_pan),
            on_select_node: std::rc::Rc::new(on_select_node),
            on_select_wire: std::rc::Rc::new({
                let this = this.clone();
                move |id, window, cx| {
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
                            && shell
                                .element_double_clicked(StudioShell::element_key("route", route))
                        {
                            shell.open_inspector();
                        }
                        cx.notify();
                    })
                }
            }),
            on_camera_fit: std::rc::Rc::new(on_camera_fit),
        },
        // Shared with the meter frame: its overlay repaints bars at this
        // frame's geometry between cache rebuilds.
        shell.graph_canvas_frame.clone(),
    );

    div()
        .id("graph-page")
        .size_full()
        .flex()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .child(
            // Row-reversed so the canvas leads the DOM: gpui has no z-index,
            // and the library's protruding handle must paint above the canvas.
            div()
                .relative()
                .flex()
                .flex_row_reverse()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(
                    div()
                        .id("graph-canvas-host")
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(canvas_view)
                        .child(editing_toolbar(shell, &this)),
                )
                .child(sidebar::library_host(shell, window, cx)),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn editing_toolbar(shell: &StudioShell, this: &Entity<StudioShell>) -> impl IntoElement {
    use controls::*;
    let mut view = div()
        .absolute()
        .top(px(12.))
        .left(px(12.))
        .flex()
        .items_center()
        .gap(px(4.))
        .p(px(4.))
        .rounded(px(theme::RADIUS_MD))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER);
    for (id, name, icon, hand) in [
        ("tool-select", t!("Select"), ActionIcon::Cursor, false),
        ("tool-hand", t!("Hand"), ActionIcon::HandMove, true),
    ] {
        let this = this.clone();
        view = view.child(
            button(id, name.clone())
                .icon(icon)
                .icon_only()
                .tooltip(name)
                .variant(if shell.hand_tool == hand {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        s.hand_tool = hand;
                        cx.notify();
                    })
                }),
        );
    }
    view = view.child(
        div()
            .flex_none()
            .w(px(1.))
            .h(px(16.))
            .mx(px(2.))
            .rounded_full()
            .bg(theme::BORDER),
    );
    // Zoom-out · readout · zoom-in reads as a stepper around the percentage;
    // the readout's width is pinned so the stepper never reflows.
    let zoom_step = |id: &'static str, icon: ActionIcon, name: SharedString, factor: f32| {
        button(id, name.clone())
            .icon(icon)
            .icon_only()
            .tooltip(name)
            .on_click({
                let this = this.clone();
                move |_, _, cx: &mut App| {
                    this.update(cx, |s, cx| {
                        s.camera.zoom_around_center(s.canvas_size, factor);
                        cx.notify();
                    })
                }
            })
    };
    view = view.child(zoom_step(
        "zoom-out",
        ActionIcon::ZoomOut,
        t!("Zoom out").into(),
        1. / 1.1,
    ));
    view = view.child(
        div()
            .min_w(px(44.))
            .flex()
            .justify_center()
            .child(label(format!(
                "{}%",
                (shell.camera.zoom * 100.).round() as i32
            ))),
    );
    view = view.child(zoom_step(
        "zoom-in",
        ActionIcon::ZoomIn,
        t!("Zoom in").into(),
        1.1,
    ));
    view.child(button("zoom-fit", t!("Fit")).on_click({
        let this = this.clone();
        move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.camera.adjusted = false;
                cx.notify();
            })
        }
    }))
}
