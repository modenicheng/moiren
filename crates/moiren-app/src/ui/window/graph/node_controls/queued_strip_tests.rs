//! Native strip and Pan intents must compose before a new snapshot is published.
use super::StudioShell;
use crate::service::{
    AppRequest, GraphEdit, GraphSnapshot, InputSelection, MixerState, ServiceContext,
    ServiceMailbox,
};
use crate::ui::window::{Camera, Page};
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, Role, ScrollDelta, TestAppContext, VisualTestContext, point, px,
    size,
};
use moiren_core::graph::{NodeId, NodeKind};

struct ManualService {
    context: ServiceContext,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        // Subscription messages share the mailbox, so consume through try_dispatch.
        while let Some(request) = self.mailbox.try_dispatch(&mut self.context.signals) {
            match request {
                Some(AppRequest::EditGraph(edit)) => {
                    self.context.graph.edit(edit.clone()).unwrap();
                    edits.push(edit);
                }
                Some(AppRequest::RefreshCatalog) | None => {}
                other => panic!("unexpected queued-strip request: {other:?}"),
            }
        }
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.sync_node_controls(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
        });
        cx.run_until_parked();
        edits
    }
}

fn fixture(
    cx: &mut TestAppContext,
    kind: NodeKind,
    inspector: bool,
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
    context
        .graph
        .edit(GraphEdit::SetMixer {
            node: id,
            state: MixerState {
                balance: 0.25,
                dimmed: true,
                channel_mode: 1,
                ..Default::default()
            },
        })
        .unwrap();
    if kind == NodeKind::Source {
        context
            .graph
            .edit(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Tone { frequency_hz: 440. },
            })
            .unwrap();
    }
    if kind == NodeKind::Pan {
        context
            .graph
            .edit(GraphEdit::SetPan {
                node: id,
                pan: 0.375,
            })
            .unwrap();
    }
    // No worker runs: native events can render, but cannot consume or publish edits.
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
            shell.inspector_open = inspector;
            shell.selected_node = Some(id);
            shell.selected_nodes.clear();
            shell.selected_nodes.insert(id);
            cx.notify();
        });
        window.render_frame(cx);
    });
    settle_panels(cx);
    let mut service = ManualService { context, mailbox };
    assert!(service.drain(&view, cx).is_empty());
    (view, cx, service, id)
}

fn type_level(id: NodeId, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(format!("card-param-{id:?}-Level"));
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
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input("-6");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn click_mute(id: NodeId, twice: bool, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let control = format!("card-mute-{id:?}");
        assert!(window.find(control.clone()).visible());
        if twice {
            window.double_click(control, cx);
        } else {
            window.click(control, cx);
        }
    });
    cx.run_until_parked();
}

fn assert_unpublished(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    before: &GraphSnapshot,
) {
    view.read_with(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert_eq!(
            &*state.snapshot.graph, before,
            "native edits must stay queued"
        );
        assert!(state.pending_request, "native controls must admit an edit");
    });
}

fn assert_fields_and_topology(actual: &GraphSnapshot, expected: &GraphSnapshot) {
    assert_eq!(
        actual.graph, expected.graph,
        "strip edits must not alter topology"
    );
    assert_eq!(actual.exports, expected.exports);
    assert_eq!(actual.rack_epochs, expected.rack_epochs);
    assert_eq!(
        actual.nodes, expected.nodes,
        "preserve sibling fields, source binding and all other nodes"
    );
    assert!(actual.last_error.is_none());
}

fn assert_level_and_mute_compose(cx: &mut TestAppContext, mute_first: bool) {
    // Tone uses the common card actions, Sink the IO card, and Bus the level fader.
    for kind in [NodeKind::Source, NodeKind::Sink, NodeKind::Bus] {
        let (view, cx, mut service, id) = fixture(cx, kind, false);
        let before = service.context.graph.snapshot();
        let consumed = view.read_with(cx, |shell, cx| {
            shell.state.read(cx).snapshot.consumed_request_id
        });
        if mute_first {
            click_mute(id, false, cx);
            assert_unpublished(&view, cx, &before);
            type_level(id, cx);
        } else {
            type_level(id, cx);
            assert_unpublished(&view, cx, &before);
            click_mute(id, false, cx);
        }
        assert_unpublished(&view, cx, &before);
        view.read_with(cx, |shell, cx| {
            assert_eq!(shell.state.read(cx).snapshot.consumed_request_id, consumed);
        });
        let edits = service.drain(&view, cx);
        assert!(!edits.is_empty());
        let mut expected = before;
        let target = expected
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap();
        target.mixer.gain = 10f64.powf(-6. / 20.);
        target.mixer.muted = true;
        let actual = service.context.graph.snapshot();
        assert_eq!(
            actual.node(id).unwrap().mixer,
            target.mixer,
            "{kind:?}, mute_first={mute_first}: queued edits lost a sibling; edits={edits:?}"
        );
        assert_fields_and_topology(&actual, &expected);
    }
}

#[gpui_kit::test]
fn queued_strip_level_then_mute_preserves_both_fields(cx: &mut TestAppContext) {
    assert_level_and_mute_compose(cx, false);
}

#[gpui_kit::test]
fn queued_strip_mute_then_level_preserves_both_fields(cx: &mut TestAppContext) {
    assert_level_and_mute_compose(cx, true);
}

#[gpui_kit::test]
fn queued_strip_mute_double_click_restores_original_fields(cx: &mut TestAppContext) {
    for kind in [NodeKind::Source, NodeKind::Sink, NodeKind::Bus] {
        let (view, cx, mut service, id) = fixture(cx, kind, false);
        let before = service.context.graph.snapshot();
        click_mute(id, true, cx);
        assert_unpublished(&view, cx, &before);
        let edits = service.drain(&view, cx);
        assert!(!edits.is_empty());
        let actual = service.context.graph.snapshot();
        assert_eq!(
            actual.node(id).unwrap().mixer.muted,
            before.node(id).unwrap().mixer.muted,
            "{kind:?}: both native mute clicks must take effect; edits={edits:?}"
        );
        assert_fields_and_topology(&actual, &before);
    }
}

fn assert_pan_double_clicks(cx: &mut TestAppContext, inspector: bool) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Pan, inspector);
    let before = service.context.graph.snapshot();
    let consumed = view.read_with(cx, |shell, cx| {
        shell.state.read(cx).snapshot.consumed_request_id
    });
    let root = if inspector {
        "inspector-parameters".to_owned()
    } else {
        format!("node-controls-{id:?}")
    };
    cx.update(|window, cx| {
        window.render_frame(cx);
        // Card and Inspector deliberately share button IDs: scope native hit tests.
        for control in ["mix-mono", "mix-swap"] {
            let mut surface = window.within(root.clone());
            assert!(surface.find(control).visible());
            surface.double_click(control, cx);
            view.read_with(cx, |shell, cx| {
                assert_eq!(shell.state.read(cx).snapshot.consumed_request_id, consumed);
            });
        }
    });
    cx.run_until_parked();
    assert_unpublished(&view, cx, &before);
    let edits = service.drain(&view, cx);
    assert!(!edits.is_empty());
    let actual = service.context.graph.snapshot();
    let target = actual.node(id).unwrap();
    assert_eq!(
        (target.mono, target.swap),
        (false, false),
        "inspector={inspector}: two native clicks per toggle must cancel; edits={edits:?}"
    );
    assert_fields_and_topology(&actual, &before);
}

#[gpui_kit::test]
fn queued_pan_card_mono_and_swap_double_clicks_restore_original_fields(cx: &mut TestAppContext) {
    assert_pan_double_clicks(cx, false);
}

#[gpui_kit::test]
fn queued_pan_inspector_mono_and_swap_double_clicks_restore_original_fields(
    cx: &mut TestAppContext,
) {
    assert_pan_double_clicks(cx, true);
}

fn type_strip_readout(label: String, text: &str, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(label.clone()).visible());
        window.click(label.clone(), cx);
        window.render_frame(cx);
        let root = window.find(label);
        let inputs: Vec<_> = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect();
        assert_eq!(
            inputs.len(),
            1,
            "the clicked readout must mount its native editor"
        );
        assert!(inputs[0].visible());
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn queued_strip_card_level_and_inspector_balance_mute_preserve_siblings(cx: &mut TestAppContext) {
    for inspector_first in [false, true] {
        let (view, cx, mut service, id) = fixture(cx, NodeKind::Bus, true);
        cx.update(|window, cx| {
            assert!(window.find("inspector-strip-toggle").visible());
            window.click("inspector-strip-toggle", cx);
            window.render_frame(cx);
            assert!(window.find(format!("balance-label-{id:?}")).visible());
            assert!(window.find(format!("strip-mute-{id:?}")).visible());
        });
        let before = service.context.graph.snapshot();
        let consumed = view.read_with(cx, |shell, cx| {
            shell.state.read(cx).snapshot.consumed_request_id
        });
        if !inspector_first {
            type_level(id, cx);
            assert_unpublished(&view, cx, &before);
        }
        type_strip_readout(format!("balance-label-{id:?}"), "-37.5", cx);
        assert_unpublished(&view, cx, &before);
        cx.update(|window, cx| window.click(format!("strip-mute-{id:?}"), cx));
        if inspector_first {
            type_level(id, cx);
        }
        assert_unpublished(&view, cx, &before);
        view.read_with(cx, |shell, cx| {
            assert_eq!(shell.state.read(cx).snapshot.consumed_request_id, consumed);
            assert_eq!(
                shell.mixer.editing_balance, None,
                "native Enter must commit"
            );
        });
        let edits = service.drain(&view, cx);
        let mut expected = before;
        let target = expected
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap();
        target.mixer.gain = 10f64.powf(-6. / 20.);
        target.mixer.balance = -0.375;
        target.mixer.muted = true;
        let actual = service.context.graph.snapshot();
        assert_eq!(
            actual.node(id).unwrap().mixer,
            target.mixer,
            "inspector_first={inspector_first}: precision Enter and Mute must preserve queued siblings; edits={edits:?}"
        );
        assert_fields_and_topology(&actual, &expected);
    }
}

#[gpui_kit::test]
fn queued_strip_mixer_precision_wheel_and_repeated_actions_preserve_siblings(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Source, false);
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
    let before = service.context.graph.snapshot();
    let (consumed, camera) = view.read_with(cx, |shell, cx| {
        (
            shell.state.read(cx).snapshot.consumed_request_id,
            (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
        )
    });
    // Exercise both shared readout adapters through native typing and Enter,
    // then accumulate wheel/toggle intents without publishing either typed edit.
    type_strip_readout(format!("strip-readout-{id:?}"), "-6.25", cx);
    assert_unpublished(&view, cx, &before);
    type_strip_readout(format!("balance-label-{id:?}"), "-37.5", cx);
    assert_unpublished(&view, cx, &before);
    cx.update(|window, cx| {
        for _ in 0..2 {
            let track = format!("balance-track-{id:?}");
            assert!(window.find(track.clone()).visible());
            window.scroll(track, ScrollDelta::Lines(point(0., 1.)), cx);
            for action in ["solo", "dim", "mode"] {
                let button = format!("strip-{action}-{id:?}");
                assert!(window.find(button.clone()).visible());
                window.click(button, cx);
            }
        }
    });
    cx.run_until_parked();
    assert_unpublished(&view, cx, &before);
    view.read_with(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).snapshot.consumed_request_id, consumed);
        assert_eq!(shell.mixer.editing_value, None, "native Enter must commit");
        assert_eq!(
            shell.mixer.editing_balance, None,
            "native Enter must commit"
        );
        assert_eq!(
            (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
            camera
        );
    });
    let edits = service.drain(&view, cx);
    assert_eq!(
        edits.len(),
        10,
        "two typed edits, two notches and six button clicks"
    );
    let mut expected = before;
    let target = expected
        .nodes
        .iter_mut()
        .find(|node| node.id == id)
        .unwrap();
    target.mixer.gain = 10f64.powf(-6.25 / 20.);
    target.mixer.balance = (-0.375 + 0.1) + 0.1;
    target.mixer.channel_mode = 3;
    let actual = service.context.graph.snapshot();
    assert_eq!(
        actual.node(id).unwrap().mixer,
        target.mixer,
        "notches must accumulate after precision Enter, Solo/Dim must toggle twice and Mode must advance twice; edits={edits:?}"
    );
    assert_fields_and_topology(&actual, &expected);
}
