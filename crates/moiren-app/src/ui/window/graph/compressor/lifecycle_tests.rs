//! Legacy Inspector acceptance through the real shell and a deterministic mailbox.
use crate::service::{
    AppHandle, AppRequest, CompressorParameter, DispatchError, GraphEdit, ServiceContext,
    ServiceMailbox,
};
use crate::ui::window::{Camera, StudioShell};
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};

mod intent;
mod readouts;

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn consume(&mut self) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        while let Some(request) = self.mailbox.try_dispatch(&mut self.context.signals) {
            match request {
                Some(AppRequest::EditGraph(edit)) => {
                    // Invalid UI submissions must remain visible to assertions;
                    // service rejection alone is not field-local validation.
                    let _ = self.context.graph.edit(edit.clone());
                    edits.push(edit);
                }
                Some(AppRequest::LoadProject(document)) => {
                    self.context.load_project(document).unwrap();
                }
                Some(AppRequest::RefreshCatalog) | None => {}
                other => panic!("unexpected legacy Inspector request: {other:?}"),
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
    let mut service = ManualService {
        context,
        handle,
        mailbox,
    };
    assert!(service.drain(&view, cx).is_empty());
    (view, cx, service, id)
}

fn blur(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.blur(cx);
        // Render before deferred Blur delivery, as a real focus transition may.
        window.render_frame(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.render_frame(cx));
}

fn compressor_input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: CompressorParameter,
) -> Entity<InputState> {
    view.read_with(cx, |shell, _| {
        shell.compressor.inputs[&(id, parameter)].clone()
    })
}

fn type_compressor(cx: &mut VisualTestContext, parameter: CompressorParameter, text: &str) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let field = window.find(format!("compressor-input-{}", parameter.id().0));
        assert!(field.visible());
        field.bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn value(service: &ManualService, id: NodeId, parameter: CompressorParameter) -> f64 {
    service
        .context
        .graph
        .snapshot()
        .node(id)
        .unwrap()
        .compressor
        .value(parameter)
}

#[gpui_kit::test]
fn legacy_compressor_queued_return_and_sibling_edit_preserve_last_intent(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let threshold = CompressorParameter::Threshold;
    let original = value(&service, id, threshold);
    for (parameter, text) in [
        (threshold, "-31.25".to_owned()),
        (CompressorParameter::Attack, "13.125".to_owned()),
        (threshold, original.to_string()),
    ] {
        type_compressor(cx, parameter, &text);
    }
    let edits = service.drain(&view, cx);
    assert_eq!(
        value(&service, id, threshold),
        original,
        "queued return must not be discarded against the old snapshot"
    );
    assert_eq!(value(&service, id, CompressorParameter::Attack), 13.125);
    assert_eq!(
        edits.len(),
        3,
        "one intent per changed field, including reversal"
    );
}

#[gpui_kit::test]
fn legacy_compressor_enter_then_blur_keeps_pending_text_and_submits_once(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Threshold;
    let input = compressor_input(&view, cx, id, parameter);
    type_compressor(cx, parameter, "-31.25");
    blur(cx);
    view.read_with(cx, |_, cx| {
        assert_eq!(
            input.read(cx).value().as_str(),
            "-31.25",
            "unconsumed accepted text must survive snapshot sync"
        )
    });
    assert_eq!(
        service.drain(&view, cx).len(),
        1,
        "Enter and Blur must not duplicate undo entries"
    );
    assert_eq!(value(&service, id, parameter), -31.25);
}

#[gpui_kit::test]
fn legacy_compressor_invalid_draft_survives_blur_and_recovers_locally(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Ratio;
    let input = compressor_input(&view, cx, id, parameter);
    let original = value(&service, id, parameter);
    for invalid in ["NaN", "not a ratio", "-1"] {
        type_compressor(cx, parameter, invalid);
        blur(cx);
        assert!(
            service.drain(&view, cx).is_empty(),
            "invalid input must not dispatch to the graph"
        );
        view.read_with(cx, |_, cx| {
            assert_eq!(
                input.read(cx).value().as_str(),
                invalid,
                "invalid text must remain available for correction"
            )
        });
        assert_eq!(value(&service, id, parameter), original);
    }
    type_compressor(cx, parameter, "3.125");
    assert_eq!(service.drain(&view, cx).len(), 1);
    assert_eq!(value(&service, id, parameter), 3.125);
}

#[gpui_kit::test]
fn legacy_compressor_busy_mailbox_keeps_the_same_draft_retryable(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Threshold;
    let input = compressor_input(&view, cx, id, parameter);
    let original = value(&service, id, parameter);
    for queued in 0..1024 {
        match service.handle.try_request(AppRequest::RefreshCatalog) {
            Ok(()) => assert!(queued < 1023, "mailbox must remain bounded"),
            Err(DispatchError::Busy) => break,
            other => panic!("unexpected mailbox result: {other:?}"),
        }
    }
    type_compressor(cx, parameter, "-31.25");
    blur(cx);
    assert!(service.drain(&view, cx).is_empty());
    view.read_with(cx, |_, cx| {
        assert_eq!(
            input.read(cx).value().as_str(),
            "-31.25",
            "failed admission must not retire the draft"
        )
    });
    assert_eq!(value(&service, id, parameter), original);
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("compressor-input-{}", parameter.id().0))
            .bounds()
            .center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(service.drain(&view, cx).len(), 1);
    assert_eq!(value(&service, id, parameter), -31.25);
}

#[gpui_kit::test]
fn legacy_compressor_untouched_blur_does_not_revert_another_surface(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Threshold;
    let input = compressor_input(&view, cx, id, parameter);
    cx.update(|window, cx| {
        window.click(format!("compressor-input-{}", parameter.id().0), cx);
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
    });
    service
        .context
        .graph
        .edit(GraphEdit::SetCompressor {
            node: id,
            parameter,
            value: -31.25,
        })
        .unwrap();
    service.publish(&view, cx);
    blur(cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "a focused, untouched field must not undo a newer card edit"
    );
    assert_eq!(value(&service, id, parameter), -31.25);
}

#[gpui_kit::test]
fn legacy_bus_connected_last_input_rejects_native_remove_click_before_dispatch(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Bus);
    service
        .context
        .graph
        .edit(GraphEdit::AddBusInput(id))
        .unwrap();
    let graph = service.context.graph.snapshot();
    let output = graph.node_of_kind(NodeKind::Source).unwrap().0.outputs()[0].id();
    let input = graph
        .graph
        .get_node(id)
        .unwrap()
        .inputs()
        .last()
        .unwrap()
        .id();
    service
        .context
        .graph
        .edit(GraphEdit::Connect {
            source: output,
            destination: input,
        })
        .unwrap();
    service.publish(&view, cx);
    let original = service.context.graph.snapshot();
    cx.update(|window, cx| window.click("remove-bus-input", cx));
    cx.run_until_parked();
    assert!(
        service.drain(&view, cx).is_empty(),
        "connected input removal must be disabled, not merely rejected by the service"
    );
    assert_eq!(service.context.graph.snapshot(), original);
    let edge = original
        .graph
        .edges()
        .iter()
        .find(|edge| edge.dst_port() == input)
        .unwrap()
        .id();
    service
        .context
        .graph
        .edit(GraphEdit::Disconnect(edge))
        .unwrap();
    service.publish(&view, cx);
    cx.update(|window, cx| window.click("remove-bus-input", cx));
    cx.run_until_parked();
    assert_eq!(
        service.drain(&view, cx),
        [GraphEdit::RemoveBusInput {
            node: id,
            port: input
        }]
    );
}
