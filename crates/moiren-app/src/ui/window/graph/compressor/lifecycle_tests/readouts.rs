//! Existing click-to-edit entry points followed by real Graph-page input events.
use super::{ManualService, blur, fixture};
use crate::service::{GraphEdit, GraphNodeState};
use crate::ui::window::StudioShell;
use crate::ui::window::mixer::{EditableValueFamily, gain_db_text, start_value_edit};
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Bounds, ClickEvent, Entity, Focusable, Modifiers, Pixels, Role, TestAppContext,
    VisualTestContext, px,
};
use moiren_core::graph::{NodeId, NodeKind};

#[derive(Clone, Copy, Debug)]
enum Field {
    Gain,
    Pan,
    Fader,
    Balance,
}

impl Field {
    fn family(self) -> EditableValueFamily {
        match self {
            Self::Gain => EditableValueFamily::NodeGain,
            Self::Pan => EditableValueFamily::NodePan,
            Self::Fader => EditableValueFamily::Fader,
            Self::Balance => EditableValueFamily::Balance,
        }
    }

    fn kind(self) -> NodeKind {
        if matches!(self, Self::Pan) {
            NodeKind::Pan
        } else {
            NodeKind::Gain
        }
    }

    fn prefill(self, node: &GraphNodeState) -> String {
        match self {
            Self::Gain => gain_db_text(node.gain),
            Self::Pan => format!("{:+.2}", node.pan),
            Self::Fader => gain_db_text(node.mixer.gain),
            Self::Balance => format!("R {:.0}", node.mixer.balance * 100.),
        }
    }

    fn input(self, shell: &StudioShell, id: NodeId) -> Entity<InputState> {
        let inputs = match self {
            Self::Gain => &shell.mixer.node_gain_inputs,
            Self::Pan => &shell.mixer.node_pan_inputs,
            Self::Fader => &shell.mixer.value_inputs,
            Self::Balance => &shell.mixer.balance_inputs,
        };
        inputs[&id].clone()
    }

    fn value(self, node: &GraphNodeState) -> f64 {
        match self {
            Self::Gain => node.gain,
            Self::Pan => node.pan,
            Self::Fader => node.mixer.gain,
            Self::Balance => node.mixer.balance,
        }
    }

    fn edit(self, node: &GraphNodeState, value: f64) -> GraphEdit {
        match self {
            Self::Gain => GraphEdit::SetGain {
                node: node.id,
                gain: value,
            },
            Self::Pan => GraphEdit::SetPan {
                node: node.id,
                pan: value,
            },
            Self::Fader | Self::Balance => {
                let mut state = node.mixer;
                if matches!(self, Self::Fader) {
                    state.gain = value;
                } else {
                    state.balance = value;
                }
                GraphEdit::SetMixer {
                    node: node.id,
                    state,
                }
            }
        }
    }
}

fn open(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    field: Field,
) -> Entity<InputState> {
    if matches!(field, Field::Fader | Field::Balance) {
        cx.update(|window, cx| {
            window.render_frame(cx);
            if window.try_find(format!("strip-mute-{id:?}")).is_none() {
                window.click("inspector-strip-toggle", cx);
            }
        });
    }
    // The legacy label does not publish a test observation. Use its real
    // production activation callback, then test native focus/keystrokes on the
    // mounted Input rather than directly emitting Enter or editing flags.
    cx.update(|window, cx| {
        let activate = start_value_edit(view.clone(), field.family(), id, move |graph| {
            field.prefill(graph.node(id).unwrap())
        });
        activate(&ClickEvent::default(), window, cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |shell, _| field.input(shell, id))
}

fn mounted_editor_bounds(
    input: &Entity<InputState>,
    field: Field,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            input.read(cx).focus_handle(cx).is_focused(window),
            "{field:?} must keep focus after Graph render"
        );
        let editor = window.find(("editable-editor", input.entity_id()));
        let bounds = editor.bounds();
        assert!(
            editor.visible(),
            "{field:?} editor must be visible: {bounds:?}"
        );
        assert!(
            bounds.size.width > px(0.) && bounds.size.height > px(0.),
            "{field:?} editor must have a real hit area: {bounds:?}"
        );
        let inputs: Vec<_> = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|observed| {
                observed.role() == Some(Role::TextInput)
                    && observed.path().starts_with(editor.path())
            })
            .collect();
        assert_eq!(inputs.len(), 1, "{field:?} must mount one native input");
        let native = inputs[0].bounds();
        assert!(
            inputs[0].visible(),
            "{field:?} native input hidden: {native:?}"
        );
        assert!(
            native.size.width > px(0.) && native.size.height > px(0.),
            "{field:?} native input collapsed inside {bounds:?}: {native:?}"
        );
        native
    })
}

fn focus_and_type(cx: &mut TestAppContext, field: Field) {
    let (view, cx, mut service, id) = fixture(cx, field.kind());
    let before = service.context.graph.snapshot();
    let input = open(&view, cx, id, field);
    mounted_editor_bounds(&input, field, cx);
    view.read_with(cx, |shell, cx| {
        assert!(
            shell.typing_focus(&input.read(cx).focus_handle(cx), cx),
            "{field:?} must own Graph shortcuts"
        )
    });
    cx.simulate_keystrokes("ctrl-a delete");
    cx.run_until_parked();
    assert!(
        service.drain(&view, cx).is_empty(),
        "text deletion must never enqueue node deletion"
    );
    assert_eq!(service.context.graph.snapshot(), before);
    view.read_with(cx, |_, cx| assert!(input.read(cx).value().is_empty()));
    let text = if matches!(field, Field::Gain | Field::Fader) {
        "-7.125"
    } else {
        "R 12.3456"
    };
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(service.drain(&view, cx).len(), 1);
    let expected = if matches!(field, Field::Gain | Field::Fader) {
        10f64.powf(-7.125 / 20.)
    } else {
        0.123456
    };
    assert!(
        (field.value(service.context.graph.snapshot().node(id).unwrap()) - expected).abs() < 1e-12
    );
}

#[gpui_kit::test]
fn legacy_inspector_gain_keeps_native_focus_and_delete_edits_text(cx: &mut TestAppContext) {
    focus_and_type(cx, Field::Gain);
}

#[gpui_kit::test]
fn legacy_inspector_pan_keeps_native_focus_and_delete_edits_text(cx: &mut TestAppContext) {
    focus_and_type(cx, Field::Pan);
}

#[gpui_kit::test]
fn legacy_inspector_fader_keeps_native_focus_and_delete_edits_text(cx: &mut TestAppContext) {
    focus_and_type(cx, Field::Fader);
}

#[gpui_kit::test]
fn legacy_inspector_balance_keeps_native_focus_and_delete_edits_text(cx: &mut TestAppContext) {
    focus_and_type(cx, Field::Balance);
}

fn set(service: &mut ManualService, field: Field, id: NodeId, value: f64) {
    let edit = field.edit(service.context.graph.snapshot().node(id).unwrap(), value);
    service.context.graph.edit(edit).unwrap();
}

#[gpui_kit::test]
fn legacy_inspector_untouched_blur_preserves_precision_and_undo_history(cx: &mut TestAppContext) {
    for field in [Field::Gain, Field::Pan, Field::Fader, Field::Balance] {
        let (view, cx, mut service, id) = fixture(cx, field.kind());
        set(&mut service, field, id, 0.123456789);
        service.publish(&view, cx);
        let before = service.context.graph.snapshot();
        open(&view, cx, id, field);
        blur(cx);
        assert!(
            service.drain(&view, cx).is_empty(),
            "opening and leaving {field:?} is not an edit"
        );
        assert_eq!(
            service.context.graph.snapshot(),
            before,
            "untouched {field:?} must not quantize or create undo history"
        );
    }
}

#[gpui_kit::test]
fn legacy_inspector_untouched_field_does_not_revert_a_newer_card_value(cx: &mut TestAppContext) {
    let field = Field::Gain;
    let (view, cx, mut service, id) = fixture(cx, field.kind());
    let input = open(&view, cx, id, field);
    set(&mut service, field, id, 0.123456789);
    service.publish(&view, cx);
    blur(cx);
    assert!(
        service.drain(&view, cx).is_empty(),
        "an untouched focus owner must not dispatch its old text"
    );
    assert_eq!(
        field.value(service.context.graph.snapshot().node(id).unwrap()),
        0.123456789
    );
    // The same native editor remains usable after the external update.
    open(&view, cx, id, field);
    let position = mounted_editor_bounds(&input, field, cx).center();
    cx.simulate_click(position, Modifiers::default());
    mounted_editor_bounds(&input, field, cx);
    view.read_with(cx, |shell, _| {
        assert_eq!(
            shell.mixer.editing_node_gain,
            Some(id),
            "clicking the native input must not trigger outside-click commit"
        );
    });
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("-7.125");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(service.drain(&view, cx).len(), 1);
}
