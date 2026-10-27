use super::super::parameter::Parameter;
use super::{
    Scope,
    model::{Field, Key, Surface},
    parameters,
};
use crate::service::{AppRequest, BuiltinState, GraphEdit};
use crate::ui::pages::graph_canvas::{CanvasFrame, CanvasWorld};
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell, wait_until};
use crate::ui::window::{Camera, StudioShell};
use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Bounds, Entity, Focusable, Modifiers, MouseButton, Pixels, Role, ScrollDelta, ScrollWheelEvent,
    TouchPhase, VisualTestContext, point, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
use std::{sync::Arc, time::Duration};

fn flush_requests(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
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

fn assert_contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>, label: &str) {
    let tolerance = px(0.5);
    assert!(
        inner.size.width > px(0.) && inner.size.height > px(0.),
        "{label}"
    );
    assert!(
        inner.left() >= outer.left() - tolerance
            && inner.right() <= outer.right() + tolerance
            && inner.top() >= outer.top() - tolerance
            && inner.bottom() <= outer.bottom() + tolerance,
        "{label}: actual native input {inner:?} escapes {outer:?}"
    );
}

fn add_rack(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    channels: usize,
    processors: Vec<RackProcessor>,
) -> NodeId {
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Rack,
                channels,
            }));
        });
    });
    flush_requests(view, cx);
    let node = view.update(cx, |shell, cx| {
        let node = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Rack)
            .unwrap()
            .0
            .id();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                node,
                state: BuiltinState::Rack(RackSettings {
                    slots: processors
                        .into_iter()
                        .enumerate()
                        .map(|(index, processor)| RackSlot {
                            id: index as u32 + 1,
                            bypass: false,
                            processor,
                        })
                        .collect(),
                    ..Default::default()
                }),
            }));
        });
        node
    });
    flush_requests(view, cx);
    node
}

fn render_frames(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, cx| window.render_frame(cx));
    }
}

fn show_rack(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    node: NodeId,
    inspector: bool,
) {
    cx.update(|window, _| {
        window.resize(size(px(1600.), px(1000.)));
        window.activate_window();
    });
    cx.run_until_parked();
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = inspector;
        shell.selected_node = Some(node);
        shell.selected_nodes.clear();
        shell.selected_nodes.insert(node);
        for other in &shell.state.read(cx).snapshot.graph.nodes {
            shell.node_positions.insert(other.id, (4000., 4000.));
        }
        shell.node_positions.insert(node, (20., 20.));
        shell.camera = Camera {
            pan_x: 20.,
            pan_y: 65.,
            zoom: 1.,
            adjusted: true,
        };
        cx.notify();
    });
    settle_panels(cx);
    render_frames(cx);
}

fn key(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    node: NodeId,
    surface: Surface,
    slot: u32,
    field: Field,
) -> Key {
    view.update(cx, |shell, cx| Key {
        surface,
        node,
        epoch: shell.state.read(cx).snapshot.graph.rack_epochs[&node],
        slot,
        field,
    })
}

fn rack_id(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    node: NodeId,
    surface: Surface,
    suffix: &str,
) -> String {
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        Scope {
            node,
            surface,
            epoch: graph.rack_epochs[&node],
            settings: graph.node(node).unwrap().builtin.rack().unwrap(),
            inputs: &[],
            outputs: &[],
        }
        .id(suffix)
    })
}

fn settings(view: &Entity<StudioShell>, cx: &mut VisualTestContext, node: NodeId) -> RackSettings {
    view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .unwrap()
            .builtin
            .rack()
            .unwrap()
            .clone()
    })
}

fn camera(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> (f32, f32, f32) {
    view.update(cx, |shell, _| {
        (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom)
    })
}

fn input_bounds(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    key: Key,
) -> Bounds<Pixels> {
    let id = rack_id(
        view,
        cx,
        key.node,
        key.surface,
        &format!("field-{}-{:?}", key.slot, key.field),
    );
    cx.update(|window, _| {
        let field = window.find(id);
        let inputs = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput)
                    && input.path().len() > field.path().len()
                    && input.path().starts_with(field.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].visible());
        inputs[0].bounds()
    })
}

fn click_at(cx: &mut VisualTestContext, position: gpui_kit::Point<Pixels>) {
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    render_frames(cx);
}

#[gpui_kit::test]
fn rack_native_invalid_input_keeps_delete_in_the_field(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(&view, cx, 2, vec![RackProcessor::Eq(Default::default())]);
    show_rack(&view, cx, node, true);
    let before = view.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    for surface in [Surface::Card, Surface::Inspector] {
        let key = key(
            &view,
            cx,
            node,
            surface,
            1,
            Field::Scalar(Parameter::EqGain),
        );
        let input = view.update(cx, |shell, _| shell.node_controls.rack.inputs[&key].clone());
        let position = input_bounds(&view, cx, key).center();
        click_at(cx, position);
        cx.update(|window, cx| {
            assert!(input.read(cx).focus_handle(cx).is_focused(window));
            view.update(cx, |shell, cx| {
                assert!(shell.typing_focus(&input.read(cx).focus_handle(cx), cx))
            });
            input.update(cx, |input, cx| {
                input.set_value("not-a-number", window, cx);
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
        render_frames(cx);
        view.update(cx, |shell, _| {
            assert!(shell.node_controls.rack.errors.contains_key(&key))
        });
        cx.simulate_keystrokes("ctrl-a delete");
        render_frames(cx);
        assert!(input.read_with(cx, |input, _| input.value().is_empty()));
        flush_requests(&view, cx);
        view.update(cx, |shell, cx| {
            assert_eq!(shell.state.read(cx).snapshot.graph, before);
            assert_eq!(shell.selected_node, Some(node));
        });
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_native_wheel_targets_one_scalar_and_never_moves_camera(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(
        &view,
        cx,
        2,
        vec![
            RackProcessor::Eq(Default::default()),
            RackProcessor::Gain { initial: 0.75 },
        ],
    );
    show_rack(&view, cx, node, false);
    let key = key(
        &view,
        cx,
        node,
        Surface::Card,
        1,
        Field::Scalar(Parameter::EqGain),
    );
    let knob_id = rack_id(
        &view,
        cx,
        node,
        Surface::Card,
        &format!("knob-1-{:?}", key.field),
    );
    let position = cx.update(|window, _| window.find(knob_id).bounds().center());
    let before_camera = camera(&view, cx);
    let before = settings(&view, cx, node);
    let before_graph = view.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    cx.simulate_mouse_move(position, None, Modifiers::default());
    for delta in [
        ScrollDelta::Pixels(point(px(24.), px(0.))),
        ScrollDelta::Lines(point(-1., 0.)),
    ] {
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta,
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        flush_requests(&view, cx);
        assert_eq!(settings(&view, cx, node), before);
        assert_eq!(camera(&view, cx), before_camera);
    }
    let expected = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let projection =
            parameters::Projection::new(graph.node(node).unwrap(), &before.slots[0].processor)
                .unwrap();
        let next = shell.node_controls.rack.sliders[&key]
            .state
            .read(cx)
            .value()
            .end()
            + 0.01;
        projection.value_at_position(key.field, next).unwrap()
    });
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., 1.)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    flush_requests(&view, cx);
    render_frames(cx);
    let mut expected_settings = before;
    let RackProcessor::Eq(eq) = &mut expected_settings.slots[0].processor else {
        panic!("EQ")
    };
    eq.gain_db = expected;
    assert_eq!(settings(&view, cx, node), expected_settings);
    assert_eq!(camera(&view, cx), before_camera);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.graph, before_graph.graph);
        assert_eq!(graph.rack_epochs, before_graph.rack_epochs);
        assert_eq!(
            graph.candidate_revision,
            before_graph.candidate_revision + 1
        );
        for other in before_graph.nodes.iter().filter(|other| other.id != node) {
            assert_eq!(graph.node(other.id), Some(other));
        }
        assert!(!shell.node_controls.rack.sliders[&key].dragging.get());
        assert!(shell.node_drag.is_none());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_native_outside_release_commits_final_value_and_clears_dragging(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(&view, cx, 2, vec![RackProcessor::Eq(Default::default())]);
    show_rack(&view, cx, node, false);
    let key = key(
        &view,
        cx,
        node,
        Surface::Card,
        1,
        Field::Scalar(Parameter::EqGain),
    );
    let knob_id = rack_id(
        &view,
        cx,
        node,
        Surface::Card,
        &format!("knob-1-{:?}", key.field),
    );
    let knob = cx.update(|window, _| window.find(knob_id).bounds());
    let start = knob.center();
    let outside = point(knob.right() + px(240.), start.y - px(50.));
    let before = settings(&view, cx, node);
    let before_camera = camera(&view, cx);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        start - point(px(0.), px(8.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    render_frames(cx);
    cx.simulate_mouse_move(
        start - point(px(0.), px(20.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    let expected = view.update(cx, |shell, cx| {
        let slider = &shell.node_controls.rack.sliders[&key];
        assert!(slider.dragging.get());
        let position = slider.state.read(cx).value().end();
        assert!(position > 0.5);
        let projection = parameters::Projection::new(
            shell.state.read(cx).snapshot.graph.node(node).unwrap(),
            &before.slots[0].processor,
        )
        .unwrap();
        projection.value_at_position(key.field, position).unwrap()
    });
    flush_requests(&view, cx);
    render_frames(cx);
    let consumed = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.consumed_request_id
    });
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, cx| {
        assert!(!shell.node_controls.rack.sliders[&key].dragging.get());
        let state = shell.state.read(cx);
        assert!(
            state.pending_request || state.snapshot.consumed_request_id > consumed,
            "outside release must dispatch the final commit"
        );
        assert!(shell.node_drag.is_none());
        assert_eq!(shell.node_positions[&node], (20., 20.));
    });
    flush_requests(&view, cx);
    let mut expected_settings = before;
    let RackProcessor::Eq(eq) = &mut expected_settings.slots[0].processor else {
        panic!("EQ")
    };
    eq.gain_db = expected;
    assert_eq!(settings(&view, cx, node), expected_settings);
    assert_eq!(camera(&view, cx), before_camera);
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_native_stage_selection_is_shared_but_inputs_remain_surface_local(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(
        &view,
        cx,
        2,
        vec![
            RackProcessor::Gain { initial: 1. },
            RackProcessor::Eq(Default::default()),
        ],
    );
    show_rack(&view, cx, node, true);
    let before = view.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    for surface in [Surface::Card, Surface::Inspector] {
        let selector = rack_id(&view, cx, node, surface, "select-2");
        cx.update(|window, cx| window.click(selector, cx));
        render_frames(cx);
        let card_key = key(
            &view,
            cx,
            node,
            Surface::Card,
            2,
            Field::Scalar(Parameter::EqGain),
        );
        let inspector_key = Key {
            surface: Surface::Inspector,
            ..card_key
        };
        view.update(cx, |shell, cx| {
            let state = &shell.node_controls.rack;
            assert_eq!(state.selected[&node], 2);
            let card = &state.inputs[&card_key];
            let inspector = &state.inputs[&inspector_key];
            assert_ne!(card.entity_id(), inspector.entity_id());
            assert!(shell.typing_focus(&card.read(cx).focus_handle(cx), cx));
            assert!(shell.typing_focus(&inspector.read(cx).focus_handle(cx), cx));
            assert!(
                state
                    .inputs
                    .keys()
                    .filter(|key| key.node == node)
                    .all(|key| key.slot == 2)
            );
            assert_eq!(shell.state.read(cx).snapshot.graph, before);
        });
        for surface in [Surface::Card, Surface::Inspector] {
            let editor = rack_id(&view, cx, node, surface, "editor-2");
            cx.update(|window, _| assert!(window.find(editor).visible()));
        }
        let other_surface = if surface == Surface::Card {
            Surface::Inspector
        } else {
            Surface::Card
        };
        let selector = rack_id(&view, cx, node, other_surface, "select-1");
        cx.update(|window, cx| window.click(selector, cx));
        render_frames(cx);
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_navigation_retains_invalid_hidden_draft_and_commits_correction(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(
        &view,
        cx,
        2,
        vec![
            RackProcessor::Eq(Default::default()),
            RackProcessor::Gain { initial: 1. },
        ],
    );
    show_rack(&view, cx, node, true);
    let draft_key = key(
        &view,
        cx,
        node,
        Surface::Inspector,
        1,
        Field::Scalar(Parameter::EqGain),
    );
    let clean_key = Key {
        surface: Surface::Card,
        ..draft_key
    };
    let input = view.update(cx, |shell, _| {
        shell.node_controls.rack.inputs[&draft_key].clone()
    });
    let position = input_bounds(&view, cx, draft_key).center();
    click_at(cx, position);
    cx.update(|window, cx| {
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
        input.update(cx, |input, cx| {
            input.set_value("invalid-stage-draft", window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    render_frames(cx);
    let (error, baseline) = view.update(cx, |shell, cx| {
        let state = &shell.node_controls.rack;
        let saved = (
            state.errors[&draft_key].clone(),
            state.baselines[&draft_key].clone(),
        );
        shell.inspector_open = false;
        cx.notify();
        saved
    });
    settle_panels(cx);
    let before = settings(&view, cx, node);
    let select_second = rack_id(&view, cx, node, Surface::Card, "select-2");
    cx.update(|window, cx| window.click(select_second, cx));
    render_frames(cx);
    flush_requests(&view, cx);
    view.update(cx, |shell, cx| {
        let state = &shell.node_controls.rack;
        assert_eq!(
            state.selected[&node], 2,
            "hidden Inspector errors must not block Card navigation"
        );
        assert!(
            state.inputs.contains_key(&draft_key),
            "navigation discarded the invalid outgoing draft"
        );
        assert_eq!(state.inputs[&draft_key].entity_id(), input.entity_id());
        assert_eq!(
            state.inputs[&draft_key].read(cx).value().as_str(),
            "invalid-stage-draft"
        );
        assert_eq!(state.errors[&draft_key], error);
        assert_eq!(state.baselines[&draft_key], baseline);
        assert!(
            !state.inputs.contains_key(&clean_key),
            "clean inactive input should be reclaimed"
        );
        assert!(!state.baselines.contains_key(&clean_key));
        assert!(!state.sliders.contains_key(&draft_key));
        assert!(!state.sliders.contains_key(&clean_key));
    });
    assert_eq!(settings(&view, cx, node), before);

    let select_first = rack_id(&view, cx, node, Surface::Card, "select-1");
    cx.update(|window, cx| window.click(select_first, cx));
    render_frames(cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell.node_controls.rack.inputs[&draft_key].entity_id(),
            input.entity_id()
        );
        assert_eq!(input.read(cx).value().as_str(), "invalid-stage-draft");
        assert_eq!(shell.node_controls.rack.errors[&draft_key], error);
    });
    cx.update(|window, cx| {
        // The Inspector is hidden: no Enter or focus loss can commit this
        // correction. Navigation must explicitly flush the outgoing draft.
        assert!(!input.read(cx).focus_handle(cx).is_focused(window));
        input.update(cx, |input, cx| input.set_value("-3.125", window, cx));
    });
    let select_second = rack_id(&view, cx, node, Surface::Card, "select-2");
    cx.update(|window, cx| window.click(select_second, cx));
    flush_requests(&view, cx);
    render_frames(cx);
    let mut expected = before;
    let RackProcessor::Eq(eq) = &mut expected.slots[0].processor else {
        panic!("EQ")
    };
    eq.gain_db = -3.125;
    assert_eq!(settings(&view, cx, node), expected);
    view.update(cx, |shell, _| {
        let state = &shell.node_controls.rack;
        assert_eq!(state.selected[&node], 2);
        assert!(
            !state.inputs.contains_key(&draft_key),
            "committed inactive drafts must be reclaimed"
        );
        assert!(!state.errors.contains_key(&draft_key));
        assert!(!state.baselines.contains_key(&draft_key));
        assert!(!state.sliders.contains_key(&draft_key));
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_navigation_retains_paged_draft_and_reclaims_replaced_epoch(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(
        &view,
        cx,
        8,
        vec![RackProcessor::ChannelMapper(Default::default())],
    );
    show_rack(&view, cx, node, false);
    let draft_key = key(
        &view,
        cx,
        node,
        Surface::Card,
        1,
        Field::Weight {
            input: 0,
            output: 0,
        },
    );
    let clean_key = Key {
        field: Field::Weight {
            input: 1,
            output: 0,
        },
        ..draft_key
    };
    let input = view.update(cx, |shell, _| {
        shell.node_controls.rack.inputs[&draft_key].clone()
    });
    let cell = rack_id(&view, cx, node, Surface::Card, "weight-1-0-0");
    cx.update(|window, cx| {
        window.click(cell, cx);
        input.update(cx, |input, cx| {
            input.set_value("invalid-weight", window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    render_frames(cx);
    let before = settings(&view, cx, node);
    let next = rack_id(&view, cx, node, Surface::Card, "page-true->");
    cx.update(|window, cx| window.click(next, cx));
    render_frames(cx);
    view.update(cx, |shell, cx| {
        let state = &shell.node_controls.rack;
        assert_eq!(state.mapper_pages[&(Surface::Card, node)], (0, 1));
        assert_eq!(state.mapper_pages[&(Surface::Inspector, node)], (0, 0));
        assert!(
            state.inputs.contains_key(&draft_key),
            "paging discarded the invalid outgoing weight"
        );
        assert_eq!(state.inputs[&draft_key].entity_id(), input.entity_id());
        assert_eq!(input.read(cx).value().as_str(), "invalid-weight");
        assert!(state.errors.contains_key(&draft_key));
        assert!(!state.inputs.contains_key(&clean_key));
        assert!(!state.baselines.contains_key(&clean_key));
        assert_eq!(
            state
                .inputs
                .keys()
                .filter(|key| key.node == node && key.surface == Surface::Card)
                .count(),
            17
        );
        assert!(state.sliders.keys().all(|key| key.node != node));
    });
    let previous = rack_id(&view, cx, node, Surface::Card, "page-true-<");
    cx.update(|window, cx| window.click(previous, cx));
    render_frames(cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell.node_controls.rack.inputs[&draft_key].entity_id(),
            input.entity_id()
        );
        assert_eq!(input.read(cx).value().as_str(), "invalid-weight");
        assert!(shell.node_controls.rack.errors.contains_key(&draft_key));
    });
    let next = rack_id(&view, cx, node, Surface::Card, "page-true->");
    cx.update(|window, cx| window.click(next, cx));
    render_frames(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                node,
                state: BuiltinState::Rack(before.clone()),
            }));
        });
    });
    flush_requests(&view, cx);
    render_frames(cx);
    view.update(cx, |shell, cx| {
        assert_ne!(
            shell.state.read(cx).snapshot.graph.rack_epochs[&node],
            draft_key.epoch
        );
        let state = &shell.node_controls.rack;
        assert!(
            state
                .inputs
                .keys()
                .all(|key| key.node != node || key.epoch != draft_key.epoch)
        );
        assert!(!state.errors.contains_key(&draft_key));
        assert!(!state.baselines.contains_key(&draft_key));
        assert!(!state.sliders.contains_key(&draft_key));
    });
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.set_value("-2.5", window, cx);
            cx.emit(InputEvent::Blur);
        })
    });
    flush_requests(&view, cx);
    assert_eq!(
        settings(&view, cx, node),
        before,
        "detached stale-epoch Blur must not edit the replacement"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_card_dial_inputs_are_centered_on_knobs_and_contained(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Rack,
                channels: 2,
            }));
        });
    });
    flush_requests(&view, cx);
    let processors = [
        RackProcessor::Gain { initial: 1. },
        RackProcessor::Pan {
            initial: 0.,
            mono: false,
            swap: false,
        },
        RackProcessor::Eq(Default::default()),
        RackProcessor::Compressor(Default::default()),
        RackProcessor::Gate(Default::default()),
        RackProcessor::Limiter(Default::default()),
        RackProcessor::Delay(Default::default()),
    ];
    let node = view.update(cx, |shell, cx| {
        let node = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Rack)
            .unwrap()
            .0
            .id();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                node,
                state: BuiltinState::Rack(RackSettings {
                    slots: processors
                        .into_iter()
                        .enumerate()
                        .map(|(index, processor)| RackSlot {
                            id: index as u32 + 1,
                            bypass: false,
                            processor,
                        })
                        .collect(),
                    ..Default::default()
                }),
            }));
        });
        node
    });
    flush_requests(&view, cx);
    cx.update(|window, _| window.resize(size(px(1440.), px(900.))));
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.selected_node = Some(node);
        shell.selected_nodes.clear();
        shell.selected_nodes.insert(node);
        for other in &shell.state.read(cx).snapshot.graph.nodes {
            shell.node_positions.insert(other.id, (4000., 4000.));
        }
        shell.node_positions.insert(node, (20., 20.));
        shell.camera = Camera {
            pan_x: 20.,
            pan_y: 65.,
            zoom: 1.,
            adjusted: true,
        };
        cx.notify();
    });
    settle_panels(cx);

    for slot_id in 1..=7 {
        for zoom in [0.85, 1., 1.25] {
            cx.update(|window, cx| {
                view.update(cx, |shell, cx| {
                    shell.camera.zoom = zoom;
                    shell.node_controls.rack.selected.insert(node, slot_id);
                    super::sync(shell, window, cx);
                    cx.notify();
                });
            });
            // Native card height is published by a deferred layout callback.
            for _ in 0..4 {
                cx.update(|window, cx| window.render_frame(cx));
            }
            cx.update(|window, cx| {
                let (card, ids) = view.update(cx, |shell, cx| {
                    let snapshot = &shell.state.read(cx).snapshot;
                    let owner = snapshot.graph.node(node).unwrap();
                    let settings = owner.builtin.rack().unwrap();
                    let slot = settings.slots.iter().find(|slot| slot.id == slot_id).unwrap();
                    let scope = Scope {
                        node,
                        epoch: snapshot.graph.rack_epochs[&node],
                        surface: Surface::Card,
                        settings,
                        inputs: &[2],
                        outputs: &[2],
                    };
                    let (kind, leaf) = parameters::stage_node(owner, slot);
                    let ids = parameters::fields(kind, &leaf, Surface::Card).into_iter()
                        .map(|field| (
                            scope.id(&format!("field-{slot_id}-{field:?}")),
                            scope.id(&format!("knob-{slot_id}-{field:?}")),
                        )).collect::<Vec<_>>();
                    let world = CanvasWorld::measured(snapshot, &shell.node_positions, &shell.node_controls.heights);
                    let index = world.nodes.iter().position(|entry| entry.node_id == Some(node)).unwrap();
                    let frame = CanvasFrame::new(Arc::new(world), shell.camera, window.find("graph-canvas-input").bounds());
                    let card = frame.nodes[index].card;
                    (Bounds::new(
                        frame.bounds.origin + point(px(card.left()), px(card.top())),
                        size(px(card.size.width), px(card.size.height)),
                    ), ids)
                });
                assert!(!ids.is_empty());
                let observed = gpui_kit::base::test_support::snapshots(window);
                let body = window.find(format!("node-controls-{node:?}")).bounds();
                for (field_id, knob_id) in ids {
                    let field = window.find(field_id.clone());
                    let knob = window.find(knob_id).bounds();
                    // Measure the real Input, not its 64px wrapper: the wrong
                    // inherent Input::h also overflowed this wrapper vertically.
                    let inputs = observed.iter().filter(|element| {
                        element.role() == Some(Role::TextInput)
                            && element.path().len() > field.path().len()
                            && element.path().starts_with(field.path())
                    }).collect::<Vec<_>>();
                    assert_eq!(inputs.len(), 1, "{field_id}: expected one observed Input");
                    let input = inputs[0];
                    assert!(input.visible(), "{field_id}: native Input is clipped at zoom {zoom}");
                    let bounds = input.bounds();
                    assert!(
                        (f32::from(bounds.center().x) - f32::from(knob.center().x)).abs() <= 0.5,
                        "{field_id} at zoom {zoom}: native Input {bounds:?} is not centered on knob {knob:?}"
                    );
                    assert_contains(field.bounds(), bounds, &field_id);
                    assert_contains(body, bounds, &field_id);
                    assert_contains(card, bounds, &field_id);
                }
            });
        }
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn rack_native_rapid_double_clicks_toggle_bypass_mono_and_swap_twice(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let node = add_rack(
        &view,
        cx,
        2,
        vec![
            RackProcessor::Pan {
                initial: 0.375,
                mono: false,
                swap: false,
            },
            RackProcessor::Gain { initial: 0.75 },
        ],
    );
    show_rack(&view, cx, node, false);
    let before = settings(&view, cx, node);
    let before_camera = camera(&view, cx);
    let (epoch, consumed) = view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        (
            state.snapshot.graph.rack_epochs[&node],
            state.snapshot.consumed_request_id,
        )
    });
    let controls =
        ["bypass", "mono", "swap"].map(|name| rack_id(&view, cx, node, Surface::Card, name));
    cx.update(|window, cx| {
        // Keep all six native clicks inside one update, with no service poll.
        // Each click therefore sees the same rendered flags, not an ack.
        for control in controls {
            assert!(window.find(control.clone()).visible());
            window.double_click(control, cx);
            view.update(cx, |shell, cx| {
                let state = shell.state.read(cx);
                assert_eq!(state.snapshot.consumed_request_id, consumed);
                assert!(state.pending_request, "native toggle clicks must dispatch");
            });
        }
    });
    flush_requests(&view, cx);
    assert_eq!(
        settings(&view, cx, node),
        before,
        "two clicks must restore Bypass, Mono and Swap without changing Pan gain, another stage or routing"
    );
    assert_eq!(camera(&view, cx), before_camera);
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert!(state.snapshot.consumed_request_id.0 >= consumed.0 + 6);
        assert_eq!(state.snapshot.graph.rack_epochs[&node], epoch);
        assert_eq!(
            shell.node_controls.rack.selected_slot(
                node,
                state
                    .snapshot
                    .graph
                    .node(node)
                    .unwrap()
                    .builtin
                    .rack()
                    .unwrap()
            ),
            Some(1)
        );
    });
    finish_service(runtime);
}
