//! Real mailbox admission with explicitly controlled snapshot publication.
use super::{Parameter, StudioShell, frequency};
use crate::service::{
    AppHandle, AppRequest, DispatchError, GraphEdit, InputSelection, ServiceContext, ServiceMailbox,
};
use crate::ui::{UiState, shell_with_state};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};
use moiren_core::graph::{NodeId, NodeKind};

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn publish(&self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.sync_node_controls(window, cx);
            });
        });
        cx.run_until_parked();
    }

    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        while let Some(request) = self.mailbox.try_request() {
            match request {
                AppRequest::EditGraph(edit) => {
                    self.context.graph.edit(edit).unwrap();
                }
                AppRequest::RefreshCatalog => {}
                other => panic!("unexpected request in stopped card fixture: {other:?}"),
            }
        }
        self.publish(view, cx);
    }

    fn bind(
        &mut self,
        view: &Entity<StudioShell>,
        cx: &mut VisualTestContext,
        id: NodeId,
        source: InputSelection,
    ) {
        self.context
            .graph
            .edit(GraphEdit::BindSource { node: id, source })
            .unwrap();
        self.publish(view, cx);
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
    let id = context
        .graph
        .snapshot()
        .node_of_kind(NodeKind::Source)
        .unwrap()
        .0
        .id();
    context
        .graph
        .edit(GraphEdit::BindSource {
            node: id,
            source: InputSelection::Tone { frequency_hz: 440. },
        })
        .unwrap();
    // No background consumer: queued edits and Busy are deterministic, while
    // the production mailbox, UI subscriptions and graph validation stay real.
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), initial));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, cx| {
        window.activate_window();
        view.update(cx, |shell, cx| {
            shell.settings_open = false;
            shell.selected_node = Some(id);
            shell.sync_node_controls(window, cx);
        });
        window.render_frame(cx);
    });
    cx.run_until_parked();
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

#[gpui_kit::test]
fn tone_card_review_queued_return_to_snapshot_value_and_pending_text_survive_sync(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let key = (id, Parameter::Frequency);
    let input = view.read_with(cx, |shell, _| shell.node_controls.inputs[&key].clone());
    enter(&input, "1000", cx);
    assert_eq!(
        frequency(&view, cx, id),
        440.,
        "the snapshot must remain unconsumed"
    );
    enter(&input, "440", cx);
    service.drain(&view, cx);
    assert_eq!(
        frequency(&view, cx, id),
        440.,
        "the second changed draft must enqueue even when equal to the old snapshot"
    );

    enter(&input, "880", cx);
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        assert_eq!(
            input.read(cx).value().as_str(),
            "880",
            "a successful pending edit must not be replaced by the old 440 Hz snapshot"
        );
    });
    service.drain(&view, cx);
    assert_eq!(frequency(&view, cx, id), 880.);
}

#[gpui_kit::test]
fn tone_card_review_busy_preserves_dirty_draft_baseline_and_allows_unchanged_retry(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let key = (id, Parameter::Frequency);
    let (input, baseline) = view.read_with(cx, |shell, _| {
        (
            shell.node_controls.inputs[&key].clone(),
            shell.node_controls.input_baselines[&key].clone(),
        )
    });
    let mut queued = 0;
    loop {
        match service.handle.try_request(AppRequest::RefreshCatalog) {
            Ok(()) => {
                queued += 1;
                assert!(queued < 1024, "the real mailbox must be bounded");
            }
            Err(DispatchError::Busy) => break,
            error => panic!("unexpected mailbox fill result: {error:?}"),
        }
    }
    assert!(queued > 0);
    enter(&input, "1000", cx);
    view.read_with(cx, |shell, cx| {
        assert!(
            shell
                .state
                .read(cx)
                .dispatch_error
                .as_deref()
                .is_some_and(|error| error.contains("Busy"))
        );
        assert_eq!(
            shell.node_controls.input_baselines[&key], baseline,
            "Busy is not a successful commit and must not mark the draft clean"
        );
        assert!(
            shell.node_controls.errors.contains_key(&key),
            "the failed field must offer a visible retryable error"
        );
    });
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
    });
    cx.run_until_parked();
    service.drain(&view, cx);
    assert_eq!(frequency(&view, cx, id), 440.);
    cx.update(|_, cx| assert_eq!(input.read(cx).value().as_str(), "1000"));

    // Enter the same draft without touching its text after queue capacity returns.
    cx.update(|_, cx| {
        input.update(cx, |_, cx| {
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            })
        })
    });
    cx.run_until_parked();
    service.drain(&view, cx);
    assert_eq!(frequency(&view, cx, id), 1000.);
    view.read_with(cx, |shell, cx| {
        assert!(shell.state.read(cx).dispatch_error.is_none());
        assert!(!shell.node_controls.errors.contains_key(&key));
        assert_eq!(shell.node_controls.input_baselines[&key], "1000");
    });
}

#[gpui_kit::test]
fn tone_card_review_rebinding_retires_old_input_and_ignores_delayed_blur(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let key = (id, Parameter::Frequency);
    let old = view.read_with(cx, |shell, _| shell.node_controls.inputs[&key].clone());
    enter(&old, "invalid", cx);
    view.read_with(cx, |shell, _| {
        assert!(shell.node_controls.errors.contains_key(&key))
    });
    cx.update(|window, cx| old.update(cx, |input, cx| input.set_value("900", window, cx)));
    service.bind(
        &view,
        cx,
        id,
        InputSelection::Physical {
            endpoint_id: "test-input".into(),
        },
    );
    let retired = view.read_with(cx, |shell, cx| {
        !shell.node_controls.inputs.contains_key(&key)
            && !shell.node_controls.input_baselines.contains_key(&key)
            && !shell.node_controls.errors.contains_key(&key)
            && !shell.node_controls.sliders.contains_key(&key)
            && !shell
                .node_controls
                .owns_focus(&old.read(cx).focus_handle(cx), cx)
    });
    service.bind(&view, cx, id, InputSelection::Tone { frequency_hz: 880. });
    let fresh = view.read_with(cx, |shell, _| shell.node_controls.inputs[&key].clone());
    // Keep the retired entity alive, as an already-queued Blur callback can be.
    cx.update(|_, cx| old.update(cx, |_, cx| cx.emit(InputEvent::Blur)));
    cx.run_until_parked();
    service.drain(&view, cx);
    let current = frequency(&view, cx, id);
    assert!(
        retired && fresh.entity_id() != old.entity_id() && current == 880.,
        "Tone -> Physical -> Tone must retire drafts/entities and fence old Blur: retired={retired}, same_entity={}, actual={current}",
        fresh.entity_id() == old.entity_id()
    );
    view.read_with(cx, |shell, cx| {
        assert_eq!(fresh.read(cx).value().as_str(), "880");
        assert_eq!(shell.node_controls.input_baselines[&key], "880");
        assert!(!shell.node_controls.errors.contains_key(&key));
    });
}
