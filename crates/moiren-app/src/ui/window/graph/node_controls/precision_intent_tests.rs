//! Native editing intent is distinct from rounded text and snapshot equality.
use super::{Parameter, StudioShell};
use crate::service::{
    AppHandle, AppRequest, DispatchError, GraphEdit, ServiceContext, ServiceMailbox,
};
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Modifiers, Role, TestAppContext, VisualTestContext, px, size};
use moiren_core::graph::{NodeId, NodeKind};

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        while let Some(request) = self.mailbox.try_dispatch(&mut self.context.signals) {
            match request {
                Some(AppRequest::EditGraph(edit)) => {
                    self.context.graph.edit(edit.clone()).unwrap();
                    edits.push(edit);
                }
                Some(AppRequest::RefreshCatalog) | None => {}
                other => panic!("unexpected scalar-intent request: {other:?}"),
            }
        }
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.sync_node_controls(window, cx);
            });
            window.render_frame(cx);
        });
        cx.run_until_parked();
        edits
    }
}

fn parameter(kind: NodeKind) -> Parameter {
    match kind {
        NodeKind::Eq => Parameter::EqGain,
        NodeKind::Gate => Parameter::GateThreshold,
        NodeKind::Limiter => Parameter::LimitThreshold,
        NodeKind::Delay => Parameter::DelayFeedback,
        _ => panic!("not a scalar-intent test node: {kind:?}"),
    }
}

fn fixture(
    cx: &mut TestAppContext,
    kind: NodeKind,
    initial: f64,
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
    let snapshot = context.graph.snapshot();
    let (topology, node) = snapshot.node_of_kind(kind).unwrap();
    let id = topology.id();
    context
        .graph
        .edit(parameter(kind).edit_precision(initial, kind, node).unwrap())
        .unwrap();
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

fn input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: Parameter,
    inspector: bool,
) -> Entity<InputState> {
    view.read_with(cx, |shell, _| {
        let inputs = if inspector {
            &shell.node_controls.precision.inputs
        } else {
            &shell.node_controls.inputs
        };
        inputs[&(id, parameter)].clone()
    })
}

fn value(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    kind: NodeKind,
) -> f64 {
    view.read_with(cx, |shell, cx| {
        parameter(kind).value(kind, shell.state.read(cx).snapshot.graph.node(id).unwrap())
    })
}

fn focus_native(id: NodeId, parameter: Parameter, inspector: bool, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(if inspector {
            format!("inspector-precision-{id:?}-{parameter:?}")
        } else {
            format!("card-param-{id:?}-{parameter:?}")
        });
        assert!(root.visible());
        let inputs: Vec<_> = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].visible());
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
}

fn retype(text: &str, cx: &mut VisualTestContext) {
    // Exercise real Change events, including a return to the identical string.
    // InputState::set_value deliberately emits no Change and cannot prove this.
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input(text);
    cx.run_until_parked();
}

fn assert_retyped_baseline_wins(cx: &mut TestAppContext, kind: NodeKind, inspector: bool) {
    let (view, cx, mut service, id) = fixture(cx, kind, -1.);
    let parameter = parameter(kind);
    let target = input(&view, cx, id, parameter, inspector);
    let other = input(&view, cx, id, parameter, !inspector);
    assert_ne!(target.entity_id(), other.entity_id());
    let baseline = target.read_with(cx, |input, _| input.value().to_string());

    focus_native(id, parameter, !inspector, cx);
    retype("-6", cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        value(&view, cx, id, kind),
        -1.,
        "do not publish between edits"
    );

    focus_native(id, parameter, inspector, cx);
    retype(&baseline, cx);
    assert_eq!(
        target.read_with(cx, |input, _| input.value().to_string()),
        baseline
    );
    cx.simulate_keystrokes("enter");
    let edits = service.drain(&view, cx);
    assert_eq!(
        value(&view, cx, id, kind),
        -1.,
        "{kind:?}, inspector={inspector}: explicitly retyped baseline must supersede the queued other surface; edits={edits:?}"
    );
    assert_eq!(
        edits.len(),
        2,
        "each explicit edit submits exactly one intent"
    );
}

#[gpui_kit::test]
fn scalar_precision_intent_eq_inspector_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Eq, true);
}

#[gpui_kit::test]
fn scalar_precision_intent_eq_card_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Eq, false);
}

#[gpui_kit::test]
fn scalar_precision_intent_gate_inspector_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Gate, true);
}

#[gpui_kit::test]
fn scalar_precision_intent_gate_card_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Gate, false);
}

#[gpui_kit::test]
fn scalar_precision_intent_limiter_inspector_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Limiter, true);
}

#[gpui_kit::test]
fn scalar_precision_intent_limiter_card_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Limiter, false);
}

#[gpui_kit::test]
fn scalar_precision_intent_delay_inspector_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Delay, true);
}

#[gpui_kit::test]
fn scalar_precision_intent_delay_card_retyped_baseline_wins(cx: &mut TestAppContext) {
    assert_retyped_baseline_wins(cx, NodeKind::Delay, false);
}

fn assert_untouched_preserves_precision(cx: &mut TestAppContext, inspector: bool, blur: bool) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Eq, -1.23456789);
    let original = value(&view, cx, id, NodeKind::Eq);
    let parameter = Parameter::EqGain;
    if !inspector {
        assert_eq!(
            input(&view, cx, id, parameter, false).read_with(cx, |input, _| input.value()),
            "-1.23",
            "this guard must exercise a genuinely rounded card display"
        );
    }
    focus_native(id, parameter, inspector, cx);
    if blur {
        cx.update(|window, cx| window.blur(cx));
    } else {
        cx.simulate_keystrokes("enter");
    }
    cx.run_until_parked();
    assert!(
        service.drain(&view, cx).is_empty(),
        "untouched text is not an edit"
    );
    assert_eq!(
        value(&view, cx, id, NodeKind::Eq).to_bits(),
        original.to_bits()
    );

    focus_native(id, parameter, !inspector, cx);
    retype("-6", cx);
    cx.simulate_keystrokes("enter");
    focus_native(id, parameter, inspector, cx);
    if blur {
        cx.update(|window, cx| window.blur(cx));
    } else {
        cx.simulate_keystrokes("enter");
    }
    cx.run_until_parked();
    let edits = service.drain(&view, cx);
    assert_eq!(
        edits.len(),
        1,
        "untouched old text must not undo another surface"
    );
    assert_eq!(value(&view, cx, id, NodeKind::Eq), -6.);
}

#[gpui_kit::test]
fn scalar_precision_intent_untouched_enter_neither_quantizes_nor_reverses(cx: &mut TestAppContext) {
    for inspector in [false, true] {
        assert_untouched_preserves_precision(cx, inspector, false);
    }
}

#[gpui_kit::test]
fn scalar_precision_intent_untouched_blur_neither_quantizes_nor_reverses(cx: &mut TestAppContext) {
    for inspector in [false, true] {
        assert_untouched_preserves_precision(cx, inspector, true);
    }
}

fn assert_busy_retries_same_baseline(cx: &mut TestAppContext, inspector: bool) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Eq, -1.);
    let parameter = Parameter::EqGain;
    let target = input(&view, cx, id, parameter, inspector);
    let baseline = target.read_with(cx, |input, _| input.value().to_string());
    focus_native(id, parameter, !inspector, cx);
    retype("-6", cx);
    cx.simulate_keystrokes("enter");
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
    focus_native(id, parameter, inspector, cx);
    retype(&baseline, cx);
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |shell, _| {
        let errors = if inspector {
            &shell.node_controls.precision.errors
        } else {
            &shell.node_controls.errors
        };
        assert!(
            errors.contains_key(&(id, parameter)),
            "Busy must remain retryable"
        );
    });
    assert_eq!(service.drain(&view, cx).len(), 1);
    assert_eq!(value(&view, cx, id, NodeKind::Eq), -6.);
    assert_eq!(
        target.read_with(cx, |input, _| input.value().to_string()),
        baseline
    );

    // Capacity recovered; retry without another Change, then blur after success.
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| window.blur(cx));
    cx.run_until_parked();
    assert_eq!(
        service.drain(&view, cx).len(),
        1,
        "successful retry clears edit intent"
    );
    assert_eq!(value(&view, cx, id, NodeKind::Eq), -1.);
    view.read_with(cx, |shell, _| {
        let errors = if inspector {
            &shell.node_controls.precision.errors
        } else {
            &shell.node_controls.errors
        };
        assert!(!errors.contains_key(&(id, parameter)));
    });
}

#[gpui_kit::test]
fn scalar_precision_intent_busy_card_retries_identical_baseline(cx: &mut TestAppContext) {
    assert_busy_retries_same_baseline(cx, false);
}

#[gpui_kit::test]
fn scalar_precision_intent_busy_inspector_retries_identical_baseline(cx: &mut TestAppContext) {
    assert_busy_retries_same_baseline(cx, true);
}
