use super::*;

mod compressor;
mod library;
pub(super) use compressor::CompressorControls;
mod node_inspector;
pub(super) use library::PendingNodeSpawn;
use library::library;
pub(super) use node_inspector::inspector;

#[cfg(all(test, feature = "gpui-test"))]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct ConnectionDrag {
    port: pages::graph_canvas::PortTarget,
    cursor: (f32, f32),
}

impl StudioShell {
    /// Shared pointer-session preamble for canvas hit targets: cancel any
    /// in-flight drag or click first; under the hand tool the pointer instead
    /// becomes a pan grab. Returns true when the hand tool consumed the event.
    fn begin_pointer_session(&mut self, window: &mut Window) -> bool {
        self.node_drag = None;
        self.pan_grab = None;
        self.background_click = None;
        if self.hand_tool {
            let position = window.mouse_position();
            self.pan_grab = Some((f32::from(position.x), f32::from(position.y)));
            true
        } else {
            false
        }
    }

    fn delete_graph_selection(&mut self, cx: &mut Context<Self>) {
        let graph = &self.state.read(cx).snapshot.graph.graph;
        let edit = self
            .selected_node
            .as_ref()
            .and_then(|selected| {
                graph
                    .nodes()
                    .iter()
                    .find(|node| format!("{:?}", node.id()) == *selected)
                    .map(|node| crate::service::GraphEdit::RemoveNode(node.id()))
            })
            .or_else(|| {
                self.selected_route.and_then(|ports| {
                    graph
                        .edges()
                        .iter()
                        .find(|edge| (edge.src_port(), edge.dst_port()) == ports)
                        .map(|edge| crate::service::GraphEdit::Disconnect(edge.id()))
                })
            });
        if let Some(edit) = edit
            && self.state.update(cx, |state, _| {
                state.try_dispatch(AppRequest::EditGraph(edit)).is_ok()
            })
        {
            self.selected_node = None;
            self.selected_route = None;
        }
        self.connection_drag = None;
        self.node_drag = None;
        cx.notify();
    }
}

pub(super) fn render_graph_page(shell: &StudioShell, cx: &mut Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let mut projected =
        pages::graph_canvas::CanvasWorld::positioned(&state.snapshot, &shell.node_positions);
    if state.snapshot.graph.active_revision == Some(state.snapshot.graph.candidate_revision) {
        for node in &mut projected.nodes {
            node.peaks = node
                .node_id
                .and_then(|id| shell.node_meters.levels(id))
                .map(|levels| levels.map(|level| level.peak));
        }
    }
    let world = std::sync::Arc::new(projected);
    let camera = shell.camera;
    let selected = shell.selected_node.clone();
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
                shell.selected_node = Some(id.to_string());
                shell.selected_route = None;
                if let Some(node) = nodes.nodes.iter().find(|node| node.id == id) {
                    shell
                        .node_positions
                        .entry(id.to_string())
                        .or_insert((node.x, node.y));
                }
                let pos = window.mouse_position();
                shell.camera.adjusted = true;
                shell.node_drag = Some((id.into(), f32::from(pos.x), f32::from(pos.y)));
                if shell.compact {
                    shell.inspector_open = true;
                    shell.sidebar_open = false;
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
                // Release the pan grab as soon as neither pan button is held,
                // even when the release happened outside the canvas (or was a
                // touchpad tap with no physical button at all).
                let held = matches!(pressed, Some(MouseButton::Left | MouseButton::Middle));
                if !held {
                    shell.pan_grab = None;
                    shell.node_drag = None;
                    if shell.background_click.take().is_some_and(|(x, y)| {
                        (f32::from(position.x) - x).hypot(f32::from(position.y) - y) <= 3.
                    }) {
                        shell.selected_node = None;
                        shell.selected_route = None;
                        cx.notify();
                    }
                    if shell.connection_drag.take().is_some() {
                        cx.notify();
                    }
                    return;
                }
                if let Some((id, gx, gy)) = shell.node_drag.clone() {
                    shell.camera.adjusted = true;
                    let delta = shell.camera.canvas_delta_to_world((
                        f32::from(position.x) - gx,
                        f32::from(position.y) - gy,
                    ));
                    if let Some(pos) = shell.node_positions.get_mut(&id) {
                        pos.0 += delta.0;
                        pos.1 += delta.1;
                    }
                    shell.node_drag = Some((id, f32::from(position.x), f32::from(position.y)));
                    cx.notify();
                }
                if let Some((gx, gy)) = shell.pan_grab {
                    if shell.background_click.is_some_and(|(x, y)| {
                        (f32::from(position.x) - x).hypot(f32::from(position.y) - y) > 3.
                    }) {
                        shell.background_click = None;
                    }
                    let (dx, dy) = (f32::from(position.x) - gx, f32::from(position.y) - gy);
                    shell.camera.pan_by(dx, dy);
                    shell.node_drag = None;
                    shell.pan_grab = Some((f32::from(position.x), f32::from(position.y)));
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
                shell.pan_grab = Some(anchor);
                shell.background_click =
                    (event.button == MouseButton::Left && !shell.hand_tool).then_some(anchor);
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
        selected.as_deref(),
        selected_wire.as_deref(),
        pages::graph_canvas_view::CanvasCallbacks {
            focus: shell.graph_focus.clone(),
            connection: shell.connection_drag.map(|drag| (drag.port, drag.cursor)),
            on_port_down: std::rc::Rc::new({
                let this = this.clone();
                let nodes = world.clone();
                move |(port, cursor), window, cx| {
                    this.update(cx, |shell, cx| {
                        if !shell.begin_pointer_session(window) {
                            shell.camera.adjusted = true;
                            for node in &nodes.nodes {
                                shell
                                    .node_positions
                                    .entry(node.id.clone())
                                    .or_insert((node.x, node.y));
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
                        shell.inspector_open = true;
                        if shell.compact {
                            shell.sidebar_open = false;
                        }
                        cx.notify();
                    })
                }
            }),
            on_camera_fit: std::rc::Rc::new(on_camera_fit),
        },
    );

    div()
        .id("graph-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(48.))
                .items_center()
                .justify_between()
                .px(px(theme::SPACE_3))
                .child(div().flex().gap(px(8.)).items_center().child(
                    controls::button("toggle-library", "Node library").on_click({
                        let this = this.clone();
                        move |_, _, cx| {
                            this.update(cx, |s, cx| {
                                s.sidebar_open = !s.sidebar_open;
                                if s.compact {
                                    s.inspector_open = false;
                                }
                                cx.notify();
                            })
                        }
                    }),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(controls::apply(shell, cx))
                        .child(controls::inspector_toggle("toggle-inspector", cx)),
                ),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .when(shell.sidebar_open, |v| v.child(library(shell, cx)))
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
                .when(shell.inspector_open, |view| {
                    if shell.selected_node.is_none() && shell.selected_route.is_some() {
                        view.child(matrix::inspector(shell, cx))
                    } else {
                        view.child(inspector(shell, cx))
                    }
                }),
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
    for (id, name, hand) in [
        ("tool-select", "Select", false),
        ("tool-hand", "Hand", true),
    ] {
        let this = this.clone();
        view = view.child(
            button(id, name)
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
    for (id, name, factor) in [("zoom-out", "−", 1. / 1.1), ("zoom-in", "+", 1.1)] {
        let this = this.clone();
        view = view.child(button(id, name).on_click(move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.camera.zoom_around_center(s.canvas_size, factor);
                cx.notify();
            })
        }));
    }
    view.child(label(format!(
        "{}%",
        (shell.camera.zoom * 100.).round() as i32
    )))
    .child(button("zoom-fit", "Fit").on_click({
        let this = this.clone();
        move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.camera.adjusted = false;
                cx.notify();
            })
        }
    }))
}
