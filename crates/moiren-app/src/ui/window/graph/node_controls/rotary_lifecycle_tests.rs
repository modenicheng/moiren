//! Node-local rotary admission, acknowledgement and retired callback regressions.
use super::{Parameter, StudioShell};
use crate::service::{
    AppHandle, AppRequest, BuiltinState, DispatchError, GraphEdit, RackParameter, ServiceContext,
    ServiceMailbox,
};
use crate::ui::widgets::fader::SliderBehavior;
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, Role, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, point, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::processor::{DelaySettings, LimiterSettings};

const PARAMETER: Parameter = Parameter::LimitInput;

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn consume(&mut self) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        // Subscriptions share the queue; None is not necessarily end-of-queue.
        while let Some(request) = self.mailbox.try_dispatch(&mut self.context.signals) {
            match request {
                Some(AppRequest::EditGraph(edit)) => {
                    self.context.graph.edit(edit.clone()).unwrap();
                    edits.push(edit);
                }
                Some(AppRequest::LoadProject(document)) => {
                    self.context.load_project(document).unwrap();
                }
                Some(AppRequest::RefreshCatalog) | None => {}
                other => panic!("unexpected rotary fixture request: {other:?}"),
            }
        }
        edits
    }

    fn publish(&self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.sync_node_controls(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
        });
        cx.run_until_parked();
    }

    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> Vec<GraphEdit> {
        let edits = self.consume();
        self.publish(view, cx);
        edits
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
    fixture_kind(cx, NodeKind::Limiter)
}

fn fixture_kind(
    cx: &mut TestAppContext,
    kind: NodeKind,
) -> (
    Entity<StudioShell>,
    &mut VisualTestContext,
    ManualService,
    NodeId,
) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode { kind, channels: 2 })
        .unwrap();
    let id = context.graph.snapshot().nodes.last().unwrap().id;
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let state = cx.new(|_| UiState::new(handle.clone(), handle.try_snapshot().unwrap()));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, cx| {
        window.resize(size(px(1600.), px(1000.)));
        window.activate_window();
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
        window.render_frame(cx);
    });
    settle_panels(cx);
    let mut service = ManualService {
        context,
        handle,
        mailbox,
    };
    assert!(service.drain(&view, cx).is_empty());
    (view, cx, service, id)
}

fn retained(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
) -> (Entity<SliderState>, SliderBehavior) {
    view.read_with(cx, |shell, _| {
        let slot = &shell.node_controls.sliders[&(id, PARAMETER)];
        (slot.state.clone(), slot.behavior.clone())
    })
}

fn slider_position(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> f32 {
    view.read_with(cx, |shell, cx| {
        shell.node_controls.sliders[&(id, PARAMETER)]
            .state
            .read(cx)
            .value()
            .end()
    })
}

fn assert_position(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    expected: f32,
) {
    let actual = slider_position(view, cx, id);
    assert!(
        (actual - expected).abs() < 0.00001,
        "expected admitted rotary position {expected}, got {actual}"
    );
}

fn value(service: &ManualService, id: NodeId) -> f64 {
    service
        .context
        .graph
        .snapshot()
        .node(id)
        .unwrap()
        .builtin
        .limiter()
        .unwrap()
        .input_gain_db
}

fn expected_value(position: f32) -> f64 {
    -60. + f64::from(position) * 120.
}

fn exact(id: NodeId, value: f64) -> GraphEdit {
    GraphEdit::SetBuiltinParameter {
        node: id,
        parameter: RackParameter::LimitInput,
        value,
    }
}

fn commit(behavior: &SliderBehavior, position: f32, cx: &mut VisualTestContext) {
    cx.update(|window, cx| (behavior.commit)(position, window, cx));
    cx.run_until_parked();
}

fn delayed_drag(slider: &Entity<SliderState>, position: f32, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        slider.update(cx, |slider, cx| {
            slider.set_value(position, window, cx);
            cx.emit(SliderEvent::Change(position.into()));
            cx.emit(SliderEvent::Release(position.into()));
        });
    });
    cx.run_until_parked();
}

fn wheel(id: NodeId, fine: bool, cx: &mut VisualTestContext) {
    // Always repaint without draining the service: this is the regression gap.
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let knob = window.find(format!("card-knob-{id:?}-{PARAMETER:?}"));
        assert!(knob.visible());
        knob.bounds().center()
    });
    let modifiers = Modifiers {
        shift: fine,
        ..Default::default()
    };
    cx.simulate_mouse_move(position, None, modifiers);
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., 1.)),
        modifiers,
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
}

fn queue_replacement(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) {
    view.update(cx, |shell, cx| {
        let mut graph = (*shell.state.read(cx).snapshot.graph).clone();
        graph
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap()
            .builtin = BuiltinState::Limiter(LimiterSettings {
            input_gain_db: 12.,
            ..Default::default()
        });
        let document =
            crate::project::ProjectDocument::from_graph("Replacement rotary", &graph).unwrap();
        assert_eq!(
            document
                .restore_graph()
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.builtin.limiter().is_some())
                .unwrap()
                .id,
            id,
            "exercise same-kind NodeId reuse"
        );
        let request = shell.state.update(cx, |state, _| {
            state
                .try_dispatch(AppRequest::LoadProject(document.clone()))
                .unwrap()
        });
        shell.pending_project = Some((request, document, "replacement-rotary.moiren.json".into()));
        shell.project_busy = true;
    });
}

#[gpui_kit::test]
fn rotary_lifecycle_native_wheel_accumulates_across_unacknowledged_renders(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let camera = view.read_with(cx, |shell, _| shell.camera);
    wheel(id, false, cx);
    wheel(id, false, cx);
    assert_position(&view, cx, id, 0.52);
    let edits = service.drain(&view, cx);
    assert_eq!(edits.len(), 2, "both wheel intents must be admitted");
    assert!((value(&service, id) - 2.4).abs() < 0.0001);
    view.read_with(cx, |shell, _| {
        assert_eq!(shell.camera.pan_x, camera.pan_x);
        assert_eq!(shell.camera.pan_y, camera.pan_y);
        assert_eq!(shell.camera.zoom, camera.zoom);
    });
    wheel(id, true, cx);
    wheel(id, true, cx);
    service.drain(&view, cx);
    assert!((value(&service, id) - 2.64).abs() < 0.0001);
}

#[gpui_kit::test]
fn rotary_lifecycle_partial_ack_keeps_newest_intent_then_accepts_authoritative_state(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let (_, behavior) = retained(&view, cx, id);
    commit(&behavior, 0.6, cx);
    assert_eq!(service.consume(), [exact(id, expected_value(0.6))]);
    // The first request is consumed but not published; the second is still queued.
    commit(&behavior, 0.7, cx);
    service.publish(&view, cx);
    assert_position(&view, cx, id, 0.7);
    assert_eq!(service.drain(&view, cx), [exact(id, expected_value(0.7))]);
    service.context.graph.edit(exact(id, -24.)).unwrap();
    service.publish(&view, cx);
    assert_position(&view, cx, id, 0.3);
}

#[gpui_kit::test]
fn rotary_lifecycle_pending_load_blocks_wheel_reset_and_delayed_release(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let (slider, behavior) = retained(&view, cx, id);
    queue_replacement(&view, cx, id);
    wheel(id, false, cx);
    commit(&behavior, behavior.reset, cx);
    delayed_drag(&slider, 0.8, cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "outgoing rotary events must not enqueue edits behind LoadProject"
    );
    assert_eq!(value(&service, id), 12.);
}

#[gpui_kit::test]
fn rotary_lifecycle_successful_load_rejects_retained_commit_and_drag_callbacks(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let (old_slider, old_behavior) = retained(&view, cx, id);
    queue_replacement(&view, cx, id);
    assert!(service.drain(&view, cx).is_empty());
    let (fresh_slider, fresh_behavior) = retained(&view, cx, id);
    assert_ne!(old_slider.entity_id(), fresh_slider.entity_id());
    commit(&old_behavior, 0.9, cx);
    delayed_drag(&old_slider, 0.8, cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "retired events must not target the replacement node"
    );
    assert_eq!(value(&service, id), 12.);
    commit(&fresh_behavior, 0.7, cx);
    assert_eq!(service.drain(&view, cx), [exact(id, expected_value(0.7))]);
}

#[gpui_kit::test]
fn rotary_lifecycle_removed_and_undone_node_rejects_retired_slider(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let (old_slider, old_behavior) = retained(&view, cx, id);
    service
        .context
        .graph
        .edit(GraphEdit::RemoveNode(id))
        .unwrap();
    service.publish(&view, cx);
    commit(&old_behavior, 0.8, cx);
    delayed_drag(&old_slider, 0.9, cx);
    assert!(service.consume().is_empty());
    assert!(service.context.graph.undo());
    service.publish(&view, cx);
    let (fresh_slider, _) = retained(&view, cx, id);
    assert_ne!(old_slider.entity_id(), fresh_slider.entity_id());
    commit(&old_behavior, 0.8, cx);
    delayed_drag(&old_slider, 0.9, cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "undo may restore the NodeId but cannot restore callback ownership"
    );
    assert_eq!(value(&service, id), 0.);
}

#[gpui_kit::test]
fn rotary_lifecycle_busy_does_not_acknowledge_or_pin_rejected_intent(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let (_, behavior) = retained(&view, cx, id);
    commit(&behavior, 0.6, cx);
    let mut queued = 0;
    loop {
        match service.handle.try_request(AppRequest::RefreshCatalog) {
            Ok(()) => {
                queued += 1;
                assert!(queued < 1024, "mailbox must be bounded");
            }
            Err(DispatchError::Busy) => break,
            other => panic!("unexpected mailbox fill result: {other:?}"),
        }
    }
    assert!(queued > 0);
    commit(&behavior, 0.7, cx);
    assert_eq!(service.drain(&view, cx), [exact(id, expected_value(0.6))]);
    assert_position(&view, cx, id, 0.6);
    commit(&behavior, 0.7, cx);
    assert_eq!(service.drain(&view, cx), [exact(id, expected_value(0.7))]);
    assert_position(&view, cx, id, 0.7);
}

#[gpui_kit::test]
fn rotary_lifecycle_active_release_endpoints_and_reset_preserve_existing_domain(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let (slider, behavior) = retained(&view, cx, id);
    delayed_drag(&slider, 0.8, cx);
    let edits = service.drain(&view, cx);
    assert!(!edits.is_empty());
    assert_eq!(value(&service, id), expected_value(0.8));
    for (position, expected) in [(0., -60.), (1., 60.), (behavior.reset, 0.)] {
        commit(&behavior, position, cx);
        assert_eq!(service.drain(&view, cx), [exact(id, expected)]);
        assert_eq!(value(&service, id), expected);
    }
}

fn typed_then_wheel(cx: &mut TestAppContext, inspector: bool) {
    let (view, cx, mut service, id) = fixture(cx);
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(if inspector {
            format!("inspector-precision-{id:?}-{PARAMETER:?}")
        } else {
            format!("card-param-{id:?}-{PARAMETER:?}")
        });
        assert!(root.visible());
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .expect("native scalar field")
            .bounds()
            .center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("12.125");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    // Plain Enter must retain full precision and enqueue only the exact edit.
    // Consume without publishing so the wheel still sees the old UI snapshot.
    assert_eq!(service.consume(), [exact(id, 12.125)]);
    assert_eq!(value(&service, id), 12.125);
    view.read_with(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .builtin
                .limiter()
                .unwrap()
                .input_gain_db,
            0.
        );
    });
    wheel(id, false, cx);
    let edits = service.drain(&view, cx);
    assert_eq!(edits.len(), 1, "the wheel adds one ordered gesture");
    assert!(
        (value(&service, id) - 13.325).abs() < 0.0001,
        "wheel must advance the admitted numeric value, not the old snapshot"
    );
}

#[gpui_kit::test]
fn rotary_lifecycle_card_numeric_admission_precedes_wheel(cx: &mut TestAppContext) {
    typed_then_wheel(cx, false);
}

#[gpui_kit::test]
fn rotary_lifecycle_inspector_numeric_admission_precedes_wheel(cx: &mut TestAppContext) {
    typed_then_wheel(cx, true);
}

#[gpui_kit::test]
fn rotary_lifecycle_delay_reset_refreshes_when_storage_maximum_changes(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture_kind(cx, NodeKind::Delay);
    let key = (id, Parameter::DelayTime);
    let original = view.read_with(cx, |shell, _| {
        shell.node_controls.sliders[&key].state.entity_id()
    });
    for (maximum, expected) in [(80., 80.), (500., 250.)] {
        service
            .context
            .graph
            .edit(GraphEdit::SetBuiltin {
                node: id,
                state: BuiltinState::Delay(DelaySettings {
                    delay_ms: 20.,
                    max_delay_ms: maximum,
                    ..Default::default()
                }),
            })
            .unwrap();
        service.publish(&view, cx);
        let behavior = view.read_with(cx, |shell, cx| {
            let slot = &shell.node_controls.sliders[&key];
            assert_eq!(
                slot.state.entity_id(),
                original,
                "refresh without recreating the slider"
            );
            let graph = &shell.state.read(cx).snapshot.graph;
            assert_eq!(
                slot.behavior.reset,
                Parameter::DelayTime.default_position(NodeKind::Delay, graph.node(id).unwrap()),
                "reset must use the current Delay capacity"
            );
            slot.behavior.clone()
        });
        commit(&behavior, behavior.reset, cx);
        assert_eq!(service.drain(&view, cx).len(), 1);
        let snapshot = service.context.graph.snapshot();
        let settings = snapshot.node(id).unwrap().builtin.delay().unwrap();
        assert!((settings.delay_ms - expected).abs() < 0.001);
        assert_eq!(settings.max_delay_ms, maximum);
    }
}
