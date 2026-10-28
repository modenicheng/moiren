//! Cross-surface intent through native inputs, without intervening publication.
use super::{blur, compressor_input, fixture, value};
use crate::service::{CompressorParameter, GraphEdit};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Focusable, Modifiers, Role, TestAppContext, VisualTestContext};
use moiren_core::graph::{NodeId, NodeKind};

fn focus_native(id: NodeId, inspector: bool, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(if inspector {
            format!("compressor-input-{}", CompressorParameter::Threshold.id().0)
        } else {
            format!("card-param-{id:?}-Compressor(Threshold)")
        });
        assert!(root.visible());
        let inputs: Vec<_> = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect();
        assert_eq!(inputs.len(), 1, "target the actual native text editor");
        assert!(inputs[0].visible());
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
}

fn retype(text: &str, cx: &mut VisualTestContext) {
    // Real deletion and retyping move the text away from and back to its
    // baseline; a Change notification with identical text proves no intent.
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input(text);
    cx.run_until_parked();
}

fn queue_card_threshold(id: NodeId, cx: &mut VisualTestContext) {
    focus_native(id, false, cx);
    retype("-20", cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn legacy_compressor_explicit_inspector_retype_overrides_queued_card_threshold(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Threshold;
    let input = compressor_input(&view, cx, id, parameter);
    let baseline = input.read_with(cx, |input, _| input.value().to_string());
    let original = value(&service, id, parameter);
    assert_ne!(original, -20.);

    queue_card_threshold(id, cx);
    assert_eq!(value(&service, id, parameter), original);
    focus_native(id, true, cx);
    cx.update(|window, cx| {
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(input.read(cx).value().as_str(), baseline);
    });
    retype(&baseline, cx);
    assert_eq!(
        input.read_with(cx, |input, _| input.value().to_string()),
        baseline
    );
    cx.simulate_keystrokes("enter");
    blur(cx);

    let edits = service.drain(&view, cx);
    assert_eq!(
        value(&service, id, parameter),
        original,
        "explicit baseline retype must supersede the queued card edit; edits={edits:?}"
    );
    assert_eq!(
        edits,
        vec![
            GraphEdit::SetCompressor {
                node: id,
                parameter,
                value: -20.,
            },
            GraphEdit::SetCompressor {
                node: id,
                parameter,
                value: original,
            },
        ],
        "each explicit edit submits once; Enter followed by Blur is not another intent"
    );
}

#[gpui_kit::test]
fn legacy_compressor_untouched_inspector_blur_preserves_queued_card_threshold(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, NodeKind::Compressor);
    let parameter = CompressorParameter::Threshold;
    let input = compressor_input(&view, cx, id, parameter);
    let baseline = input.read_with(cx, |input, _| input.value().to_string());
    let original = value(&service, id, parameter);
    assert_ne!(original, -20.);

    queue_card_threshold(id, cx);
    assert_eq!(value(&service, id, parameter), original);
    focus_native(id, true, cx);
    cx.update(|window, cx| {
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(input.read(cx).value().as_str(), baseline);
    });
    blur(cx);

    assert_eq!(
        service.drain(&view, cx),
        vec![GraphEdit::SetCompressor {
            node: id,
            parameter,
            value: -20.,
        }],
        "untouched Inspector text must not dispatch over a queued card edit"
    );
    assert_eq!(value(&service, id, parameter), -20.);
}
