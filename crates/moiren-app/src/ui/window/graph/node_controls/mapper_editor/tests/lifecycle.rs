//! Retained callbacks must not cross document or history editor lifetimes.
use super::super::model::{self, Key, Surface};
use super::*;
use crate::service::GraphEdit;
use gpui_kit::Focusable;
use gpui_kit::component::input::{InputEvent, InputState};

fn editor(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    node: NodeId,
    surface: Surface,
) -> (Key, Entity<InputState>) {
    view.read_with(cx, |shell, _| {
        let key = Key {
            surface,
            node,
            incarnation: shell.node_controls.mapper.incarnation(node).unwrap(),
            output: 0,
            input: 0,
        };
        (key, shell.node_controls.mapper.inputs[&key].clone())
    })
}

fn delayed_blur(input: &Entity<InputState>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.set_value("-9.5", window, cx);
            cx.emit(InputEvent::Blur);
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn mapper_native_loaded_document_retires_old_preset_pager_and_weight_callbacks(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    let (old_key, old_input) = editor(&view, cx, id, Surface::Card);
    let old_target = view.read_with(cx, |shell, _| {
        shell.node_controls.mapper.target(id).unwrap()
    });
    service
        .context
        .graph
        .edit(GraphEdit::SetMapperWeight {
            node: id,
            input: 0,
            output: 0,
            weight: 3.25,
        })
        .unwrap();
    let snapshot = service.context.graph.snapshot();
    let document = crate::project::ProjectDocument::from_graph("Replacement", &snapshot).unwrap();
    // Same IDs and dimensions, but the real successful-load presentation reset
    // replaces every editor owner and restarts its local incarnation sequence.
    service
        .context
        .graph
        .replace(snapshot.graph, snapshot.nodes)
        .unwrap();
    service.publish(&view, cx);
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            shell.apply_project_presentation(&document, cx).unwrap();
            shell.selected_node = Some(id);
            shell.camera.zoom = 1.;
            shell.sync_node_controls(window, cx);
        });
    });
    let (fresh_key, fresh_input) = editor(&view, cx, id, Surface::Card);
    assert_eq!(
        old_key.incarnation, fresh_key.incarnation,
        "exercise numeric ID collision"
    );
    assert_ne!(old_input.entity_id(), fresh_input.entity_id());
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            model::preset(shell, &old_target, true, window, cx);
            model::navigate(shell, &old_target, Surface::Card, true, 1, window, cx);
        })
    });
    delayed_blur(&old_input, cx);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 3.25);
    view.read_with(cx, |shell, _| {
        assert_eq!(
            shell.node_controls.mapper.pages[&(Surface::Card, id)],
            (0, 0)
        );
    });
}

#[gpui_kit::test]
fn mapper_native_history_hook_retires_callbacks_before_and_after_publication(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    service
        .context
        .graph
        .edit(GraphEdit::SetMapperWeight {
            node: id,
            input: 0,
            output: 0,
            weight: 2.5,
        })
        .unwrap();
    service.publish(&view, cx);
    let (_, old_input) = editor(&view, cx, id, Surface::Card);
    let old_target = view.read_with(cx, |shell, _| {
        shell.node_controls.mapper.target(id).unwrap()
    });
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| {
            shell.undo_redo_graph(false, window, cx);
            assert!(
                shell.node_controls.mapper.inputs.is_empty(),
                "real history hook retires immediately"
            );
            model::preset(shell, &old_target, true, window, cx);
            model::navigate(shell, &old_target, Surface::Card, true, 1, window, cx);
        });
    });
    delayed_blur(&old_input, cx);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 1.);
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            model::preset(shell, &old_target, true, window, cx);
            model::navigate(shell, &old_target, Surface::Card, true, 1, window, cx);
        })
    });
    delayed_blur(&old_input, cx);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 1.);
    let fresh_target = view.read_with(cx, |shell, _| {
        shell.node_controls.mapper.target(id).unwrap()
    });
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            model::preset(shell, &fresh_target, true, window, cx);
        })
    });
    service.drain(&view, cx);
    assert_eq!(service.settings(id).mapping[..2], [1, 0]);
}

#[gpui_kit::test]
fn mapper_native_disclosure_departure_flushes_without_blur_and_retires_focus(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    let (_, input) = editor(&view, cx, id, Surface::Inspector);
    // Deliberately never focus/blur this field. Collapse must use its own
    // departure hook rather than rely on the native input sending Blur.
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_value("2.125", window, cx)));
    click(cx, "inspector-parameters-toggle");
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 2.125);
    view.read_with(cx, |shell, cx| {
        assert!(!shell.node_controls.mapper_disclosure_open(id));
        assert!(
            !shell
                .node_controls
                .mapper
                .owns_focus(&input.read(cx).focus_handle(cx), cx)
        );
    });
    let revision = service.context.graph.snapshot().candidate_revision;
    frames(cx);
    service.drain(&view, cx);
    assert_eq!(
        service.context.graph.snapshot().candidate_revision,
        revision
    );
    click(cx, "inspector-parameters-toggle");
    let (_, reopened) = editor(&view, cx, id, Surface::Inspector);
    cx.update(|_, cx| assert_eq!(reopened.read(cx).value().as_str(), "2.125"));
}

#[gpui_kit::test]
fn mapper_native_disclosure_retains_invalid_draft_without_hidden_keyboard_owner(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    let (key, input) = editor(&view, cx, id, Surface::Inspector);
    type_cell(cx, "inspector", id, 0, 0, "invalid-weight", true);
    click(cx, "inspector-parameters-toggle");
    service.drain(&view, cx);
    view.read_with(cx, |shell, cx| {
        assert_eq!(
            shell.node_controls.mapper.inputs[&key].entity_id(),
            input.entity_id()
        );
        assert!(shell.node_controls.mapper.errors.contains_key(&key));
        assert!(
            !shell
                .node_controls
                .mapper
                .owns_focus(&input.read(cx).focus_handle(cx), cx)
        );
    });
    click(cx, "inspector-parameters-toggle");
    let (_, reopened) = editor(&view, cx, id, Surface::Inspector);
    assert_eq!(reopened.entity_id(), input.entity_id());
    cx.update(|_, cx| assert_eq!(reopened.read(cx).value().as_str(), "invalid-weight"));
    assert_eq!(weight(&service, id, 0, 0), 1.);
}
