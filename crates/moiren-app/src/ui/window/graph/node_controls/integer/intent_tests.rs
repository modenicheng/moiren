//! Follow-up intent regressions, separate from the baseline meter_integer_ filter.
use super::{
    AppRequest, BuiltinState, Field, Modifiers, NodeId, NodeKind, TestAppContext, TestWindowExt,
    VisualTestContext, blur_and_sync, confirmed, fixture, input_id, inspector_input,
    requested_values, type_value,
};

fn click_preset(cx: &mut VisualTestContext, node: NodeId, frames: usize) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let button = window.find(format!("card-meter-window-{node:?}-{frames}"));
        assert!(button.visible());
        button.bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn integer_intent_explicit_retype_reverses_queued_preset_but_untouched_blur_does_not(
    cx: &mut TestAppContext,
) {
    for retype in [false, true] {
        let (view, cx, mut service, node) = fixture(cx, NodeKind::Meter, 2);
        let key = (node, Field::MeterWindow);
        let input = inspector_input(&view, cx, key);
        assert_eq!(confirmed(&view, cx, key), 480);
        click_preset(cx, node, 128);
        assert_eq!(
            confirmed(&view, cx, key),
            480,
            "no snapshot between surfaces"
        );
        if retype {
            // Retype exactly the displayed baseline, not an intermediate value.
            type_value(cx, key, "480", true);
        } else {
            let position = cx.update(|window, _| window.find(input_id(key)).bounds().center());
            cx.simulate_click(position, Modifiers::default());
        }
        blur_and_sync(&view, cx);
        view.read_with(cx, |_, cx| {
            assert_eq!(input.read(cx).value().as_str(), "480")
        });
        let expected_requests = if retype { vec![128, 480] } else { vec![128] };
        assert_eq!(
            requested_values(service.drain(&view, cx), key),
            expected_requests,
            "explicit Enter is ordered intent; merely leaving the old field is not"
        );
        let expected = if retype { 480 } else { 128 };
        assert_eq!(confirmed(&view, cx, key), expected);
        view.read_with(cx, |_, cx| {
            assert_eq!(input.read(cx).value().as_str(), expected.to_string());
        });
        // Automatic snapshot sync must not create a new user edit either.
        let position = cx.update(|window, _| window.find(input_id(key)).bounds().center());
        cx.simulate_click(position, Modifiers::default());
        blur_and_sync(&view, cx);
        assert!(service.drain(&view, cx).is_empty());
    }
}

#[gpui_kit::test]
fn integer_intent_pending_load_blocks_old_meter_preset_before_same_id_snapshot(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, node) = fixture(cx, NodeKind::Meter, 2);
    let key = (node, Field::MeterWindow);
    let mut replacement = service.context.graph.snapshot();
    let BuiltinState::Meter(settings) = &mut replacement
        .nodes
        .iter_mut()
        .find(|entry| entry.id == node)
        .unwrap()
        .builtin
    else {
        panic!("expected Meter fixture");
    };
    settings.window_frames = 777;
    let document =
        crate::project::ProjectDocument::from_graph("Replacement Meter", &replacement).unwrap();
    assert_eq!(
        document
            .restore_graph()
            .unwrap()
            .graph
            .get_node(node)
            .unwrap()
            .kind(),
        NodeKind::Meter,
        "the replacement must reuse the old Meter NodeId"
    );
    view.update(cx, |shell, cx| {
        let request = shell.state.update(cx, |state, _| {
            state
                .try_dispatch(AppRequest::LoadProject(document.clone()))
                .unwrap()
        });
        // Match stage_project admission without a file picker or filesystem IO.
        shell.pending_project = Some((request, document, "replacement-meter.moiren.json".into()));
        shell.project_busy = true;
    });
    click_preset(cx, node, 128);
    assert_eq!(confirmed(&view, cx, key), 480);
    view.read_with(cx, |shell, _| assert!(shell.pending_project.is_some()));
    let edits = service.consume();
    assert!(
        service.context.loaded_project.is_some(),
        "the real load must be consumed"
    );
    assert_eq!(
        service
            .context
            .graph
            .snapshot()
            .node(node)
            .unwrap()
            .builtin
            .meter()
            .unwrap()
            .window_frames,
        777,
        "an old preset click must not overwrite the loaded same-ID Meter"
    );
    assert!(
        edits.is_empty(),
        "no preset edit may queue behind LoadProject"
    );
    service.publish(&view, cx);
    assert_eq!(confirmed(&view, cx, key), 777);
    view.read_with(cx, |shell, _| assert!(shell.pending_project.is_none()));
}
