//! EQ-only acceptance over the real shell and deterministic service admission.
use super::{Parameter, StudioShell};
use crate::service::{
    AppHandle, AppRequest, GraphEdit, RackParameter, ServiceContext, ServiceMailbox,
};
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Focusable, Modifiers, MouseButton, Role, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, point, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::processor::EqSettings;

const FIELDS: [(Parameter, &str, f64); 3] = [
    (Parameter::EqFrequency, "1234.56 Hz", 1234.56),
    (Parameter::EqGain, "-7.25 dB", -7.25),
    (Parameter::EqQ, "1.27 Q", 1.27),
];

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        while let Some(request) = self.mailbox.try_request() {
            match request {
                AppRequest::EditGraph(edit) => {
                    self.context.graph.edit(edit).unwrap();
                }
                AppRequest::LoadProject(document) => {
                    self.context.load_project(document).unwrap();
                }
                AppRequest::RefreshCatalog => {}
                other => panic!("unexpected stopped EQ request: {other:?}"),
            }
        }
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.sync_node_controls(window, cx);
            });
            window.render_frame(cx);
        });
        cx.run_until_parked();
    }
}

fn fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<StudioShell>,
    &mut VisualTestContext,
    ManualService,
    NodeId,
) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    let id = context
        .graph
        .snapshot()
        .node_of_kind(NodeKind::Eq)
        .unwrap()
        .0
        .id();
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let state = cx.new(|_| UiState::new(handle.clone(), handle.try_snapshot().unwrap()));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, cx| {
        window.resize(size(px(1600.), px(1000.)));
        window.activate_window();
        window.render_frame(cx);
    });
    view.update(cx, |shell, cx| {
        for node in &shell.state.read(cx).snapshot.graph.nodes {
            shell.node_positions.insert(node.id, (4000., 4000.));
        }
        shell.node_positions.insert(id, (20., 20.));
        shell.camera = Camera {
            pan_x: 20.,
            pan_y: 65.,
            zoom: 1.,
            adjusted: true,
        };
        shell.settings_open = false;
        shell.sidebar_open = false;
        shell.inspector_open = true;
        shell.selected_node = Some(id);
        shell.selected_nodes.clear();
        shell.selected_nodes.insert(id);
        cx.notify();
    });
    settle_panels(cx);
    (
        view,
        cx,
        ManualService {
            context,
            handle,
            mailbox,
        },
        id,
    )
}

fn settings(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> EqSettings {
    view.read_with(cx, |shell, cx| {
        *shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(id)
            .unwrap()
            .builtin
            .eq()
            .unwrap()
    })
}

fn enter(input: &Entity<InputState>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.focus(&input.read(cx).focus_handle(cx), cx);
        input.update(cx, |input, cx| {
            input.set_value(text.to_owned(), window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    cx.run_until_parked();
}

fn inspector_input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: Parameter,
) -> Entity<InputState> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(format!("inspector-precision-{id:?}-{parameter:?}"));
        assert!(root.visible());
        let native = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(native.len(), 1);
        assert!(native[0].visible());
        assert!(native[0].bounds().left() >= root.bounds().left());
        assert!(native[0].bounds().right() <= root.bounds().right());
    });
    view.read_with(cx, |shell, _| {
        shell.node_controls.precision.inputs[&(id, parameter)].clone()
    })
}

#[test]
fn eq_precision_parameter_edits_use_independent_service_fields() {
    let mut graph = crate::service::ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    let snapshot = graph.snapshot();
    let (topology, node) = snapshot.node_of_kind(NodeKind::Eq).unwrap();
    for ((parameter, _, value), expected) in FIELDS.into_iter().zip([
        RackParameter::EqFrequency,
        RackParameter::EqGain,
        RackParameter::EqQ,
    ]) {
        let edit = parameter
            .edit_precision(value, NodeKind::Eq, node)
            .expect("valid EQ precision value");
        assert_eq!(
            edit,
            GraphEdit::SetBuiltinParameter {
                node: topology.id(),
                parameter: expected,
                value
            },
            "one field must not carry stale replacement settings"
        );
        graph.edit(edit).unwrap();
    }
    assert_eq!(
        *graph
            .snapshot()
            .node(node.id)
            .unwrap()
            .builtin
            .eq()
            .unwrap(),
        EqSettings {
            frequency_hz: 1234.56,
            gain_db: -7.25,
            q: 1.27
        }
    );
}

#[gpui_kit::test]
fn eq_precision_inspector_exact_fields_have_separate_entities_and_native_focus(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let initial = settings(&view, cx, id);
    let mut expected = initial;
    for (parameter, text, value) in FIELDS {
        let input = inspector_input(&view, cx, id, parameter);
        view.read_with(cx, |shell, cx| {
            let card = &shell.node_controls.inputs[&(id, parameter)];
            assert_ne!(card.entity_id(), input.entity_id());
            assert!(shell.typing_focus(&input.read(cx).focus_handle(cx), cx));
            assert!(shell.typing_focus(&card.read(cx).focus_handle(cx), cx));
        });
        let position = cx.update(|window, _| {
            window
                .find(format!("inspector-precision-input-{id:?}-{parameter:?}"))
                .bounds()
                .center()
        });
        cx.simulate_click(position, Modifiers::default());
        cx.update(|window, cx| assert!(input.read(cx).focus_handle(cx).is_focused(window)));
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(text);
        cx.simulate_keystrokes("enter");
        service.drain(&view, cx);
        match parameter {
            Parameter::EqFrequency => expected.frequency_hz = value,
            Parameter::EqGain => expected.gain_db = value,
            Parameter::EqQ => expected.q = value,
            _ => unreachable!(),
        }
        assert_eq!(settings(&view, cx, id), expected);
    }
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("eq-precision-{id:?}")).visible());
        assert!(window.try_find("eq-frequency-up").is_none());
        assert!(window.try_find("eq-gain-down").is_none());
        assert!(window.try_find("eq-q-up").is_none());
    });
}

#[gpui_kit::test]
fn eq_precision_invalid_drafts_survive_blur_and_recover_without_deleting_node(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    for (parameter, invalid, recovery) in [
        (Parameter::EqFrequency, "0 Hz", "1234.56 Hz"),
        (Parameter::EqGain, "-24.1 dB", "-7.25 dB"),
        (Parameter::EqQ, "NaN", "1.27 Q"),
    ] {
        let input = inspector_input(&view, cx, id, parameter);
        let before = settings(&view, cx, id);
        enter(&input, invalid, cx);
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        });
        cx.run_until_parked();
        service.drain(&view, cx);
        assert_eq!(settings(&view, cx, id), before);
        view.read_with(cx, |shell, cx| {
            assert_eq!(input.read(cx).value().as_str(), invalid);
            assert!(
                shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            );
        });
        cx.update(|window, cx| {
            window.focus(&input.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
        });
        cx.simulate_keystrokes("ctrl-a delete");
        service.drain(&view, cx);
        assert_eq!(settings(&view, cx, id), before);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.set_value(recovery, window, cx));
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
            assert_eq!(
                input.read(cx).value().as_str(),
                recovery,
                "render before Blur delivery must preserve dirty precision"
            );
        });
        cx.run_until_parked();
        service.drain(&view, cx);
        view.read_with(cx, |shell, cx| {
            let node = shell.state.read(cx).snapshot.graph.node(id).unwrap();
            assert_eq!(
                parameter.value(NodeKind::Eq, node),
                super::super::parameter::parse(recovery, parameter.unit()).unwrap()
            );
            assert!(
                !shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            );
        });
    }
}

#[gpui_kit::test]
fn eq_precision_card_queued_fields_merge_without_overwriting_siblings(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let original = settings(&view, cx, id);
    for (parameter, text, _) in FIELDS {
        let input = view.read_with(cx, |shell, _| {
            shell.node_controls.inputs[&(id, parameter)].clone()
        });
        enter(&input, text, cx);
        assert_eq!(
            settings(&view, cx, id),
            original,
            "test must not poll between queued fields"
        );
    }
    service.drain(&view, cx);
    assert_eq!(
        settings(&view, cx, id),
        EqSettings {
            frequency_hz: 1234.56,
            gain_db: -7.25,
            q: 1.27
        }
    );
}

#[gpui_kit::test]
fn eq_precision_inspector_queued_fields_and_pending_text_preserve_latest_intents(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let original = settings(&view, cx, id);
    for (parameter, text, _) in FIELDS {
        let input = inspector_input(&view, cx, id, parameter);
        enter(&input, text, cx);
        assert_eq!(settings(&view, cx, id), original);
    }
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
    });
    cx.run_until_parked();
    view.read_with(cx, |shell, cx| {
        for (parameter, text, _) in FIELDS {
            assert_eq!(
                shell.node_controls.precision.inputs[&(id, parameter)]
                    .read(cx)
                    .value()
                    .as_str(),
                text
            );
        }
    });
    let gain = inspector_input(&view, cx, id, Parameter::EqGain);
    enter(&gain, "0 dB", cx);
    service.drain(&view, cx);
    assert_eq!(
        settings(&view, cx, id),
        EqSettings {
            frequency_hz: 1234.56,
            gain_db: 0.,
            q: 1.27
        }
    );
    view.read_with(cx, |shell, cx| {
        assert_eq!(
            shell.state.read(cx).snapshot.consumed_request_id,
            service.handle.try_snapshot().unwrap().consumed_request_id
        );
    });
}

#[gpui_kit::test]
fn eq_precision_plot_drag_preserves_queued_exact_q(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    for _ in 0..4 {
        cx.update(|window, cx| window.render_frame(cx));
        cx.run_until_parked();
    }
    let original = settings(&view, cx, id);
    let input = view.read_with(cx, |shell, _| {
        shell.node_controls.inputs[&(id, Parameter::EqQ)].clone()
    });
    enter(&input, "1.27 Q", cx);
    let start = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-response-{id:?}"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        start + point(px(8.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    let end = start + point(px(40.), px(-10.));
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(settings(&view, cx, id), original);
    service.drain(&view, cx);
    let actual = settings(&view, cx, id);
    assert_ne!(actual.frequency_hz, original.frequency_hz);
    assert_ne!(actual.gain_db, original.gain_db);
    assert_eq!(actual.q, 1.27, "plot drag must not replace pending exact Q");
}

#[gpui_kit::test]
fn eq_precision_plot_wheel_preserves_queued_band_fields_and_camera(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    for _ in 0..4 {
        cx.update(|window, cx| window.render_frame(cx));
        cx.run_until_parked();
    }
    let original = settings(&view, cx, id);
    let camera = view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            for (parameter, value) in [
                (RackParameter::EqFrequency, 1234.56),
                (RackParameter::EqGain, -7.25),
            ] {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltinParameter {
                    node: id,
                    parameter,
                    value,
                }));
            }
        });
        (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom)
    });
    let position = cx.update(|window, _| {
        window
            .find(format!("card-response-{id:?}"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., 1.)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert_eq!(settings(&view, cx, id), original);
    service.drain(&view, cx);
    let actual = settings(&view, cx, id);
    assert_ne!(actual.q, original.q);
    assert_eq!(actual.frequency_hz, 1234.56);
    assert_eq!(actual.gain_db, -7.25);
    view.read_with(cx, |shell, _| {
        assert_eq!(
            (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
            camera
        );
    });
}

fn wheel_q(id: NodeId, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-response-{id:?}"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., 1.)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
}

fn drag_band(id: NodeId, cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, cx| window.render_frame(cx));
        cx.run_until_parked();
    }
    let start = cx.update(|window, _| {
        window
            .find(format!("card-response-{id:?}"))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        start + point(px(8.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    for (x, y) in [(20., -4.), (40., -8.), (60., -12.)] {
        cx.simulate_mouse_move(
            start + point(px(x), px(y)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
    }
    cx.simulate_mouse_up(
        start + point(px(60.), px(-12.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
}

fn queue_replacement(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
) -> EqSettings {
    let expected = EqSettings {
        frequency_hz: 880.,
        gain_db: 5.,
        q: 3.,
    };
    view.update(cx, |shell, cx| {
        let mut graph = (*shell.state.read(cx).snapshot.graph).clone();
        graph
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap()
            .builtin = crate::service::BuiltinState::Eq(expected);
        let document =
            crate::project::ProjectDocument::from_graph("Replacement EQ", &graph).unwrap();
        assert_eq!(
            document
                .restore_graph()
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.builtin.eq().is_some())
                .unwrap()
                .id,
            id,
            "exercise same-type NodeId reuse"
        );
        let request = shell.state.update(cx, |state, _| {
            state
                .try_dispatch(AppRequest::LoadProject(document.clone()))
                .unwrap()
        });
        // Match stage_project's admission state without a file picker or disk IO.
        shell.pending_project = Some((request, document, "replacement-eq.moiren.json".into()));
        shell.project_busy = true;
    });
    expected
}

#[gpui_kit::test]
fn eq_precision_review_queued_wheel_increments_accumulate_without_poll(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let before = settings(&view, cx, id);
    wheel_q(id, cx);
    wheel_q(id, cx);
    assert_eq!(settings(&view, cx, id), before);
    service.drain(&view, cx);
    let after = settings(&view, cx, id);
    assert!((after.q - before.q * 0.2_f64.exp()).abs() < 1e-12);
    assert_eq!(after.frequency_hz, before.frequency_hz);
    assert_eq!(after.gain_db, before.gain_db);
}

#[gpui_kit::test]
fn eq_precision_review_typed_q_then_wheel_uses_latest_queued_value(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    for inspector in [false, true] {
        let input = if inspector {
            inspector_input(&view, cx, id, Parameter::EqQ)
        } else {
            view.read_with(cx, |shell, _| {
                shell.node_controls.inputs[&(id, Parameter::EqQ)].clone()
            })
        };
        enter(&input, "1.27 Q", cx);
        wheel_q(id, cx);
        service.drain(&view, cx);
        assert!((settings(&view, cx, id).q - 1.27 * 0.1_f64.exp()).abs() < 1e-12);
    }
}

#[gpui_kit::test]
fn eq_precision_review_native_band_drag_is_one_undo_step(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let before = settings(&view, cx, id);
    drag_band(id, cx);
    service.drain(&view, cx);
    let after = settings(&view, cx, id);
    assert_ne!(after.frequency_hz, before.frequency_hz);
    assert_ne!(after.gain_db, before.gain_db);
    assert_eq!(after.q, before.q);
    assert!(service.context.graph.undo());
    assert_eq!(
        *service
            .context
            .graph
            .snapshot()
            .node(id)
            .unwrap()
            .builtin
            .eq()
            .unwrap(),
        before,
        "one gesture must not alternate separate frequency/gain undo keys"
    );
    assert!(service.context.graph.redo());
    assert_eq!(
        *service
            .context
            .graph
            .snapshot()
            .node(id)
            .unwrap()
            .builtin
            .eq()
            .unwrap(),
        after
    );
}

fn pending_load_input(cx: &mut TestAppContext, card: bool) {
    let (view, cx, mut service, id) = fixture(cx);
    let parameter = Parameter::EqGain;
    let input = if card {
        view.read_with(cx, |shell, _| {
            shell.node_controls.inputs[&(id, parameter)].clone()
        })
    } else {
        inspector_input(&view, cx, id, parameter)
    };
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(if card {
            format!("card-param-{id:?}-{parameter:?}")
        } else {
            format!("inspector-precision-{id:?}-{parameter:?}")
        });
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|element| {
                element.role() == Some(Role::TextInput) && element.path().starts_with(root.path())
            })
            .expect("native precision input")
            .bounds()
            .center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("-12.5 dB");
    cx.update(|window, cx| assert!(input.read(cx).focus_handle(cx).is_focused(window)));
    let expected = queue_replacement(&view, cx, id);
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| window.blur(cx));
    cx.run_until_parked();
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected);
    // A delayed notification from the retired old entity remains harmless.
    cx.update(|_, cx| input.update(cx, |_, cx| cx.emit(InputEvent::Blur)));
    cx.run_until_parked();
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected);
}

#[gpui_kit::test]
fn eq_precision_review_pending_load_blocks_native_inspector_enter_and_blur(
    cx: &mut TestAppContext,
) {
    pending_load_input(cx, false);
}

#[gpui_kit::test]
fn eq_precision_review_pending_load_blocks_native_card_enter_and_blur(cx: &mut TestAppContext) {
    pending_load_input(cx, true);
}

#[gpui_kit::test]
fn eq_precision_review_pending_load_blocks_native_plot_wheel_and_drag(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let expected = queue_replacement(&view, cx, id);
    wheel_q(id, cx);
    drag_band(id, cx);
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected);
}

#[test]
fn eq_precision_review_gesture_endpoints_are_exact_without_relaxing_typed_domains() {
    let mut graph = crate::service::ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    let snapshot = graph.snapshot();
    let (_, node) = snapshot.node_of_kind(NodeKind::Eq).unwrap();
    for parameter in [Parameter::EqFrequency, Parameter::EqGain, Parameter::EqQ] {
        let (min, max) = parameter.range(node);
        for (position, expected) in [(0., min), (1., max)] {
            let value = parameter.value_at_position(position, node);
            assert_eq!(value, expected, "{parameter:?} endpoint must be exact");
            graph
                .edit(parameter.edit(value, NodeKind::Eq, node).unwrap())
                .unwrap();
        }
        assert!(
            parameter
                .edit_precision(f64::from_bits(max.to_bits() + 1), NodeKind::Eq, node)
                .is_none()
        );
    }
    assert_eq!(
        Parameter::Level.value_at_position(0., node),
        f64::NEG_INFINITY
    );
    let maximum = Parameter::Level.range(node).1;
    assert_eq!(Parameter::Level.value_at_position(1., node), maximum);
    assert!(
        Parameter::Level
            .edit(maximum, NodeKind::Gain, node)
            .is_some()
    );
    assert!(
        Parameter::Level
            .edit_precision(maximum, NodeKind::Gain, node)
            .is_none()
    );
    let interior = 20. + (0.5 * (20_000_f64 - 20.).ln_1p()).exp_m1();
    assert_eq!(
        Parameter::EqFrequency.value_at_position(0.5, node),
        interior
    );
}
