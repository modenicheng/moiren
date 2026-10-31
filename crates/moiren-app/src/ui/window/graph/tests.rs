use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Modifiers, MouseButton, PinchEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, TouchPhase, VisualTestContext, point, px, size,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::{Camera, StudioShell, pages};
use crate::ui::UiState;
use crate::ui::window::test_harness::{
    finish_service, initial_projection, settle_panels, shell_with_state, spawn_service, test_shell,
    wait_for, wait_until,
};
use pages::graph_canvas::{CanvasFrame, CanvasWorld};

fn render(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find("graph-canvas-input").bounds()
    })
}

fn canvas_frame(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> CanvasFrame {
    let bounds = render(cx);
    view.update(cx, |shell, cx| {
        let world = CanvasWorld::positioned(&shell.state.read(cx).snapshot, &shell.node_positions);
        CanvasFrame::new(std::sync::Arc::new(world), shell.camera, bounds)
    })
}

fn node_position(frame: &CanvasFrame, index: usize) -> Point<Pixels> {
    let card = frame.nodes[index].card;
    frame.bounds.origin
        + point(
            px(card.origin.x + card.size.width / 2.),
            px(card.origin.y + 20. * frame.camera.zoom),
        )
}

fn wire_position(frame: &CanvasFrame, index: usize) -> Point<Pixels> {
    let geometry = &frame.wires[index];
    let middle = geometry.points.len() / 2;
    let midpoint = if geometry.points.len() == 2 {
        let [start, end] = geometry.points.as_slice() else {
            unreachable!()
        };
        ((start.0 + end.0) / 2., (start.1 + end.1) / 2.)
    } else {
        geometry.points[middle]
    };
    frame.bounds.origin + point(px(midpoint.0), px(midpoint.1))
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
}

fn wait_for_graph(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    predicate: impl Fn(&crate::service::GraphSnapshot) -> bool,
) {
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            predicate(&shell.state.read(cx).snapshot.graph)
        })
    });
    render(cx);
}

#[gpui_kit::test]
fn graph_port_drag_connects_real_topology_in_both_directions(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{AppRequest, GraphEdit};
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.camera = Camera {
            pan_x: 50.,
            pan_y: 90.,
            zoom: 0.89,
            adjusted: true,
        };
        cx.notify();
    });
    settle_panels(cx);
    for reverse in [false, true] {
        let (edge, source, destination) = view.update(cx, |shell, cx| {
            let edge = &shell.state.read(cx).snapshot.graph.graph.edges()[0];
            (edge.id(), edge.src_port(), edge.dst_port())
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::Disconnect(edge)))
            });
        });
        wait_for_graph(&view, cx, |graph| graph.graph.edges().len() == 2);
        if reverse {
            view.update(cx, |shell, cx| {
                shell.camera.adjusted = false;
                cx.notify();
            });
        }
        let frame = canvas_frame(&view, cx);
        let (source_index, destination_index, input_index) = view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph.graph;
            let source_node = graph
                .nodes()
                .iter()
                .find(|node| node.outputs().iter().any(|port| port.id() == source))
                .unwrap();
            let destination_node = graph
                .nodes()
                .iter()
                .find(|node| node.inputs().iter().any(|port| port.id() == destination))
                .unwrap();
            (
                frame
                    .world
                    .nodes
                    .iter()
                    .position(|node| node.id == format!("{:?}", source_node.id()))
                    .unwrap(),
                frame
                    .world
                    .nodes
                    .iter()
                    .position(|node| node.id == format!("{:?}", destination_node.id()))
                    .unwrap(),
                destination_node
                    .inputs()
                    .iter()
                    .position(|port| port.id() == destination)
                    .unwrap(),
            )
        });
        let position = |anchor: (f32, f32)| frame.bounds.origin + point(px(anchor.0), px(anchor.1));
        let output = position(frame.nodes[source_index].output.unwrap());
        let input = position(frame.nodes[destination_index].inputs[input_index]);
        let (start, end) = if reverse {
            (input, output)
        } else {
            (output, input)
        };
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        render(cx);
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        wait_for_graph(&view, cx, |graph| {
            graph
                .graph
                .edges()
                .iter()
                .any(|edge| edge.src_port() == source && edge.dst_port() == destination)
        });
        let after = canvas_frame(&view, cx);
        assert!(after.camera.adjusted);
        assert_close(after.camera.pan_x, frame.camera.pan_x);
        assert_close(after.camera.pan_y, frame.camera.pan_y);
        assert_close(after.camera.zoom, frame.camera.zoom);
        for before in &frame.world.nodes {
            let node = after
                .world
                .nodes
                .iter()
                .find(|node| node.id == before.id)
                .unwrap();
            assert_close(node.x, before.x);
            assert_close(node.y, before.y);
        }
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_backspace_removes_selected_node_and_its_edges(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let frame = canvas_frame(&view, cx);
    let node = frame.world.nodes[1].id.clone();
    cx.simulate_click(node_position(&frame, 1), Modifiers::default());
    render(cx);
    cx.simulate_keystrokes("backspace");
    wait_for_graph(&view, cx, |graph| graph.nodes.len() == 3);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        assert!(
            graph
                .nodes()
                .iter()
                .all(|id| format!("{:?}", id.id()) != node)
        );
        assert!(graph.edges().iter().all(|edge| {
            format!("{:?}", edge.src()) != node && format!("{:?}", edge.dst()) != node
        }));
        assert!(shell.selected_node.is_none());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_delete_keeps_selection_when_the_service_cannot_queue_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let frame = canvas_frame(&view, cx);
    let selected = frame.world.nodes[1].node_id.expect("projected node id");
    cx.simulate_click(node_position(&frame, 1), Modifiers::default());
    render(cx);
    finish_service(runtime);
    cx.simulate_keystrokes("backspace");
    view.update(cx, |shell, cx| {
        assert_eq!(shell.selected_node, Some(selected));
        assert_eq!(shell.state.read(cx).snapshot.graph.nodes.len(), 4);
        assert!(shell.state.read(cx).dispatch_error.is_some());
    });
}

#[gpui_kit::test]
fn graph_backspace_in_node_name_edits_text_without_removing_node(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let frame = canvas_frame(&view, cx);
    cx.simulate_click(node_position(&frame, 1), Modifiers::default());
    render(cx);
    // Enter the inline rename mode by clicking the name display; the editor
    // replaces the heading in place.
    cx.update(|window, cx| window.click("node-name-display", cx));
    render(cx);
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            shell
                .node_name
                .update(cx, |input, cx| input.set_value("Rename me", window, cx));
        });
        window.click("node-name", cx);
    });
    cx.simulate_keystrokes("ctrl-a backspace");
    view.update(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).snapshot.graph.nodes.len(), 4);
        assert!(shell.selected_node.is_some());
        assert_eq!(shell.node_name.read(cx).value().as_str(), "");
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_port_drag_cancel_and_invalid_targets_preserve_topology(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let frame = canvas_frame(&view, cx);
    let position = |anchor: (f32, f32)| frame.bounds.origin + point(px(anchor.0), px(anchor.1));
    let output = position(frame.nodes[0].output.unwrap());
    let other_output = position(frame.nodes[1].output.unwrap());
    let background = frame.bounds.origin + point(px(80.), px(20.));
    for target in [
        other_output,
        background,
        frame.bounds.origin - point(px(10.), px(10.)),
    ] {
        cx.simulate_mouse_down(output, MouseButton::Left, Modifiers::default());
        view.update(cx, |shell, _| assert!(shell.connection_drag.is_some()));
        cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
        render(cx);
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
        view.update(cx, |shell, cx| {
            assert!(shell.connection_drag.is_none());
            assert!(!shell.state.read(cx).pending_request);
            assert_eq!(shell.state.read(cx).snapshot.graph.graph.edges().len(), 3);
        });
    }
    cx.simulate_mouse_down(output, MouseButton::Left, Modifiers::default());
    render(cx);
    cx.simulate_keystrokes("escape");
    cx.simulate_mouse_up(
        position(frame.nodes[1].inputs[0]),
        MouseButton::Left,
        Modifiers::default(),
    );
    view.update(cx, |shell, cx| {
        assert!(shell.connection_drag.is_none());
        assert!(!shell.state.read(cx).pending_request);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_delete_disconnects_selected_wire_without_removing_nodes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let frame = canvas_frame(&view, cx);
    cx.simulate_click(wire_position(&frame, 0), Modifiers::default());
    render(cx);
    view.update(cx, |shell, _| {
        assert!(
            shell.selected_route.is_some(),
            "wire must be selected before deletion"
        );
    });
    cx.simulate_keystrokes("delete");
    wait_for_graph(&view, cx, |graph| graph.graph.edges().len() == 2);
    view.update(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).snapshot.graph.nodes.len(), 4);
        assert!(shell.selected_route.is_none());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn compressor_inspector_edits_only_its_selected_node_and_hides_on_other_selection(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{CompressorParameter, GraphEdit};
    use moiren_core::graph::NodeKind;
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Compressor,
                channels: 2,
            }))
        })
    });
    // Both waits below share the original single 3s budget.
    let budget = Instant::now() + Duration::from_secs(3);
    let id = wait_for(budget.saturating_duration_since(Instant::now()), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .graph
                .nodes()
                .iter()
                .find(|node| node.kind() == NodeKind::Compressor)
                .map(|node| node.id())
        })
    });
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(id);
        shell.inspector_open = true;
        cx.notify();
    });
    render(cx);
    cx.update(|window, _| assert!(window.find("compressor-input-0").visible()));
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            let input = &shell.compressor.inputs[&(id, CompressorParameter::Threshold)];
            input.update(cx, |state, cx| {
                state.set_value("-30", window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    let remaining = budget.saturating_duration_since(Instant::now());
    wait_until(remaining, || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .compressor
                .threshold_db
                == -30.
        })
    });
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot.graph;
        assert!(
            snapshot
                .nodes
                .iter()
                .filter(|node| node.id != id)
                .all(|node| node.compressor == Default::default())
        );
        let gain = snapshot
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Gain)
            .unwrap()
            .id();
        shell.selected_node = Some(gain);
        cx.notify();
    });
    render(cx);
    cx.update(|window, _| assert!(window.try_find("compressor-input-0").is_none()));
    finish_service(runtime);
}

#[gpui_kit::test]
fn builtin_library_creation_opens_real_inspector_controls_and_updates_candidate_state(
    cx: &mut gpui_kit::TestAppContext,
) {
    use moiren_core::graph::NodeKind;

    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));

    cx.update(|window, cx| window.click("add-Parametric EQ", cx));
    let id = wait_for(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        render(cx);
        view.update(cx, |shell, cx| {
            let state = shell.state.read(cx);
            state
                .snapshot
                .graph
                .graph
                .nodes()
                .iter()
                .find(|node| node.kind() == NodeKind::Eq)
                .map(|node| node.id())
        })
    });

    cx.update(|window, _| assert!(window.find("eq-frequency-up").visible()));
    cx.update(|window, cx| window.click("eq-frequency-up", cx));
    wait_for_graph(&view, cx, |graph| {
        graph
            .node(id)
            .and_then(|node| node.builtin.eq())
            .is_some_and(|settings| settings.frequency_hz == 1_100.)
    });
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.active_revision, None);
        assert_eq!(
            graph.node(id).unwrap().builtin.eq().unwrap().frequency_hz,
            1_100.
        );
    });

    finish_service(runtime);
}

#[gpui_kit::test]
fn channel_mapper_inspector_edits_a_weighted_matrix_candidate(cx: &mut gpui_kit::TestAppContext) {
    use moiren_core::graph::NodeKind;

    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));

    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            shell
                .node_search
                .update(cx, |input, cx| input.set_value("channel", window, cx));
        });
    });
    render(cx);
    cx.update(|window, cx| window.click("add-Channel mapper", cx));
    let id = wait_for(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        render(cx);
        view.update(cx, |shell, cx| {
            let state = shell.state.read(cx);
            state
                .snapshot
                .graph
                .graph
                .nodes()
                .iter()
                .find(|node| node.kind() == NodeKind::ChannelMapper)
                .map(|node| node.id())
        })
    });

    cx.update(|window, _| {
        assert!(window.find("channel-map-edit-matrix").visible());
    });
    cx.update(|window, cx| window.click("channel-map-edit-matrix", cx));
    wait_for_graph(&view, cx, |graph| {
        graph
            .node(id)
            .and_then(|node| node.builtin.channel_mapper())
            .is_some_and(|settings| settings.matrix == vec![vec![1.0, 0.0], vec![0.0, 1.0]])
    });
    cx.update(|window, _| {
        assert!(
            window
                .try_find("channel-map-matrix-out-0-in-1-up")
                .is_some()
        );
    });
    cx.update(|window, cx| window.click("channel-map-matrix-out-0-in-1-up", cx));
    wait_for_graph(&view, cx, |graph| {
        graph
            .node(id)
            .and_then(|node| node.builtin.channel_mapper())
            .is_some_and(|settings| settings.matrix[0][1] == 0.25)
    });
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.active_revision, None);
        assert_eq!(
            graph
                .node(id)
                .unwrap()
                .builtin
                .channel_mapper()
                .unwrap()
                .matrix[0][1],
            0.25
        );
    });

    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_inspector_reports_observation_without_candidate_boundary_copy(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
    view.update(cx, |shell, cx| {
        let id = shell.state.read(cx).snapshot.graph.graph.nodes()[1].id();
        shell.selected_node = Some(id);
        cx.notify();
    });
    render(cx);
    cx.update(|window, _| {
        assert!(window.find("node-observation-status").visible());
        // Observation copy no longer narrates internal candidate/active state.
        assert!(window.try_find("node-observation-candidate").is_none());
    });

    finish_service(runtime);
}

#[gpui_kit::test]
fn selected_graph_inspector_node_keeps_its_level_topic_demanded(cx: &mut gpui_kit::TestAppContext) {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    let node = snapshot.graph.graph.nodes()[1].id();
    snapshot.phase = crate::service::SessionPhase::Running;
    Arc::make_mut(&mut snapshot.graph).active_revision = Some(snapshot.graph.candidate_revision);
    snapshot
        .signal_topics
        .node_levels
        .insert(node, snapshot.signal_topics.tone_output.clone());
    let topic = snapshot.signal_topics.node_levels[&node].clone();
    let state = cx.new(|_| UiState::new(handle.clone(), Arc::new(snapshot)));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(node);
        shell.node_meter_points.borrow_mut().clear();
        // Frame-driven pumping may already have created the subscription
        // for the canvas-visible node, so the change flag is not stable
        // here; the selected node must end up demanded either way.
        shell.update_node_meters(true, cx);
        assert_eq!(
            shell.state.read(cx).node_meters.status(node),
            crate::ui::monitoring::MeterStatus::NoPublisher,
            "selected node keeps its subscription polled"
        );
        cx.notify();
    });
    wait_until(Duration::from_secs(2), || topic.business_subscribers() != 0);
    view.update(cx, |shell, cx| {
        shell.selected_node = None;
        shell.node_meter_points.borrow_mut().clear();
        shell.update_node_meters(true, cx);
        assert!(shell.state.read(cx).node_meters.levels(node).is_none());
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
    });
    // The subscription survives a short invisibility gap (viewport flaps must
    // not reset the RT window), then demand releases; drive pumps while
    // waiting because the closure sees no display frames.
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.update_node_meters(true, cx);
        });
        topic.business_subscribers() == 0
    });

    finish_service(runtime);
}

#[gpui_kit::test]
fn node_library_native_drop_uses_camera_world_position_and_opens_only_node_properties(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
    view.update(cx, |shell, cx| {
        shell.camera = Camera {
            pan_x: 60.,
            pan_y: 35.,
            zoom: 0.89,
            adjusted: true,
        };
        cx.notify();
    });
    let frame = canvas_frame(&view, cx);
    let origin = cx.update(|window, _| {
        let bounds = window.find("library-Gain").bounds();
        bounds.origin + point(px(8.), px(12.))
    });
    let target = frame.bounds.origin + point(px(320.), px(240.));
    let expected = frame.camera.canvas_to_world((320., 240.));
    cx.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        origin + point(px(15.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    render(cx);
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    render(cx);
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        render(cx);
        view.update(cx, |shell, cx| {
            shell.state.read(cx).snapshot.graph.nodes.len() == 5
                && shell.pending_node_spawn.is_none()
        })
    });
    view.update(cx, |shell, cx| {
        let key = shell.selected_node.expect("created node is selected");
        let position = shell.node_positions[&key];
        assert_close(position.0, expected.0);
        assert_close(position.1, expected.1);
        assert!(shell.selected_route.is_none());
        assert_eq!(shell.state.read(cx).snapshot.graph.active_revision, None);
    });
    cx.update(|window, _| {
        assert!(window.find("node-name-display").visible());
        assert!(window.try_find("disconnect-route").is_none());
    });
    finish_service(runtime);
}

/// The rubber band is frame-driven: the canvas prepaint paints it and
/// applies its hits once per frame. With every meter at rest and no session
/// running, the meter demand is idle — the gesture itself must arm the
/// frame loop, or box select freezes until the next elapsed-time publish.
/// With signal playing, starting a box select must not degrade the card
/// meters to the elapsed-publication cadence: the drag arms frames, and the
/// hot layer keeps pumping the ballistics every frame while the gesture
/// holds.
#[gpui_kit::test]
fn graph_box_select_keeps_the_meters_pumping_during_the_drag(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = crate::ui::window::test_harness::shell_with_state(cx, state);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.compact = false;
        cx.notify();
    });
    render(cx);
    crate::ui::window::test_harness::settle_panels(cx);
    let frame = canvas_frame(&view, cx);
    let center = node_position(&frame, 0);
    // Signal is flowing: the loop runs on meter motion alone.
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        render(cx);
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.raw_levels(node).is_some()
        })
    });
    // Start the box select away from the card and hold it: the hot layer's
    // pump must keep advancing every frame while the gesture holds — the
    // pump is what updates the card meters' readings.
    let anchor = center + point(px(300.), px(200.));
    cx.simulate_mouse_down(anchor, MouseButton::Left, Modifiers::default());
    let cadence = |view: &Entity<StudioShell>, cx: &mut VisualTestContext| {
        view.read_with(cx, |shell, _| {
            let trace = shell.mixer.trace.borrow();
            (trace.graph_frames, trace.graph_meter_paints)
        })
    };
    let mut painted_rounds = 0;
    let mut previous = cadence(&view, cx);
    for step in 0..6 {
        cx.simulate_mouse_move(
            anchor + point(px(10. * (step as f32) + 10.), px(0.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        let (frames, paints) = cadence(&view, cx);
        if paints > previous.1 {
            painted_rounds += 1;
        }
        assert!(
            frames > previous.0,
            "the meter pump stalled during the box select (frame {step})"
        );
        previous = (frames, paints);
    }
    view.read_with(cx, |shell, _| {
        assert!(shell.selection_drag.is_some(), "the gesture must be held");
    });
    assert!(
        painted_rounds >= 5,
        "the card meters stopped painting during the box select: {painted_rounds}/6 rounds painted"
    );
    // A stale platform visibility flag (fully occluded, then a fast alt-tab)
    // must not blank the meter updates mid-gesture: the drag proves the
    // window is interactive.
    view.update(cx, |shell, cx| {
        shell.presentation.visible = false;
        shell.presentation.minimized = true;
        cx.notify();
    });
    let mut previous = cadence(&view, cx).0;
    let mut pumped_while_hidden = 0;
    for step in 0..4 {
        cx.simulate_mouse_move(
            anchor + point(px(80. + 10. * (step as f32)), px(0.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        let frames = cadence(&view, cx).0;
        if frames > previous {
            pumped_while_hidden += 1;
        }
        previous = frames;
    }
    view.read_with(cx, |shell, cx| {
        assert!(
            shell.selection_drag.is_some(),
            "the gesture must still hold"
        );
        assert!(
            shell.state.read(cx).node_meters.status(node)
                != crate::ui::monitoring::MeterStatus::Hidden,
            "the node meters must not drop their subscriptions mid-gesture"
        );
    });
    assert!(
        pumped_while_hidden >= 3,
        "the pump stalled while the visibility flag was stale: {pumped_while_hidden}/4 rounds"
    );
    cx.simulate_mouse_up(
        anchor + point(px(120.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_box_select_runs_frames_and_applies_hits_while_everything_rests(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.compact = false;
        cx.notify();
    });
    render(cx);
    // The sidebar/inspector close springs animate; let them finish before
    // measuring the at-rest frame loop.
    crate::ui::window::test_harness::settle_panels(cx);
    let frame = canvas_frame(&view, cx);
    let node = frame.world.nodes[0].node_id.expect("projected node id");
    let center = node_position(&frame, 0);
    // At rest: no session, meters parked — the loop must be standing down
    // (any spring tail from the initial layout dies within a few frames).
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
    });
    let mut rest_frames = 0;
    for _ in 0..6 {
        cx.update(|window, cx| {
            rest_frames = window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    cx.update(|window, cx| assert_eq!(window.simulate_next_frame(cx), 0));
    // Background press: the gesture arms the loop immediately.
    let anchor = center - point(px(120.), px(60.));
    cx.simulate_mouse_down(anchor, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| {
        assert!(
            window.simulate_next_frame(cx) > 0,
            "the box-select gesture must arm the frame loop"
        );
        window.draw(cx).clear(cx);
    });
    // Drag the band across the node: the prepaint applies the hits.
    let finish = center + point(px(120.), px(60.));
    cx.simulate_mouse_move(
        anchor + point(px(60.), px(30.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    cx.simulate_mouse_move(finish, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    view.update(cx, |shell, _| {
        assert!(
            shell.selected_nodes.contains(&node),
            "the band crossing the node must select it"
        );
    });
    // The selection highlight is cached-canvas content: every selection
    // change must rebuild the page within the gesture's frames — the tint
    // may not wait for the next publish.
    let renders = view.read_with(cx, |shell, _| shell.mixer.trace.borrow().graph_renders);
    // Shrink the band back to the anchor: the node leaves the selection.
    // The rebuild rides two effect passes (shell notify -> host notify ->
    // render), so give the cycle two frames.
    cx.simulate_mouse_move(
        anchor + point(px(8.), px(8.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    for _ in 0..2 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    view.read_with(cx, |shell, _| {
        assert!(
            shell.selected_nodes.is_empty(),
            "the shrunken band must deselect"
        );
        assert!(
            shell.mixer.trace.borrow().graph_renders > renders,
            "a selection change must rebuild the cached page immediately"
        );
    });
    // Release: the loop stands down again.
    cx.simulate_mouse_up(finish, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, _| assert!(shell.selection_drag.is_none()));
    for _ in 0..6 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    cx.update(|window, cx| {
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "the frame loop must stand down when the gesture ends"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn set_page_cancels_canvas_gestures_and_restores_the_poll_pump(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    cx.update(gpui_kit::init);
    let state = cx.new(|_| crate::ui::state::UiState::new(handle, snapshot));
    let (view, cx) = crate::ui::window::test_harness::shell_with_state(cx, state);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.compact = false;
        cx.notify();
    });
    render(cx);
    crate::ui::window::test_harness::settle_panels(cx);
    let frame = canvas_frame(&view, cx);
    let node = frame.world.nodes[0].node_id.expect("projected node id");
    let center = node_position(&frame, 0);
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
        });
        render(cx);
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.raw_levels(node).is_some()
        })
    });
    // Middle-button press on the canvas background: a pan grab in flight.
    let anchor = center + point(px(300.), px(200.));
    cx.simulate_mouse_down(anchor, MouseButton::Middle, Modifiers::default());
    view.read_with(cx, |shell, cx| {
        assert!(
            shell.pan_grab.is_some(),
            "the middle press must arm a pan grab"
        );
        assert!(shell.drag_active());
        assert!(
            shell.frames_wanted(cx),
            "the in-flight gesture must keep taking frames on the canvas page"
        );
    });
    // Switch pages while the middle button stays held (the real trigger: a
    // left click on a workspace tab during a pan). The grab can never see
    // its mouse-up on the new page, so the switch must cancel it.
    view.update(cx, |shell, cx| shell.set_page(pages::Page::Matrix, cx));
    view.read_with(cx, |shell, cx| {
        assert!(
            shell.pan_grab.is_none(),
            "the pan grab must not survive the page switch"
        );
        assert!(shell.selection_drag.is_none());
        assert!(shell.node_drag.is_none());
        assert!(shell.connection_drag.is_none());
        assert!(shell.background_click.is_none());
        assert!(!shell.drag_active());
        assert!(
            !shell.frames_wanted(cx),
            "the DRAG demand must be cleared along with the gesture"
        );
    });
    // Defense in depth: a grab that dodged every lifecycle hook must not
    // starve the control-side poll, the only meter pump the Matrix page has.
    view.update(cx, |shell, _| shell.pan_grab = Some((100., 100.)));
    view.read_with(cx, |shell, cx| {
        assert!(shell.drag_active());
        assert!(
            !shell.frames_wanted(cx),
            "a stale grab off the canvas page must not gate the poll pump"
        );
    });
    // The desktop poll skips its meter pump exactly while frames are wanted;
    // once the stale grab stops counting, one poll must advance the meters.
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            !shell.frames_wanted(cx) && shell.pump_meters(cx)
        })
    });
    // Back on the Graph page the leftover grab is cancelled by the same
    // path, and no rubber band state can resurface with it.
    view.update(cx, |shell, cx| shell.set_page(pages::Page::Graph, cx));
    view.read_with(cx, |shell, _| {
        assert!(shell.pan_grab.is_none());
        assert!(
            shell.selection_drag.is_none(),
            "no rubber band may survive the round trip"
        );
    });
    render(cx);
    finish_service(runtime);
}

#[gpui_kit::test]
fn window_deactivation_cancels_canvas_gestures(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.compact = false;
        cx.notify();
    });
    render(cx);
    let frame = canvas_frame(&view, cx);
    let anchor = node_position(&frame, 0) - point(px(120.), px(60.));
    cx.simulate_mouse_down(anchor, MouseButton::Middle, Modifiers::default());
    view.read_with(cx, |shell, _| assert!(shell.pan_grab.is_some()));
    // The test platform only tracks a foreground window after an explicit
    // activation; make this window active before blurring it.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    // Alt+Tab mid-pan: the mouse-up lands in the other window and the grab
    // would stay armed forever; deactivation must cancel it instead.
    cx.deactivate_window();
    view.read_with(cx, |shell, _| {
        assert!(
            shell.pan_grab.is_none() && !shell.drag_active(),
            "deactivation must cancel the in-flight pan grab"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_fractional_zoom_drag_hand_pan_and_fit_share_rendered_geometry(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.compact = false;
        shell.camera = Camera {
            pan_x: 120.,
            pan_y: 90.,
            zoom: 0.89,
            adjusted: true,
        };
        cx.notify();
    });
    // Closing the library opens the rail spring; canvas geometry must be
    // measured against settled panel widths.
    settle_panels(cx);
    let frame = canvas_frame(&view, cx);
    let id = frame.world.nodes[0].node_id.expect("projected node id");
    let start = node_position(&frame, 0);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    let finish = start + point(px(89.), px(-44.5));
    cx.simulate_mouse_move(finish, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(finish, MouseButton::Left, Modifiers::default());
    let dragged = canvas_frame(&view, cx);
    assert_close(dragged.world.nodes[0].x, frame.world.nodes[0].x + 100.);
    assert_close(dragged.world.nodes[0].y, frame.world.nodes[0].y - 50.);
    assert_close(
        dragged.wires[0].points[0].0,
        frame.wires[0].points[0].0 + 89.,
    );
    view.update(cx, |shell, _| {
        assert_eq!(shell.selected_node, Some(id));
        assert!(shell.node_drag.is_none());
        assert_eq!(shell.camera, frame.camera);
    });
    cx.update(|window, cx| window.click("tool-hand", cx));
    let start = node_position(&dragged, 0);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    let finish = start + point(px(-42.), px(25.));
    cx.simulate_mouse_move(finish, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(finish, MouseButton::Left, Modifiers::default());
    let panned = canvas_frame(&view, cx);
    assert_close(panned.camera.pan_x, frame.camera.pan_x - 42.);
    assert_close(panned.camera.pan_y, frame.camera.pan_y + 25.);
    assert_close(panned.world.nodes[0].x, dragged.world.nodes[0].x);
    let background = panned.bounds.origin + point(px(900.), px(420.));
    cx.simulate_mouse_down(background, MouseButton::Middle, Modifiers::default());
    let finish = background + point(px(18.), px(-12.));
    cx.simulate_mouse_move(finish, MouseButton::Middle, Modifiers::default());
    cx.simulate_mouse_up(finish, MouseButton::Middle, Modifiers::default());
    let middle_panned = canvas_frame(&view, cx);
    assert_close(middle_panned.camera.pan_x, panned.camera.pan_x + 18.);
    assert_close(middle_panned.camera.pan_y, panned.camera.pan_y - 12.);
    view.update(cx, |shell, _| assert!(shell.pan_grab.is_none()));
    cx.update(|window, cx| {
        window.click("tool-select", cx);
        window.click("zoom-fit", cx);
    });
    let fitted = canvas_frame(&view, cx);
    for layout in &fitted.nodes {
        assert!(fitted.viewport().contains(&layout.card.origin));
        assert!(fitted.viewport().contains(
            &(layout.card.origin + point(layout.card.size.width, layout.card.size.height))
        ));
    }
    let start = node_position(&fitted, 0);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, _| assert_eq!(shell.selected_node, Some(id)));
    cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_rubber_band_selects_then_delete_removes_the_whole_selection(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        let nodes = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .topological_order()
            .unwrap();
        for (index, id) in nodes.iter().enumerate() {
            shell
                .node_positions
                .insert(*id, (80. + index as f32 * 230., 60.));
        }
        shell.camera = Camera {
            pan_x: 0.,
            pan_y: 0.,
            zoom: 1.,
            adjusted: true,
        };
        shell.sidebar_open = false;
        shell.inspector_open = false;
        cx.notify();
    });
    settle_panels(cx);
    let frame = canvas_frame(&view, cx);
    // Band from empty background over the first two node cards only.
    let start = frame.bounds.origin + point(px(20.), px(10.));
    let end = frame.bounds.origin + point(px(500.), px(170.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    // The band selection is applied from the canvas prepaint each frame.
    render(cx);
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
        assert_eq!(shell.selected_nodes.len(), 2, "band covers two nodes");
        assert!(shell.selected_node.is_none(), "no primary from a band");
    });
    render(cx);
    cx.simulate_keystrokes("delete");
    wait_for_graph(&view, cx, |graph| graph.nodes.len() == 2);
    view.update(cx, |shell, _| {
        assert!(shell.selected_nodes.is_empty());
        assert!(shell.selected_node.is_none());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_curved_wire_click_after_fit_selects_real_edge_without_changing_topology(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        let nodes = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .topological_order()
            .unwrap();
        for (index, id) in nodes.iter().enumerate() {
            shell.node_positions.insert(
                *id,
                (
                    80. + index as f32 * 230.,
                    if index % 2 == 0 { 50. } else { 520. },
                ),
            );
        }
        shell.camera.adjusted = false;
        cx.notify();
    });
    settle_panels(cx);
    let fitted = canvas_frame(&view, cx);
    let position = wire_position(&fitted, 0);
    // First click selects; the second opens the inspector.
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_click(position, Modifiers::default());
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let edge = &state.snapshot.graph.graph.edges()[0];
        assert_eq!(
            shell.selected_route,
            Some((edge.src_port(), edge.dst_port()))
        );
        assert!(shell.selected_node.is_none());
        assert_eq!(state.snapshot.graph.graph.edges().len(), 3);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find("matrix-inspector").visible());
        assert!(window.find("disconnect-route").visible());
    });
    // The freshly opened panel is still springing open; let it settle before
    // clicking controls inside it.
    settle_panels(cx);
    cx.update(|window, cx| {
        window.click("route-up", cx);
    });
    wait_for_graph(&view, cx, |graph| graph.graph.edges()[0].params().gain > 1.);
    view.update(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).snapshot.graph.graph.edges().len(), 3);
        assert!(shell.selected_route.is_some());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_panning_preserves_node_or_wire_selection_and_inspector_visibility(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    for wire_selected in [false, true] {
        for inspector_open in [false, true] {
            for mode in 0..4 {
                let selection = view.update(cx, |shell, cx| {
                    let graph = &shell.state.read(cx).snapshot.graph.graph;
                    let node = graph.nodes()[1].id();
                    let edge = &graph.edges()[0];
                    let route = (edge.src_port(), edge.dst_port());
                    shell.selected_node = (!wire_selected).then_some(node);
                    shell.selected_nodes = shell.selected_node.into_iter().collect();
                    shell.selected_route = wire_selected.then_some(route);
                    shell.inspector_open = inspector_open;
                    shell.sidebar_open = false;
                    shell.hand_tool = mode >= 2;
                    shell.camera = Camera {
                        pan_x: 60.,
                        pan_y: 80.,
                        zoom: 0.75,
                        adjusted: true,
                    };
                    cx.notify();
                    (shell.selected_node, shell.selected_route)
                });
                let frame = canvas_frame(&view, cx);
                let start = match mode {
                    2 => node_position(&frame, 0),
                    3 => wire_position(&frame, 0),
                    _ => frame.bounds.origin + point(px(180.), frame.bounds.size.height - px(80.)),
                };
                let button = match mode {
                    // Background panning is right-button (Windows style);
                    // middle always pans; the hand tool pans with the left.
                    0 => MouseButton::Right,
                    1 => MouseButton::Middle,
                    _ => MouseButton::Left,
                };
                cx.simulate_mouse_down(start, button, Modifiers::default());
                view.update(cx, |shell, _| {
                    assert_eq!((shell.selected_node, shell.selected_route), selection);
                    assert_eq!(shell.inspector_open, inspector_open);
                });
                let end = start + point(px(28.), px(18.));
                cx.simulate_mouse_move(end, Some(button), Modifiers::default());
                cx.simulate_mouse_up(end, button, Modifiers::default());
                view.update(cx, |shell, _| {
                    assert_eq!((shell.selected_node, shell.selected_route), selection);
                    assert_eq!(shell.inspector_open, inspector_open);
                    assert_close(shell.camera.pan_x, 88.);
                    assert_close(shell.camera.pan_y, 98.);
                    assert!(shell.pan_grab.is_none());
                });
            }
        }
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_scroll_pinch_and_toolbar_zoom_preserve_their_canvas_anchors(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.camera = Camera {
            pan_x: 100.,
            pan_y: 80.,
            zoom: 0.89,
            adjusted: true,
        };
        cx.notify();
    });
    let before = canvas_frame(&view, cx);
    let local = (180., 180.);
    let position = before.bounds.origin + point(px(local.0), px(local.1));
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., 1.)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    let wheel = canvas_frame(&view, cx);
    let anchor = before.camera.canvas_to_world(local);
    assert_close(wheel.camera.zoom, 0.89 * 1.02);
    assert_close(wheel.camera.world_to_canvas(anchor).0, local.0);
    assert_close(wheel.camera.world_to_canvas(anchor).1, local.1);
    cx.simulate_event(PinchEvent {
        position,
        delta: 0.1,
        modifiers: Modifiers::default(),
        phase: TouchPhase::Moved,
    });
    let pinched = canvas_frame(&view, cx);
    assert_close(pinched.camera.zoom, wheel.camera.zoom * 1.1);
    assert_close(pinched.camera.world_to_canvas(anchor).0, local.0);
    assert_close(pinched.camera.world_to_canvas(anchor).1, local.1);
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(13.), px(-9.))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    let panned = canvas_frame(&view, cx);
    assert_close(panned.camera.pan_x, pinched.camera.pan_x + 13.);
    assert_close(panned.camera.pan_y, pinched.camera.pan_y - 9.);
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(60.))),
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
        touch_phase: TouchPhase::Moved,
    });
    let control_zoomed = canvas_frame(&view, cx);
    assert_close(control_zoomed.camera.zoom, panned.camera.zoom * 1.02);
    let control_anchor = panned.camera.canvas_to_world(local);
    assert_close(
        control_zoomed.camera.world_to_canvas(control_anchor).0,
        local.0,
    );
    assert_close(
        control_zoomed.camera.world_to_canvas(control_anchor).1,
        local.1,
    );
    let center = (
        control_zoomed.viewport().size.width / 2.,
        control_zoomed.viewport().size.height / 2.,
    );
    let center_world = control_zoomed.camera.canvas_to_world(center);
    cx.update(|window, cx| window.click("zoom-in", cx));
    let zoomed = canvas_frame(&view, cx);
    assert_close(zoomed.camera.world_to_canvas(center_world).0, center.0);
    assert_close(zoomed.camera.world_to_canvas(center_world).1, center.1);
    finish_service(runtime);
}

#[gpui_kit::test]
fn collapsed_library_rail_keeps_spawning_and_the_handle_stays_visible(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    let baseline = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.nodes.len()
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        // Regression: the handle used to paint under the canvas host.
        assert!(window.find("library-toggle").visible());
        // The detail panel is open, so the icon rail stays hidden.
        assert!(
            !window.find("library-rail").visible(),
            "rail must not show while the library is expanded"
        );
        window.click("library-toggle", cx);
    });
    view.update(cx, |shell, _| {
        assert!(!shell.sidebar_open, "handle collapses the detail sleeve");
    });
    settle_panels(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        // Collapsed, the rail replaces the detail panel entirely.
        assert!(window.find("library-rail").visible());
        // The rail rides the library edge with the handle beside it.
        let page = window.find("graph-page").bounds();
        let handle = window.find("library-toggle").bounds();
        assert_close(
            f32::from(handle.origin.x - page.origin.x),
            crate::ui::theme::LIBRARY_RAIL_WIDTH + 2.,
        );
        // Icons keep spawning nodes from the collapsed form.
        window.click("rail-Gain", cx);
    });
    wait_for_graph(&view, cx, |graph| graph.nodes.len() == baseline + 1);
    view.update(cx, |shell, _| {
        assert!(
            shell.selected_node.is_some(),
            "the spawned node is selected"
        );
        assert!(shell.inspector_open, "spawning opens the inspector");
    });
    finish_service(runtime);
}

/// Meter motion on the graph page must not rebuild the cached page content
/// or chrome: every drawn frame rebuilds the shell's root element, and that
/// rebuild must splice the cached graph page (projection, library, toolbar)
/// instead of re-projecting. The meter frame still pumps per frame, real
/// edits still invalidate the cache, and the loop stands down under the
/// settings overlay.
#[gpui_kit::test]
fn graph_meter_animation_does_not_rebuild_the_cached_page(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.update(|window, cx| {
        assert!(
            !window.is_a11y_active(),
            "this regression measures the accessibility-inactive cache path"
        );
        window.render_frame(cx);
    });
    // Live publications must arm the graph meter frame before steady-state
    // measurements begin.
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            shell.state.read(cx).node_meters.raw_levels(node).is_some()
        })
    });
    // The panels' opening springs animate against the shell; let them settle
    // before measuring so only the meter frame can request frames.
    std::thread::sleep(Duration::from_millis(900));
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    let before = view.read_with(cx, |shell, _| {
        let trace = shell.mixer.trace.borrow();
        (trace.graph_renders, trace.graph_frames)
    });
    for _ in 0..10 {
        cx.update(|window, cx| {
            assert!(window.simulate_next_frame(cx) > 0);
            window.draw(cx).clear(cx);
        });
    }
    let after = view.read_with(cx, |shell, _| {
        assert!(
            shell.node_meter_points.borrow().contains(&node),
            "graph meter demand must survive animation-only frames"
        );
        let trace = shell.mixer.trace.borrow();
        (trace.graph_renders, trace.graph_frames)
    });
    // Every drawn frame rebuilds the shell's root element (GPUI lays the
    // root out from scratch); the isolation contract is that this rebuild
    // splices the cached page content instead of re-projecting the graph.
    // The toolbar and status bar rebuild every frame by necessity: their
    // window drag regions must re-register their hitboxes per frame (the
    // splice does not replay window-control registrations).
    assert_eq!(
        after.0, before.0,
        "meter animation must not rebuild the cached graph page"
    );
    assert!(
        after.1 >= before.1 + 10,
        "the graph meter frame must actually pump per frame"
    );

    // A real state change still invalidates the cached page.
    view.update(cx, |shell, cx| {
        shell.selected_nodes.insert(node);
        shell.selected_node = Some(node);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        view.read_with(cx, |shell, _| shell.mixer.trace.borrow().graph_renders) > after.0,
        "a selection change must still re-render the graph page"
    );

    // The settings overlay must stop the meter frame loop.
    view.update(cx, |shell, cx| {
        shell.settings_open = true;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| {
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "settings must stop the graph meter frame loop"
        );
    });
    finish_service(runtime);
}

/// A cached view replays last frame's paint operations on cache hits, so the
/// bars the cached canvas painted stand still there. The meter frame's
/// overlay must repaint them from live ballistics on every animation-only
/// frame: its paint counter advances with the pump, the cached page stays
/// flat, and the display ballistics keep decaying into what it paints.
#[gpui_kit::test]
fn graph_meter_frame_repaints_node_bars_across_animation_only_frames(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::{
        AppRequest, InputSelection, ManagedDriver, RunLimit, ServiceRuntime, SessionPhase,
        SessionSpec,
    };
    use std::sync::atomic::Ordering;
    let backend = crate::service::test_backend::FakeBackend::default();
    backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
    let backend_state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "input".into(),
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }))
        .unwrap();
    let snapshot = wait_for(Duration::from_secs(3), || {
        handle
            .try_snapshot()
            .filter(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let node = snapshot.graph.graph.nodes()[0].id();
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(handle, snapshot));
    let (view, cx) = shell_with_state(cx, state);
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    wait_until(Duration::from_secs(3), || {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |shell, cx| {
            let state = shell.state.read(cx);
            state.node_meters.raw_levels(node).is_some()
                && state.node_meters.channel_levels(node).is_some()
        })
    });
    // The panels' opening springs animate against the shell; let them settle
    // before measuring so only the meter frame can request frames.
    std::thread::sleep(Duration::from_millis(900));
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
    // Stop the source so the measured fall is display ballistics, not fresh
    // publications resetting the bar upward.
    backend_state.pause_audio.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(20));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    let (bar_before, paints_before, renders_before) = view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let trace = shell.mixer.trace.borrow();
        (
            state.node_meters.channel_levels(node).unwrap()[0].bar,
            trace.graph_meter_paints,
            trace.graph_renders,
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    for _ in 0..10 {
        cx.update(|window, cx| {
            assert!(window.simulate_next_frame(cx) > 0);
            window.draw(cx).clear(cx);
        });
    }
    view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        let trace = shell.mixer.trace.borrow();
        assert!(
            trace.graph_meter_paints >= paints_before + 10,
            "the meter frame must repaint the node bars per animation frame"
        );
        assert_eq!(
            trace.graph_renders, renders_before,
            "bar motion must not rebuild the cached graph page"
        );
        assert!(
            state.node_meters.channel_levels(node).unwrap()[0].bar < bar_before,
            "the repainted bars must follow the decaying ballistics"
        );
    });
    finish_service(runtime);
}
