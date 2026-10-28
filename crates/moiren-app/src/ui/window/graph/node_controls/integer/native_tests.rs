//! Native acceptance for Meter presets and Inspector integer editors.
//! The manual mailbox keeps queued edits deterministic; no audio or file worker runs.
use super::{Field, Key, StudioShell, value_and_maximum};
use crate::service::{
    AppHandle, AppRequest, AppSnapshot, BuiltinState, DispatchError, GraphEdit, RecordingPhase,
    ServiceContext, ServiceMailbox,
};
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Focusable, Modifiers, Role, TestAppContext, VisualTestContext, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::recorder::RecorderSettings;

struct ManualService {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn publish_snapshot(
        &self,
        snapshot: AppSnapshot,
        view: &Entity<StudioShell>,
        cx: &mut VisualTestContext,
    ) {
        self.mailbox.publish(snapshot);
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

    fn publish(&self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        self.publish_snapshot(crate::service::snapshot(&self.context), view, cx);
    }

    fn consume(&mut self) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        // Shell subscriptions share this queue. Consume them too, so a pending
        // subscription cannot hide an edit behind try_request's None sentinel.
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
                other => panic!("unexpected integer fixture request: {other:?}"),
            }
        }
        edits
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
    channels: usize,
) -> (
    Entity<StudioShell>,
    &mut VisualTestContext,
    ManualService,
    NodeId,
) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode { kind, channels })
        .unwrap();
    let id = context.graph.snapshot().nodes.last().unwrap().id;
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let state = cx.new(|_| UiState::new(handle.clone(), handle.try_snapshot().unwrap()));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, _| {
        window.resize(size(px(1600.), px(1000.)));
        window.activate_window();
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

fn root(key: Key) -> String {
    format!("integer-precision-{:?}-{:?}", key.0, key.1)
}

fn input_id(key: Key) -> String {
    format!("integer-exact-{:?}-{:?}", key.0, key.1)
}

fn inspector_input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    key: Key,
) -> Entity<InputState> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(root(key));
        assert!(root.visible());
        assert!(
            root.path()
                .starts_with(window.find("inspector-parameters").path())
        );
        let native = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|entry| {
                entry.role() == Some(Role::TextInput) && entry.path().starts_with(root.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(native.len(), 1, "one real input per integer field");
        assert!(native[0].visible());
        // This kit exposes no disabled metadata for these inputs. Enabled/busy
        // behavior is proved by native typing, queued requests and graph state.
        assert!(native[0].bounds().left() >= root.bounds().left());
        assert!(native[0].bounds().right() <= root.bounds().right());
    });
    view.read_with(cx, |shell, cx| {
        let input = shell.node_controls.integer.inputs[&key].clone();
        assert!(shell.typing_focus(&input.read(cx).focus_handle(cx), cx));
        input
    })
}

fn type_value(cx: &mut VisualTestContext, key: Key, text: &str, submit: bool) {
    let point = cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(input_id(key)).bounds().center()
    });
    cx.simulate_click(point, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
    if submit {
        cx.simulate_keystrokes("enter");
    }
    cx.run_until_parked();
}

fn blur_and_sync(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.blur(cx);
        // A render may run before Blur delivery. Dirty/pending text must survive.
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        window.render_frame(cx);
    });
    cx.run_until_parked();
}

fn confirmed(view: &Entity<StudioShell>, cx: &mut VisualTestContext, key: Key) -> usize {
    view.read_with(cx, |shell, cx| {
        value_and_maximum(&shell.state.read(cx).snapshot, key)
            .unwrap()
            .0
    })
}

fn requested_values(edits: Vec<GraphEdit>, key: Key) -> Vec<usize> {
    edits
        .into_iter()
        .map(|edit| match edit {
            GraphEdit::SetBuiltin { node, state } if node == key.0 => match (key.1, state) {
                (Field::MeterWindow, BuiltinState::Meter(settings)) => settings.window_frames,
                (Field::RecorderCapacity, BuiltinState::Recorder(settings)) => {
                    settings.capacity_frames
                }
                other => panic!("wrong integer field: {other:?}"),
            },
            other => panic!("unexpected integer edit: {other:?}"),
        })
        .collect()
}

fn assert_error(view: &Entity<StudioShell>, cx: &mut VisualTestContext, key: Key, text: &str) {
    view.read_with(cx, |shell, cx| {
        let editors = &shell.node_controls.integer;
        assert_eq!(editors.inputs[&key].read(cx).value().as_str(), text);
        assert!(editors.drafts[&key].error.is_some());
        assert!(editors.drafts[&key].dirty(text));
        assert!(editors.drafts[&key].pending.is_none());
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find(format!("integer-error-{:?}-{:?}", key.0, key.1))
                .visible()
        );
    });
}

fn emit_retired_events(input: &Entity<InputState>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        input.update(cx, |_, cx| {
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
            cx.emit(InputEvent::Blur);
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn meter_integer_native_enter_and_blur_submit_exactly_once(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    let input = inspector_input(&view, cx, key);
    type_value(cx, key, "12345", true);
    assert_eq!(requested_values(service.drain(&view, cx), key), [12345]);
    assert_eq!(confirmed(&view, cx, key), 12345);
    blur_and_sync(&view, cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "Enter then Blur is one edit"
    );

    type_value(cx, key, "1048575", false);
    cx.update(|window, cx| assert!(input.read(cx).focus_handle(cx).is_focused(window)));
    blur_and_sync(&view, cx);
    view.read_with(cx, |_, cx| {
        assert_eq!(input.read(cx).value().as_str(), "1048575")
    });
    assert_eq!(requested_values(service.drain(&view, cx), key), [1_048_575]);
    assert_eq!(confirmed(&view, cx, key), 1_048_575);
}

#[gpui_kit::test]
fn meter_integer_native_invalid_decimal_exponent_and_zero_recover_on_blur(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    inspector_input(&view, cx, key);
    for (invalid, recovery, expected) in [
        ("1.5", "1", 1),
        ("1e3", "1000", 1000),
        ("0", "1048576", 1_048_576),
    ] {
        let before = confirmed(&view, cx, key);
        type_value(cx, key, invalid, true);
        blur_and_sync(&view, cx);
        assert!(service.drain(&view, cx).is_empty());
        assert_eq!(confirmed(&view, cx, key), before);
        assert_error(&view, cx, key, invalid);
        type_value(cx, key, recovery, false);
        blur_and_sync(&view, cx);
        assert_eq!(requested_values(service.drain(&view, cx), key), [expected]);
        assert_eq!(confirmed(&view, cx, key), expected);
        view.read_with(cx, |shell, _| {
            assert!(shell.node_controls.integer.drafts[&key].error.is_none())
        });
    }
}

#[gpui_kit::test]
fn meter_integer_native_full_mailbox_keeps_unchanged_draft_retryable(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    inspector_input(&view, cx, key);
    let before = confirmed(&view, cx, key);
    let mut queued = 0;
    loop {
        match service.handle.try_request(AppRequest::RefreshCatalog) {
            Ok(()) => {
                queued += 1;
                assert!(queued < 1024, "the mailbox must remain bounded");
            }
            Err(DispatchError::Busy) => break,
            other => panic!("unexpected mailbox fill result: {other:?}"),
        }
    }
    assert!(queued > 0);
    type_value(cx, key, "12345", true);
    assert_error(&view, cx, key, "12345");
    assert!(service.drain(&view, cx).is_empty());
    assert_eq!(confirmed(&view, cx, key), before);
    assert_error(&view, cx, key, "12345");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(requested_values(service.drain(&view, cx), key), [12345]);
    assert_eq!(confirmed(&view, cx, key), 12345);
    view.read_with(cx, |shell, _| {
        assert!(shell.node_controls.integer.drafts[&key].error.is_none())
    });
}

#[gpui_kit::test]
fn meter_integer_native_pending_return_to_snapshot_preserves_both_intents(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    let input = inspector_input(&view, cx, key);
    let original = confirmed(&view, cx, key);
    let first_text = (original + 1).to_string();
    type_value(cx, key, &first_text, true);
    let first = view.read_with(cx, |shell, _| {
        shell.node_controls.integer.drafts[&key].pending.unwrap()
    });
    blur_and_sync(&view, cx);
    view.read_with(cx, |shell, cx| {
        assert_eq!(input.read(cx).value().as_str(), first_text);
        assert_eq!(
            shell.node_controls.integer.drafts[&key].pending,
            Some(first)
        );
        assert!(shell.state.read(cx).snapshot.consumed_request_id < first);
    });
    type_value(cx, key, &original.to_string(), true);
    view.read_with(cx, |shell, _| {
        assert!(shell.node_controls.integer.drafts[&key].pending.unwrap() > first)
    });
    assert_eq!(
        confirmed(&view, cx, key),
        original,
        "no publication between edits"
    );
    assert_eq!(
        requested_values(service.drain(&view, cx), key),
        [original + 1, original]
    );
    assert_eq!(confirmed(&view, cx, key), original);
    view.read_with(cx, |shell, _| {
        assert!(shell.node_controls.integer.drafts[&key].pending.is_none())
    });
    blur_and_sync(&view, cx);
    assert!(service.drain(&view, cx).is_empty());
}

#[gpui_kit::test]
fn meter_integer_native_card_presets_allow_rapid_return_to_selected_window(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    assert_eq!(confirmed(&view, cx, key), 480);
    for frames in [128, 480] {
        let position = cx.update(|window, cx| {
            window.render_frame(cx);
            let preset = window.find(format!("card-meter-window-{id:?}-{frames}"));
            assert!(preset.visible());
            preset.bounds().center()
        });
        cx.simulate_click(position, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            confirmed(&view, cx, key),
            480,
            "both clicks must precede service publication"
        );
    }
    assert_eq!(
        requested_values(service.drain(&view, cx), key),
        [128, 480],
        "the still-selected 480 preset must enqueue the final reversal"
    );
    assert_eq!(confirmed(&view, cx, key), 480);
    // ActionButton also omits disabled metadata. Prove that the confirmed,
    // selected preset remains actionable by clicking and inspecting its request.
    let selected = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("card-meter-window-{id:?}-480"))
            .bounds()
            .center()
    });
    cx.simulate_click(selected, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(requested_values(service.drain(&view, cx), key), [480]);
    assert_eq!(confirmed(&view, cx, key), 480);
}

#[gpui_kit::test]
fn recorder_integer_native_capacity_uses_real_channels_not_saved_file_channels(
    cx: &mut TestAppContext,
) {
    for (channels, maximum) in [(1, 1_000_000), (2, 1_000_000), (64, 262_144), (256, 65_536)] {
        let (view, cx, mut service, id) = fixture(cx, NodeKind::Recorder, channels);
        let key = (id, Field::RecorderCapacity);
        assert_eq!(
            RecorderSettings::max_capacity_frames(channels),
            Some(maximum)
        );
        let mut snapshot = crate::service::snapshot(&service.context);
        let recording = snapshot
            .recordings
            .iter_mut()
            .find(|entry| entry.node == id)
            .unwrap();
        recording.phase = RecordingPhase::Finished;
        recording.channels = Some(2);
        service.publish_snapshot(snapshot, &view, cx);
        inspector_input(&view, cx, key);
        let before = confirmed(&view, cx, key);
        let invalid = (maximum + 1).to_string();
        type_value(cx, key, &invalid, true);
        blur_and_sync(&view, cx);
        // Keep the contradictory saved-file metadata published during rejection;
        // an Idle refresh here would hide a limit derived from the wrong channels.
        assert!(service.consume().is_empty());
        assert_eq!(confirmed(&view, cx, key), before);
        assert_error(&view, cx, key, &invalid);
        type_value(cx, key, &maximum.to_string(), true);
        assert_eq!(requested_values(service.drain(&view, cx), key), [maximum]);
        assert_eq!(confirmed(&view, cx, key), maximum);
        type_value(cx, key, "1", false);
        blur_and_sync(&view, cx);
        assert_eq!(requested_values(service.drain(&view, cx), key), [1]);
        assert_eq!(confirmed(&view, cx, key), 1);
    }
}

#[gpui_kit::test]
fn recorder_integer_native_busy_phases_disable_input_and_reject_delayed_events(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Recorder, 2);
    let key = (id, Field::RecorderCapacity);
    let input = inspector_input(&view, cx, key);
    let before = confirmed(&view, cx, key);
    type_value(cx, key, "12345", false);
    // These are UI projections, not a simulated recording session: assert that
    // no edit even reaches the mailbox during each confirmed busy phase.
    for phase in [
        RecordingPhase::Starting,
        RecordingPhase::Recording,
        RecordingPhase::Stopping,
    ] {
        let mut snapshot = crate::service::snapshot(&service.context);
        snapshot
            .recordings
            .iter_mut()
            .find(|entry| entry.node == id)
            .unwrap()
            .phase = phase;
        service.publish_snapshot(snapshot, &view, cx);
        inspector_input(&view, cx, key);
        type_value(cx, key, "67890", true);
        view.read_with(cx, |_, cx| {
            assert_eq!(input.read(cx).value().as_str(), "12345")
        });
        assert!(
            service.consume().is_empty(),
            "native typing/Enter must not enqueue busy capacity: {phase:?}"
        );
        assert_eq!(confirmed(&view, cx, key), before);
        // Independently exercise late notifications that bypass widget gating.
        emit_retired_events(&input, cx);
        assert!(
            service.consume().is_empty(),
            "busy capacity must not be enqueued: {phase:?}"
        );
        assert_eq!(confirmed(&view, cx, key), before);
        assert_error(&view, cx, key, "12345");
    }
    service.publish(&view, cx);
    inspector_input(&view, cx, key);
    type_value(cx, key, "48001", true);
    assert_eq!(requested_values(service.drain(&view, cx), key), [48001]);
    assert_eq!(confirmed(&view, cx, key), 48001);
}

#[gpui_kit::test]
fn meter_integer_native_removal_and_undo_fence_retired_input_entity(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    let old = inspector_input(&view, cx, key);
    type_value(cx, key, "invalid", true);
    assert_error(&view, cx, key, "invalid");
    cx.update(|window, cx| old.update(cx, |input, cx| input.set_value("900", window, cx)));
    service
        .context
        .graph
        .edit(GraphEdit::RemoveNode(id))
        .unwrap();
    service.publish(&view, cx);
    view.read_with(cx, |shell, cx| {
        assert!(!shell.node_controls.integer.inputs.contains_key(&key));
        assert!(!shell.node_controls.integer.drafts.contains_key(&key));
        assert!(
            !shell
                .node_controls
                .integer
                .owns_focus(&old.read(cx).focus_handle(cx), cx)
        );
    });
    emit_retired_events(&old, cx);
    assert!(service.drain(&view, cx).is_empty());
    assert!(service.context.graph.undo());
    service.publish(&view, cx);
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(id);
        shell.selected_nodes.insert(id);
        shell.inspector_open = true;
        cx.notify();
    });
    let fresh = inspector_input(&view, cx, key);
    assert_ne!(old.entity_id(), fresh.entity_id());
    type_value(cx, key, "2222", true);
    emit_retired_events(&old, cx);
    assert_eq!(requested_values(service.drain(&view, cx), key), [2222]);
    assert_eq!(confirmed(&view, cx, key), 2222);
    view.read_with(cx, |shell, cx| {
        assert_eq!(fresh.read(cx).value().as_str(), "2222");
        assert!(shell.node_controls.integer.drafts[&key].error.is_none());
    });
}

#[gpui_kit::test]
fn integer_native_kind_rebinding_fences_old_events_at_reused_key(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Meter, 2);
    let key = (id, Field::MeterWindow);
    let old = inspector_input(&view, cx, key);
    type_value(cx, key, "900", false);
    let original = service.context.graph.snapshot();
    let mut replacement = ServiceContext::default();
    replacement
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Recorder,
            channels: 2,
        })
        .unwrap();
    let replacement = replacement.graph.snapshot();
    assert_eq!(
        replacement.nodes.last().unwrap().id,
        id,
        "fixture must reuse the same NodeId"
    );
    service
        .context
        .graph
        .replace(replacement.graph, replacement.nodes)
        .unwrap();
    service.publish(&view, cx);
    view.read_with(cx, |shell, _| {
        assert!(!shell.node_controls.integer.inputs.contains_key(&key));
        assert!(!shell.node_controls.integer.drafts.contains_key(&key));
    });
    inspector_input(&view, cx, (id, Field::RecorderCapacity));
    emit_retired_events(&old, cx);
    assert!(service.drain(&view, cx).is_empty());
    service
        .context
        .graph
        .replace(original.graph, original.nodes)
        .unwrap();
    service.publish(&view, cx);
    let fresh = inspector_input(&view, cx, key);
    assert_ne!(old.entity_id(), fresh.entity_id());
    let before = confirmed(&view, cx, key);
    type_value(cx, key, "1e3", true);
    emit_retired_events(&old, cx);
    assert!(service.drain(&view, cx).is_empty());
    assert_eq!(confirmed(&view, cx, key), before);
    assert_error(&view, cx, key, "1e3");
}

#[gpui_kit::test]
fn integer_native_pending_load_blocks_old_enter_and_blur_before_same_id_snapshot(
    cx: &mut TestAppContext,
) {
    for (kind, field, expected) in [
        (NodeKind::Meter, Field::MeterWindow, 777),
        (NodeKind::Recorder, Field::RecorderCapacity, 32_123),
    ] {
        let (view, cx, mut service, id) = fixture(cx, kind, 2);
        let key = (id, field);
        let old = inspector_input(&view, cx, key);
        let before = confirmed(&view, cx, key);
        type_value(cx, key, "12345", false);
        cx.update(|window, cx| assert!(old.read(cx).focus_handle(cx).is_focused(window)));

        let mut replacement = service.context.graph.snapshot();
        let state = &mut replacement
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap()
            .builtin;
        match state {
            BuiltinState::Meter(settings) => settings.window_frames = expected,
            BuiltinState::Recorder(settings) => settings.capacity_frames = expected,
            _ => panic!("expected integer node"),
        }
        let document =
            crate::project::ProjectDocument::from_graph("Replacement integer node", &replacement)
                .unwrap();
        assert_eq!(
            document
                .restore_graph()
                .unwrap()
                .graph
                .get_node(id)
                .unwrap()
                .kind(),
            kind,
            "the load must reuse both node identity and field kind"
        );
        view.update(cx, |shell, cx| {
            let request = shell.state.update(cx, |state, _| {
                state
                    .try_dispatch(AppRequest::LoadProject(document.clone()))
                    .unwrap()
            });
            // Same admission state as stage_project, without a file picker or IO.
            shell.pending_project =
                Some((request, document, "replacement-integer.moiren.json".into()));
            shell.project_busy = true;
        });

        cx.simulate_keystrokes("enter");
        blur_and_sync(&view, cx);
        view.read_with(cx, |shell, cx| {
            assert!(shell.pending_project.is_some());
            let editors = &shell.node_controls.integer;
            assert_eq!(editors.inputs[&key].entity_id(), old.entity_id());
            assert_eq!(old.read(cx).value().as_str(), "12345");
            assert!(editors.drafts[&key].dirty("12345"));
            assert!(editors.drafts[&key].pending.is_none());
        });
        assert_eq!(
            confirmed(&view, cx, key),
            before,
            "no replacement snapshot yet"
        );
        assert!(
            service.consume().is_empty(),
            "old Enter/Blur must not enqueue an edit behind LoadProject"
        );
        assert_eq!(
            value_and_maximum(&crate::service::snapshot(&service.context), key)
                .unwrap()
                .0,
            expected
        );
        service.publish(&view, cx);
        assert_eq!(confirmed(&view, cx, key), expected);
        view.read_with(cx, |shell, cx| {
            assert!(shell.pending_project.is_none());
            assert!(!shell.project_busy);
            let editors = &shell.node_controls.integer;
            let fresh = &editors.inputs[&key];
            assert_ne!(fresh.entity_id(), old.entity_id());
            assert_eq!(fresh.read(cx).value().as_str(), expected.to_string());
            assert!(!editors.owns_focus(&old.read(cx).focus_handle(cx), cx));
        });
        emit_retired_events(&old, cx);
        assert!(service.drain(&view, cx).is_empty());
        assert_eq!(confirmed(&view, cx, key), expected);
    }
}

// Nested only to reuse this manual-service fixture without changing its API.
#[path = "intent_tests.rs"]
mod intent_tests;
