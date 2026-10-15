use super::*;

pub(super) fn render_graph_page(shell: &StudioShell, cx: &mut Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let world = std::sync::Arc::new(pages::graph_canvas::CanvasWorld::positioned(
        &state.snapshot,
        &shell.node_positions,
    ));
    let overview = minimap(world.clone(), shell.camera, shell.canvas_size);
    let camera = shell.camera;
    let selected = shell.selected_node.clone();
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
                shell.pan_grab = None;
                shell.node_drag = None;
                if shell.hand_tool {
                    let pos = window.mouse_position();
                    shell.pan_grab = Some((f32::from(pos.x), f32::from(pos.y)));
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
                if shell.hand_tool {
                    shell.pan_grab = Some((f32::from(pos.x), f32::from(pos.y)));
                } else {
                    shell.node_drag = Some((id.into(), f32::from(pos.x), f32::from(pos.y)));
                }
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
                    return;
                }
                if let Some((id, gx, gy)) = shell.node_drag.clone() {
                    shell.camera.adjusted = true;
                    if let Some(pos) = shell.node_positions.get_mut(&id) {
                        pos.0 += (f32::from(position.x) - gx) / shell.camera.zoom;
                        pos.1 += (f32::from(position.y) - gy) / shell.camera.zoom;
                    }
                    shell.node_drag = Some((id, f32::from(position.x), f32::from(position.y)));
                    cx.notify();
                }
                if let Some((gx, gy)) = shell.pan_grab {
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
        move |(_event, _bounds): (MouseDownEvent, Bounds<Pixels>),
              window: &mut Window,
              cx: &mut App| {
            this.update(cx, |shell, cx| {
                shell.pan_grab = Some((
                    f32::from(window.mouse_position().x),
                    f32::from(window.mouse_position().y),
                ));
                shell.selected_node = None;
                shell.selected_route = None;
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
        move |fitted: &Camera, canvas: Size<Pixels>, _window: &mut Window, cx: &mut App| {
            this.update(cx, |shell, cx| {
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
        world,
        camera,
        selected.as_deref(),
        pages::graph_canvas_view::CanvasCallbacks {
            on_background_down: std::rc::Rc::new(on_bg_down),
            on_canvas_mouse_move: Box::new(on_bg_move),
            on_zoom: std::rc::Rc::new(on_zoom),
            on_pan: Box::new(on_scroll_pan),
            on_select_node: std::rc::Rc::new(on_select_node),
            on_select_wire: std::rc::Rc::new({
                let this = this.clone();
                move |id, window, cx| {
                    this.update(cx, |shell, cx| {
                        shell.node_drag = None;
                        shell.pan_grab = None;
                        if shell.hand_tool {
                            let pos = window.mouse_position();
                            shell.pan_grab = Some((f32::from(pos.x), f32::from(pos.y)));
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
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            controls::button("toggle-library", "Session & nodes").on_click({
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
                        )
                        .child(controls::graph_summary(shell.state.read(cx))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(controls::apply(shell, cx))
                        .child(controls::button("toggle-inspector", "Inspector").on_click({
                            let this = this.clone();
                            move |_, _, cx| {
                                this.update(cx, |s, cx| {
                                    s.inspector_open = !s.inspector_open;
                                    if s.compact {
                                        s.sidebar_open = false;
                                    }
                                    cx.notify();
                                })
                            }
                        })),
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
                        .child(editing_toolbar(shell, &this))
                        .child(overview),
                )
                .when(shell.inspector_open, |v| v.child(inspector(shell, cx))),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn library(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    use crate::ui::state::{SourceMode, output_name};
    use controls::*;
    use moiren_core::graph::NodeKind;
    let this = cx.entity();
    let state = shell.state.read(cx);
    let mut view = div()
        .id("session-io-panel")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(theme::LIBRARY_WIDTH))
        .p(px(theme::SPACE_3))
        .gap(px(10.))
        .bg(theme::SURFACE)
        .border_r_1()
        .border_color(theme::BORDER)
        .overflow_y_scroll()
        .child(heading("Session"))
        .child(label("Source for next start"));
    for mode in [SourceMode::Tone, SourceMode::Input, SourceMode::Application] {
        let this = this.clone();
        view = view.child(
            button(SharedString::from(format!("mode-{mode:?}")), mode.title())
                .variant(if state.source_mode == mode {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        s.state.update(cx, |state, _| {
                            state.source_mode = mode;
                            state.bind_candidate_resources();
                        });
                        cx.notify();
                    })
                }),
        );
    }
    view = view
        .child(label(
            state
                .candidate_spec()
                .map(|spec| crate::ui::state::source_name(&state.snapshot, &spec.source))
                .unwrap_or_else(|| "Select resources in Devices".into()),
        ))
        .child(
            pages::session_setup::PickerRow::new(
                "candidate-output",
                "Output",
                state
                    .selection
                    .output_id
                    .as_ref()
                    .map(|id| output_name(&state.snapshot, id))
                    .unwrap_or_else(|| "Not selected".into()),
            )
            .disabled(state.snapshot.catalog.outputs.is_empty())
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        s.state.update(cx, |state, _| {
                            let catalog = &state.snapshot.catalog;
                            let next = state
                                .selection
                                .output_index(catalog)
                                .map_or(0, |i| (i + 1) % catalog.outputs.len().max(1));
                            state.selection.output_id =
                                catalog.outputs.get(next).map(|row| row.endpoint_id.clone());
                            state.bind_candidate_resources();
                        });
                        cx.notify();
                    })
                }
            }),
        )
        .child(button("choose-devices", "Choose devices").on_click({
            let this = this.clone();
            move |_, _, cx| this.update(cx, |s, cx| s.set_page(Page::Devices, cx))
        }))
        .child(
            div()
                .mt(px(12.))
                .child(pages::session_setup::section_heading("Add node")),
        );
    for (name, kind) in [
        ("Gain", NodeKind::Gain),
        ("Balance", NodeKind::Pan),
        ("Compressor", NodeKind::Compressor),
        ("Bus", NodeKind::Bus),
        ("Input", NodeKind::Source),
        ("Output", NodeKind::Sink),
    ] {
        view = view.child(
            button(SharedString::from(format!("add-{name}")), name)
                .icon(ActionIcon::Plus)
                .on_click(edit(
                    this.clone(),
                    crate::service::GraphEdit::AddNode { kind, channels: 2 },
                )),
        );
    }
    view.child(label(
        "New nodes are candidates. Connect them in Matrix, then apply.",
    ))
    .gpui_test_observe()
}

pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    if shell.selected_node.is_none() && shell.selected_route.is_some() {
        return matrix::inspector(shell, cx).into_any_element();
    }
    use crate::service::GraphEdit;
    use controls::*;
    use moiren_core::graph::NodeKind;
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let selected = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| shell.selected_node.as_deref() == Some(format!("{:?}", n.id()).as_str()));
    let mut panel = panel("graph-inspector").child(heading("Inspector"));
    if let Some(node) = selected {
        let id = node.id();
        let entry = snapshot.graph.node(id).expect("service metadata");
        panel = panel
            .child(heading(entry.label.clone()))
            .child(detail("Kind", format!("{:?}", node.kind())))
            .child(detail("Identity", format!("{id:?}")))
            .child(detail("Channels", node.channels().to_string()));
        if let Some(source) = &entry.source {
            panel = panel.child(detail(
                "Source",
                crate::ui::state::source_name(snapshot, source),
            ));
        }
        if let Some(output) = &entry.output_endpoint_id {
            panel = panel.child(detail(
                "Output",
                crate::ui::state::output_name(snapshot, output),
            ));
        }
        if node.kind() == NodeKind::Compressor {
            panel = panel.child(label(
                "Linked peak compressor with the engine's default settings.",
            ));
        }
        if node.kind() == NodeKind::Gain {
            panel = panel.child(detail("Candidate gain", db(entry.gain))).child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(button("node-gain-down", "−1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            gain: entry.gain * 10f64.powf(-1. / 20.),
                        },
                    )))
                    .child(button("node-gain-up", "+1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            gain: if entry.gain == 0. {
                                0.01
                            } else {
                                (entry.gain * 10f64.powf(1. / 20.)).min(16.)
                            },
                        },
                    )))
                    .child(
                        button("node-unity", "Unity")
                            .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 1. })),
                    ),
            );
        }
        if node.kind() == NodeKind::Pan {
            panel =
                panel
                    .child(detail("Candidate balance", format!("{:+.2}", entry.pan)))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(button("node-left", "Left").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetPan {
                                    node: id,
                                    pan: (entry.pan - 0.1).max(-1.),
                                },
                            )))
                            .child(button("node-center", "Center").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetPan { node: id, pan: 0. },
                            )))
                            .child(button("node-right", "Right").on_click(edit(
                                cx.entity(),
                                GraphEdit::SetPan {
                                    node: id,
                                    pan: (entry.pan + 0.1).min(1.),
                                },
                            ))),
                    );
        }
        if node.kind() == NodeKind::Pan {
            panel = panel.child(stereo_controls(entry, cx));
        }
        for (index, port) in node.inputs().iter().enumerate() {
            panel = panel.child(detail(
                &format!("Input {}", index + 1),
                format!("{:?} · {} ch", port.id(), port.channels()),
            ));
        }
        for (index, port) in node.outputs().iter().enumerate() {
            panel = panel.child(detail(
                &format!("Output {}", index + 1),
                format!("{:?} · {} ch", port.id(), port.channels()),
            ));
        }
        if node.kind() == NodeKind::Bus {
            panel = panel.child(
                button("add-bus-input", "Add bus input")
                    .on_click(edit(cx.entity(), GraphEdit::AddBusInput(id))),
            );
            if let Some(port) = node.inputs().last() {
                panel = panel.child(button("remove-bus-input", "Remove last input").on_click(
                    edit(
                        cx.entity(),
                        GraphEdit::RemoveBusInput {
                            node: id,
                            port: port.id(),
                        },
                    ),
                ));
            }
        }
        panel = panel.child(
            button("remove-node", "Delete node")
                .variant(ButtonVariant::Danger)
                .on_click(edit(cx.entity(), GraphEdit::RemoveNode(id))),
        );
    } else {
        panel = panel.child(label(
            "Select a node to inspect its identity, ports and parameters.",
        ));
    }
    panel
        .child(div().mt(px(12.)).child(graph_summary(state)))
        .child(label(
            "Changes affect the candidate graph. Apply prepares and starts that graph.",
        ))
        .child(label(state.control_feedback()))
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

fn minimap(
    world: std::sync::Arc<pages::graph_canvas::CanvasWorld>,
    camera: Camera,
    viewport: Option<(f32, f32)>,
) -> impl IntoElement {
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
        .child(controls::label("Overview"))
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let graph = world.graph_bounds;
                    let scale = (f32::from(bounds.size.width) / graph.size.width.max(1.))
                        .min(f32::from(bounds.size.height) / graph.size.height.max(1.));
                    let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                    for node in &world.nodes {
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(
                                    px(origin.0 + (node.x - graph.origin.x) * scale),
                                    px(origin.1 + (node.y - graph.origin.y) * scale),
                                ),
                                size: size(px(node.width() * scale), px(node.height() * scale)),
                            },
                            theme::soft(node.tint()),
                        ));
                    }
                    if let Some((width, height)) = viewport {
                        window.paint_quad(PaintQuad {
                            bounds: Bounds {
                                origin: point(
                                    px(origin.0
                                        + (-camera.pan_x / camera.zoom - graph.origin.x) * scale),
                                    px(origin.1
                                        + (-camera.pan_y / camera.zoom - graph.origin.y) * scale),
                                ),
                                size: size(
                                    px(width / camera.zoom * scale),
                                    px(height / camera.zoom * scale),
                                ),
                            },
                            corner_radii: 0f32.into(),
                            background: transparent_black().into(),
                            border_widths: 1f32.into(),
                            border_color: theme::TEXT_MUTED,
                            border_style: BorderStyle::default(),
                        });
                    }
                },
            )
            .w_full()
            .h(px(64.)),
        )
}
