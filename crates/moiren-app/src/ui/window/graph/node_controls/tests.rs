use super::parameter::Parameter;
use crate::service::{AppRequest, AudioBackend, DeviceRow, GraphEdit, InputSelection};
use crate::ui::pages::graph_canvas::{CanvasFrame, CanvasWorld, HitTarget};
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell, wait_until};
use crate::ui::window::{Camera, StudioShell};
use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Entity, Focusable, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TouchPhase,
    VisualTestContext, point, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use std::time::Duration;

fn show_card(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) {
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.selected_node = Some(id);
        shell.selected_nodes.clear();
        shell.selected_nodes.insert(id);
        shell.node_positions.insert(id, (20., 20.));
        shell.camera = Camera {
            pan_x: 20.,
            pan_y: 65.,
            zoom: 1.,
            adjusted: true,
        };
        cx.notify();
    });
    settle_panels(cx);
}

fn poll_graph(
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
    cx.update(|window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn card_numeric_entry_updates_only_target_and_keeps_typing_shortcuts(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Gain)
            .unwrap()
            .0
            .id()
    });
    show_card(&view, cx, id);
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            let input = &shell.node_controls.inputs[&(id, Parameter::Level)];
            let focus = input.read(cx).focus_handle(cx);
            assert!(shell.typing_focus(&focus, cx));
            window.focus(&focus, cx);
            input.update(cx, |input, cx| {
                input.set_value("-6 dB", window, cx);
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    poll_graph(&view, cx, |graph| {
        (graph.node(id).unwrap().gain - 10f64.powf(-6. / 20.)).abs() < 1e-8
    });
    cx.simulate_keystrokes("ctrl-a backspace");
    view.update(cx, |shell, cx| {
        assert!(!shell.inspector_open);
        assert!(shell.state.read(cx).snapshot.graph.node(id).is_some());
        for node in &shell.state.read(cx).snapshot.graph.nodes {
            if node.id != id {
                assert_eq!(node.mixer.gain, 1.);
            }
        }
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn card_body_drag_does_not_move_node_and_invalid_values_are_not_committed(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Gain)
            .unwrap()
            .0
            .id()
    });
    show_card(&view, cx, id);
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-param-{id:?}-Level"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        position + point(px(30.), px(15.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        position + point(px(30.), px(15.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            assert_eq!(shell.node_positions[&id], (20., 20.));
            assert!(shell.node_drag.is_none());
            shell.node_controls.inputs[&(id, Parameter::Level)].update(cx, |input, cx| {
                input.set_value("999", window, cx);
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    view.update(cx, |shell, _| {
        assert!(
            shell
                .node_controls
                .errors
                .contains_key(&(id, Parameter::Level))
        )
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn physical_device_picker_binds_node_with_inspector_closed(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let id = view.update(cx, |shell, cx| {
        let id = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Physical {
                    endpoint_id: "old-input".into(),
                },
            }))
        });
        id
    });
    poll_graph(&view, cx, |graph| {
        matches!(
            graph.node(id).unwrap().source,
            Some(InputSelection::Physical { .. })
        )
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            std::sync::Arc::make_mut(&mut std::sync::Arc::make_mut(&mut state.snapshot).catalog)
                .inputs = vec![crate::service::DeviceRow {
                endpoint_id: "chosen-input".into(),
                name: "Studio interface".into(),
            }];
        })
    });
    show_card(&view, cx, id);
    let trigger = cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(format!("card-device-{id:?}")).bounds().center()
    });
    cx.simulate_click(trigger, Modifiers::default());
    let choice = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-resource-{id:?}-0"))
            .bounds()
            .center()
    });
    cx.simulate_click(choice, Modifiers::default());
    poll_graph(&view, cx, |graph| {
        graph.node(id).unwrap().source
            == Some(InputSelection::Physical {
                endpoint_id: "chosen-input".into(),
            })
    });
    view.update(cx, |shell, _| assert!(!shell.inspector_open));
    finish_service(runtime);
}

#[gpui_kit::test]
fn overview_focus_opens_readable_card_without_opening_inspector(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let id = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Gain)
            .unwrap()
            .0
            .id()
    });
    show_card(&view, cx, id);
    view.update(cx, |shell, cx| {
        shell.camera.zoom = 0.5;
        cx.notify();
    });
    let focus = cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(format!("card-param-{id:?}-Level"))
                .is_none()
        );
        window.find(format!("card-focus-{id:?}")).bounds().center()
    });
    cx.simulate_click(focus, Modifiers::default());
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("card-param-{id:?}-Level")).visible());
    });
    view.update(cx, |shell, _| {
        assert_eq!(shell.camera.zoom, 1.);
        assert!(!shell.inspector_open);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn rotary_and_eq_curve_drag_edit_dsp_without_moving_the_card(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Eq,
                channels: 2,
            }))
        })
    });
    poll_graph(&view, cx, |graph| {
        graph.node_of_kind(NodeKind::Eq).is_some()
    });
    let id = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Eq)
            .unwrap()
            .0
            .id()
    });
    show_card(&view, cx, id);
    let knob = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-knob-{id:?}-EqGain"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(knob, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        knob - point(px(0.), px(8.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(
        knob - point(px(0.), px(40.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        knob - point(px(0.), px(40.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    poll_graph(&view, cx, |graph| {
        graph.node(id).unwrap().builtin.eq().unwrap().gain_db > 0.
    });
    let curve = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-response-{id:?}"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(curve, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        curve + point(px(8.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(
        curve + point(px(60.), px(-10.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        curve + point(px(60.), px(-10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    poll_graph(&view, cx, |graph| {
        graph.node(id).unwrap().builtin.eq().unwrap().frequency_hz > 1000.
    });
    view.update(cx, |shell, _| {
        assert_eq!(shell.node_positions[&id], (20., 20.))
    });
    finish_service(runtime);
}

fn add_eq(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> NodeId {
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    poll_graph(view, cx, |graph| graph.node_of_kind(NodeKind::Eq).is_some());
    view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Eq)
            .unwrap()
            .0
            .id()
    })
}

#[gpui_kit::test]
fn untouched_numeric_blur_does_not_revert_a_focused_knob_edit(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let id = add_eq(&view, cx);
    show_card(&view, cx, id);
    measured_card_frame(&view, cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let input = view.update(cx, |shell, _| {
        shell.node_controls.inputs[&(id, Parameter::EqGain)].clone()
    });
    let old_text = cx.update(|window, cx| {
        window.focus(&input.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        input.read(cx).value().to_string()
    });
    let knob = cx.update(|window, _| {
        window
            .find(format!("card-knob-{id:?}-EqGain"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(knob, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        knob - point(px(0.), px(8.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(
        knob - point(px(0.), px(40.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        knob - point(px(0.), px(40.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    flush_card_requests(&view, cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(input.read(cx).value().as_str(), old_text);
    });
    let (expected_gain, revision) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let gain = graph.node(id).unwrap().builtin.eq().unwrap().gain_db;
        assert!(gain > 0., "the knob must change the original zero gain");
        (gain, graph.candidate_revision)
    });
    cx.update(|window, cx| {
        window.blur(cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    flush_card_requests(&view, cx);
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(
            graph.node(id).unwrap().builtin.eq().unwrap().gain_db,
            expected_gain,
            "blurring untouched numeric text must not restore the pre-knob value"
        );
        assert_eq!(graph.candidate_revision, revision);
        assert_eq!(
            input.read(cx).value().as_str(),
            format!("{expected_gain:.2}")
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn numeric_dirty_text_survives_blur_render_and_enter_marks_it_clean(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = add_eq(&view, cx);
    show_card(&view, cx, id);
    measured_card_frame(&view, cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let input = view.update(cx, |shell, _| {
        shell.node_controls.inputs[&(id, Parameter::EqGain)].clone()
    });
    cx.update(|window, cx| {
        window.focus(&input.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        input.update(cx, |input, cx| input.set_value("3.125", window, cx));
        window.render_frame(cx);
        window.blur(cx);
        // A render can run before the queued Blur subscription. Do not replace
        // the user's draft with the snapshot before that subscription reads it.
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        assert_eq!(input.read(cx).value().as_str(), "3.125");
        window.render_frame(cx);
    });
    cx.run_until_parked();
    flush_card_requests(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .builtin
                .eq()
                .unwrap()
                .gain_db,
            3.125
        );
    });
    cx.update(|window, cx| {
        window.focus(&input.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        input.update(cx, |input, cx| {
            input.set_value("4.125", window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    flush_card_requests(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.node(id).unwrap().builtin.eq().unwrap().gain_db, 4.125);
        let edit = Parameter::EqGain
            .edit(6.25, NodeKind::Eq, graph.node(id).unwrap())
            .unwrap();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(edit));
        });
    });
    flush_card_requests(&view, cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert_eq!(input.read(cx).value().as_str(), "4.125");
        window.blur(cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    flush_card_requests(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .builtin
                .eq()
                .unwrap()
                .gain_db,
            6.25,
            "Enter must mark text clean so a later blur cannot replay it"
        );
    });
    finish_service(runtime);
}

fn flush_card_requests(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    // An unchanged-value predicate could pass before the service consumes a bad edit.
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.poll();
                assert!(state.dispatch_error.is_none(), "{:?}", state.dispatch_error);
                !state.pending_request
            })
        })
    });
}

fn measured_card_frame(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> CanvasFrame {
    // Intrinsic height is published with cx.defer; separate updates let it reach layout.
    for _ in 0..4 {
        cx.update(|window, cx| window.render_frame(cx));
    }
    let bounds = cx.update(|window, _| window.find("graph-canvas-input").bounds());
    view.update(cx, |shell, cx| {
        let world = CanvasWorld::measured(
            &shell.state.read(cx).snapshot,
            &shell.node_positions,
            &shell.node_controls.heights,
        );
        CanvasFrame::new(std::sync::Arc::new(world), shell.camera, bounds)
    })
}

#[gpui_kit::test]
fn front_gain_header_occludes_rear_eq_knob_without_parameter_drift(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let eq = add_eq(&view, cx);
    let old_ids = view.update(cx, |shell, cx| {
        let ids = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .nodes
            .iter()
            .map(|node| node.id)
            .collect::<Vec<_>>();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Gain,
                channels: 2,
            }));
        });
        ids
    });
    poll_graph(&view, cx, |graph| {
        graph.nodes.iter().any(|node| !old_ids.contains(&node.id))
    });
    let gain = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let gain = graph
            .nodes
            .iter()
            .find(|node| !old_ids.contains(&node.id))
            .unwrap()
            .id;
        assert_eq!(graph.kind_of(gain), Some(NodeKind::Gain));
        for node in &graph.nodes {
            shell.node_positions.insert(node.id, (4000., 4000.));
        }
        gain
    });
    show_card(&view, cx, eq);
    let frame = measured_card_frame(&view, cx);
    let eq_index = frame
        .world
        .nodes
        .iter()
        .position(|node| node.node_id == Some(eq))
        .unwrap();
    let gain_index = frame
        .world
        .nodes
        .iter()
        .position(|node| node.node_id == Some(gain))
        .unwrap();
    assert!(eq_index < gain_index, "the Gain must paint after the EQ");
    let knob = cx.update(|window, _| window.find(format!("card-knob-{eq:?}-EqGain")).bounds());
    let local = frame.local_position((f32::from(knob.center().x), f32::from(knob.center().y)));
    // Place the front header over the actual native knob, using the same
    // projection as graph hit testing rather than an assumed body offset.
    let gain_position = frame.camera.canvas_to_world((
        local.0 - frame.nodes[gain_index].card.size.width / 2.,
        local.1 - 20. * frame.camera.zoom,
    ));
    view.update(cx, |shell, cx| {
        shell.node_positions.insert(gain, gain_position);
        cx.notify();
    });
    let frame = measured_card_frame(&view, cx);
    let card = frame.nodes[gain_index].card;
    let header = frame.bounds.origin
        + point(
            px(card.center().x),
            px(card.top() + 20. * frame.camera.zoom),
        );
    cx.update(|window, _| {
        assert!(
            window
                .find(format!("card-knob-{eq:?}-EqGain"))
                .bounds()
                .contains(&header)
        );
    });
    assert_eq!(
        frame.hit_test((f32::from(header.x), f32::from(header.y))),
        Some(HitTarget::Node(frame.world.nodes[gain_index].id.as_str()))
    );
    let (before_gain, before_knob, revision) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(shell.selected_node, Some(eq));
        (
            graph.node(eq).unwrap().builtin.eq().unwrap().gain_db,
            shell.node_controls.sliders[&(eq, Parameter::EqGain)]
                .state
                .read(cx)
                .value()
                .end(),
            graph.candidate_revision,
        )
    });
    cx.simulate_click(header, Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    view.update(cx, |shell, _| {
        assert_eq!(shell.selected_node, Some(gain));
        assert_eq!(shell.selected_nodes.len(), 1);
        assert!(shell.selected_nodes.contains(&gain));
        assert!(shell.node_drag.is_none());
        assert!(
            !shell.node_controls.sliders[&(eq, Parameter::EqGain)]
                .dragging
                .get()
        );
    });
    // Moving after the click must not revive a covered knob's drag state.
    cx.simulate_mouse_move(header - point(px(0.), px(40.)), None, Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    flush_card_requests(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(
            graph.node(eq).unwrap().builtin.eq().unwrap().gain_db,
            before_gain
        );
        assert_eq!(graph.candidate_revision, revision);
        let slot = &shell.node_controls.sliders[&(eq, Parameter::EqGain)];
        assert_eq!(slot.state.read(cx).value().end(), before_knob);
        assert!(!slot.dragging.get());
        assert_eq!(shell.node_positions[&eq], (20., 20.));
        assert_eq!(shell.node_positions[&gain], gain_position);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn overview_focus_pins_auto_layout_target_before_neighbor_measurement(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let eq = add_eq(&view, cx);
    show_card(&view, cx, eq);
    view.update(cx, |shell, cx| {
        shell.node_positions.clear();
        shell.node_controls.heights.clear();
        shell.camera.zoom = 0.5;
        cx.notify();
    });
    let before = measured_card_frame(&view, cx);
    let target = before
        .world
        .nodes
        .iter()
        .find(|node| node.node_id == Some(eq))
        .unwrap();
    let position = (target.x, target.y);
    let neighbor = before
        .world
        .nodes
        .iter()
        .find(|node| node.node_id != Some(eq) && node.x == target.x && node.y < target.y)
        .expect("the auto-layout target needs an earlier neighbor in its column")
        .node_id
        .unwrap();
    view.update(cx, |shell, _| {
        assert!(!shell.node_positions.contains_key(&eq));
    });
    let focus = cx.update(|window, _| window.find(format!("card-focus-{eq:?}")).bounds().center());
    cx.simulate_click(focus, Modifiers::default());
    view.update(cx, |shell, cx| {
        assert_eq!(shell.node_positions.get(&eq), Some(&position));
        // Simulate the late intrinsic measurement that originally moved the
        // target after the overview button had aimed the camera at its old y.
        shell.node_controls.heights.insert(neighbor, 900.);
        let world = CanvasWorld::measured(
            &shell.state.read(cx).snapshot,
            &shell.node_positions,
            &shell.node_controls.heights,
        );
        let target = world
            .nodes
            .iter()
            .find(|node| node.node_id == Some(eq))
            .unwrap();
        assert_eq!((target.x, target.y), position);
        assert_eq!(shell.selected_node, Some(eq));
        assert_eq!(shell.camera.zoom, 1.);
        assert!(!shell.inspector_open);
        cx.notify();
    });
    let after = measured_card_frame(&view, cx);
    let target_index = after
        .world
        .nodes
        .iter()
        .position(|node| node.node_id == Some(eq))
        .unwrap();
    let card = after.nodes[target_index].card;
    assert!((card.left() - 32.).abs() <= 1.);
    assert!((card.top() - 78.).abs() <= 1.);
    cx.update(|window, _| {
        let knob = window.find(format!("card-knob-{eq:?}-EqGain"));
        assert!(knob.visible());
        assert!(after.bounds.contains(&knob.bounds().center()));
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn input_and_eq_intrinsic_cards_enclose_native_bodies_and_share_hit_bounds(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let eq = add_eq(&view, cx);
    let input = view.update(cx, |shell, cx| {
        let id = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Physical {
                    endpoint_id: "geometry-input".into(),
                },
            }));
        });
        id
    });
    flush_card_requests(&view, cx);

    for id in [input, eq] {
        view.update(cx, |shell, cx| {
            // Keep another card from covering the lower-body hit-test probe.
            for node in &shell.state.read(cx).snapshot.graph.nodes {
                shell.node_positions.insert(node.id, (4000., 4000.));
            }
        });
        show_card(&view, cx, id);
        for zoom in [0.85, 1., 1.25] {
            view.update(cx, |shell, cx| {
                shell.camera.zoom = zoom;
                cx.notify();
            });
            let frame = measured_card_frame(&view, cx);
            let index = frame
                .world
                .nodes
                .iter()
                .position(|node| node.node_id == Some(id))
                .unwrap();
            let node = &frame.world.nodes[index];
            let card = frame.nodes[index].card;
            let (body, details, parameter, visualization) = cx.update(|window, _| {
                (
                    window.find(format!("node-controls-{id:?}")).bounds(),
                    window.find(format!("card-details-{id:?}")).bounds(),
                    window
                        .find(format!(
                            "card-param-{id:?}-{}",
                            if id == input { "Level" } else { "EqQ" }
                        ))
                        .bounds(),
                    window
                        .find(if id == input {
                            format!("card-meter-{id:?}")
                        } else {
                            format!("card-response-{id:?}")
                        })
                        .bounds(),
                )
            });
            let left = f32::from(frame.bounds.origin.x) + card.left();
            let top = f32::from(frame.bounds.origin.y) + card.top();
            let right = left + card.size.width;
            let bottom = top + card.size.height;
            let measured = node
                .measured_height
                .expect("native body must publish its intrinsic height");
            assert!(body.size.height > px(0.));
            assert!(
                measured * zoom + 1. >= f32::from(body.bottom()) - top,
                "{id:?} at zoom {zoom}: intrinsic height {measured} clips body {body:?}"
            );
            assert!((f32::from(body.left()) - left - 10. * zoom).abs() <= 1.);
            assert!((f32::from(body.right()) - right + 10. * zoom).abs() <= 1.);
            assert!((f32::from(body.top()) - top - 58. * zoom).abs() <= 1.);
            for actual in [body, details, parameter, visualization] {
                assert!(actual.size.width > px(0.) && actual.size.height > px(0.));
                assert!(f32::from(actual.left()) >= left - 1.);
                assert!(f32::from(actual.right()) <= right + 1.);
                assert!(f32::from(actual.top()) >= top - 1.);
                assert!(
                    f32::from(actual.bottom()) <= bottom + 1.,
                    "{id:?} at zoom {zoom}: child {actual:?} extends past card bottom {bottom}"
                );
                assert!(
                    actual.bottom() <= body.bottom() + px(1.),
                    "intrinsic body does not contain its actual child bounds"
                );
            }
            let probe = (f32::from(body.center().x), f32::from(body.bottom()) - 1.);
            assert_eq!(
                frame.hit_test(probe),
                Some(HitTarget::Node(node.id.as_str()))
            );
        }
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn rotary_release_outside_clears_drag_ownership_and_commits_final_value(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = add_eq(&view, cx);
    show_card(&view, cx, id);
    measured_card_frame(&view, cx);
    flush_card_requests(&view, cx);
    let knob = cx.update(|window, _| window.find(format!("card-knob-{id:?}-EqGain")).bounds());
    let start = knob.center();
    let outside = point(knob.right() + px(240.), start.y - px(50.));
    assert!(!knob.contains(&outside));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        start - point(px(0.), px(8.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(
        start - point(px(0.), px(20.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    let expected = view.update(cx, |shell, cx| {
        let slot = &shell.node_controls.sliders[&(id, Parameter::EqGain)];
        assert!(
            slot.dragging.get(),
            "mouse movement must acquire drag ownership"
        );
        let position = slot.state.read(cx).value().end();
        assert!(position > 0.5, "the gesture must change the default gain");
        Parameter::EqGain.value_at_position(
            position,
            shell.state.read(cx).snapshot.graph.node(id).unwrap(),
        )
    });
    // Drain intermediate edits first: release must dispatch its own final commit,
    // even if a slow machine already sent the last move through the throttle.
    flush_card_requests(&view, cx);
    cx.update(|window, cx| window.render_frame(cx));
    let before_release = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.consumed_request_id
    });
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, cx| {
        assert!(
            !shell.node_controls.sliders[&(id, Parameter::EqGain)]
                .dragging
                .get()
        );
        let state = shell.state.read(cx);
        assert!(
            state.pending_request || state.snapshot.consumed_request_id > before_release,
            "outside release must submit the final value, even if already acknowledged"
        );
        assert!(shell.node_drag.is_none());
        assert_eq!(shell.node_positions[&id], (20., 20.));
    });
    flush_card_requests(&view, cx);
    view.update(cx, |shell, cx| {
        let actual = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(id)
            .unwrap()
            .builtin
            .eq()
            .unwrap()
            .gain_db;
        assert!(
            (actual - expected).abs() < 1e-6,
            "final gain {actual} != {expected}"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn horizontal_wheel_over_eq_controls_changes_neither_q_nor_camera(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = add_eq(&view, cx);
    show_card(&view, cx, id);
    measured_card_frame(&view, cx);
    flush_card_requests(&view, cx);
    let (q, camera) = view.update(cx, |shell, cx| {
        (
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .builtin
                .eq()
                .unwrap()
                .q,
            (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
        )
    });
    let positions = cx.update(|window, _| {
        let row = window.find(format!("card-param-{id:?}-EqQ")).bounds();
        [
            window
                .find(format!("card-response-{id:?}"))
                .bounds()
                .center(),
            window
                .find(format!("card-knob-{id:?}-EqQ"))
                .bounds()
                .center(),
            point(row.center().x, row.top() + px(2.)),
        ]
    });
    for position in positions {
        for delta in [
            ScrollDelta::Pixels(point(px(24.), px(0.))),
            ScrollDelta::Pixels(point(px(-24.), px(-0.))),
            ScrollDelta::Lines(point(1., 0.)),
            ScrollDelta::Lines(point(-1., -0.)),
        ] {
            cx.simulate_mouse_move(position, None, Modifiers::default());
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta,
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
            flush_card_requests(&view, cx);
            view.update(cx, |shell, cx| {
                assert_eq!(
                    shell
                        .state
                        .read(cx)
                        .snapshot
                        .graph
                        .node(id)
                        .unwrap()
                        .builtin
                        .eq()
                        .unwrap()
                        .q,
                    q
                );
                assert_eq!(
                    (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
                    camera
                );
            });
        }
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn asio_input_and_output_card_selectors_exclude_wasapi_rows(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let (input, output) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        (
            graph.node_of_kind(NodeKind::Source).unwrap().0.id(),
            graph.node_of_kind(NodeKind::Sink).unwrap().0.id(),
        )
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                node: input,
                source: InputSelection::Physical {
                    endpoint_id: "old-input".into(),
                },
            }));
        })
    });
    flush_card_requests(&view, cx);
    for (id, choice) in [(input, 0), (output, 1)] {
        show_card(&view, cx, id);
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let snapshot = std::sync::Arc::make_mut(&mut state.snapshot);
                snapshot.audio_settings.backend = AudioBackend::Asio;
                let catalog = std::sync::Arc::make_mut(&mut snapshot.catalog);
                catalog.inputs = (0..3)
                    .map(|i| DeviceRow {
                        endpoint_id: format!("wasapi-input-{i}"),
                        name: format!("WASAPI input {i}"),
                    })
                    .collect();
                catalog.outputs = (0..3)
                    .map(|i| DeviceRow {
                        endpoint_id: format!("wasapi-output-{i}"),
                        name: format!("WASAPI output {i}"),
                    })
                    .collect();
                catalog.asio_drivers = (0..2)
                    .map(|i| DeviceRow {
                        endpoint_id: format!("asio-driver-{i}"),
                        name: format!("ASIO driver {i}"),
                    })
                    .collect();
            });
            cx.notify();
        });
        let trigger = cx.update(|window, cx| {
            window.render_frame(cx);
            window.find(format!("card-device-{id:?}")).bounds().center()
        });
        cx.simulate_click(trigger, Modifiers::default());
        let selected = cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.find(format!("card-resource-{id:?}-0")).visible());
            assert!(window.find(format!("card-resource-{id:?}-1")).visible());
            assert!(
                window.try_find(format!("card-resource-{id:?}-2")).is_none(),
                "ASIO selector must not include WASAPI-only rows"
            );
            window
                .find(format!("card-resource-{id:?}-{choice}"))
                .bounds()
                .center()
        });
        cx.simulate_click(selected, Modifiers::default());
        flush_card_requests(&view, cx);
        view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            let node = graph.node(id).unwrap();
            let expected = format!("asio-driver-{choice}");
            if id == input {
                assert_eq!(
                    node.source,
                    Some(InputSelection::Physical {
                        endpoint_id: expected
                    })
                );
            } else {
                assert_eq!(node.output_endpoint_id.as_deref(), Some(expected.as_str()));
            }
            assert!(!shell.inspector_open);
        });
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn recorder_card_selects_file_encoding_without_changing_graph_or_destination(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::RecordingFormat;
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Recorder,
                channels: 2,
            }));
        });
    });
    poll_graph(&view, cx, |graph| {
        graph.node_of_kind(NodeKind::Recorder).is_some()
    });
    let (id, graph) = view.update(cx, |shell, cx| {
        let graph = shell.state.read(cx).snapshot.graph.clone();
        (
            graph.node_of_kind(NodeKind::Recorder).unwrap().0.id(),
            graph,
        )
    });
    let path = std::env::temp_dir().join("moiren-format-ui-test.wav");
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::SetRecordingDestination {
                node: id,
                path: path.clone(),
            });
        });
    });
    flush_card_requests(&view, cx);
    show_card(&view, cx, id);
    for (key, format) in [
        ("pcm24", RecordingFormat::WavPcm24),
        ("pcm16", RecordingFormat::WavPcm16),
        ("float32", RecordingFormat::WavFloat32),
    ] {
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.find(format!("record-format-{id:?}-{key}")).visible());
            window.click(format!("record-format-{id:?}-{key}"), cx);
        });
        flush_card_requests(&view, cx);
        view.read_with(cx, |shell, cx| {
            let snapshot = &shell.state.read(cx).snapshot;
            let recording = snapshot
                .recordings
                .iter()
                .find(|recording| recording.node == id)
                .unwrap();
            assert_eq!(recording.format, format);
            assert_eq!(recording.path.as_ref(), Some(&path));
            assert_eq!(snapshot.graph, graph);
            assert!(!shell.inspector_open);
        });
    }
    // The rendered selection is not the latest queued intent. Returning to
    // float before service publication must not discard the second click.
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(format!("record-format-{id:?}-pcm24"), cx);
        window.click(format!("record-format-{id:?}-float32"), cx);
    });
    flush_card_requests(&view, cx);
    view.read_with(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .recordings
                .iter()
                .find(|r| r.node == id)
                .unwrap()
                .format,
            RecordingFormat::WavFloat32,
            "the last native click must win before the next service snapshot"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn recorder_card_format_controls_do_not_dispatch_while_busy(cx: &mut gpui_kit::TestAppContext) {
    use crate::service::{RecordingFormat, RecordingPhase};
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Recorder,
                channels: 2,
            }));
        });
    });
    poll_graph(&view, cx, |graph| {
        graph.node_of_kind(NodeKind::Recorder).is_some()
    });
    let id = view.read_with(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Recorder)
            .unwrap()
            .0
            .id()
    });
    show_card(&view, cx, id);
    for phase in [
        RecordingPhase::Starting,
        RecordingPhase::Recording,
        RecordingPhase::Stopping,
    ] {
        // Only render the service phase here; the managed-service tests prove
        // the same guard at the actual file-ownership boundary.
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let snapshot = std::sync::Arc::make_mut(&mut state.snapshot);
                let recording = snapshot
                    .recordings
                    .iter_mut()
                    .find(|recording| recording.node == id)
                    .unwrap();
                recording.phase = phase;
                recording.format = RecordingFormat::WavFloat32;
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click(format!("record-format-{id:?}-pcm24"), cx);
        });
        view.read_with(cx, |shell, cx| {
            assert!(
                !shell.state.read(cx).pending_request,
                "busy format button dispatched in {phase:?}"
            );
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .recordings
                    .iter()
                    .find(|recording| recording.node == id)
                    .unwrap()
                    .format,
                RecordingFormat::WavFloat32
            );
        });
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_card_selects_a_stage_editor_without_opening_inspector_or_editing_graph(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::service::BuiltinState;
    use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Rack,
                channels: 2,
            }))
        });
    });
    poll_graph(&view, cx, |graph| {
        graph.node_of_kind(NodeKind::Rack).is_some()
    });
    let id = view.read_with(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Rack)
            .unwrap()
            .0
            .id()
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                node: id,
                state: BuiltinState::Rack(RackSettings {
                    slots: vec![
                        RackSlot {
                            id: 4,
                            bypass: false,
                            processor: RackProcessor::Gain { initial: 1. },
                        },
                        RackSlot {
                            id: 8,
                            bypass: false,
                            processor: RackProcessor::Eq(Default::default()),
                        },
                    ],
                    routes: vec![],
                }),
            }))
        });
    });
    flush_card_requests(&view, cx);
    show_card(&view, cx, id);
    let before = view.read_with(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("rack-Card-{id:?}-editor-4")).visible());
        window.click(format!("rack-Card-{id:?}-select-8"), cx);
        window.render_frame(cx);
        assert!(window.find(format!("rack-Card-{id:?}-editor-8")).visible());
        assert!(
            window
                .try_find(format!("rack-Card-{id:?}-editor-4"))
                .is_none()
        );
    });
    view.read_with(cx, |shell, cx| {
        assert!(!shell.inspector_open);
        assert_eq!(shell.state.read(cx).snapshot.graph, before);
        assert!(!shell.state.read(cx).pending_request);
    });
    finish_service(runtime);
}
