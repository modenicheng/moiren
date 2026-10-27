use super::{Parameter, StudioShell};
use crate::service::{AppRequest, GraphEdit, InputSelection};
use crate::ui::window::Camera;
use crate::ui::{finish_service, settle_panels, test_shell, wait_until};
use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, Focusable, Modifiers, Role, TestAppContext, VisualTestContext, px, size};
use moiren_core::graph::{NodeId, NodeKind};
use std::time::Duration;

mod card_regressions;

fn flush(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    wait_until(Duration::from_secs(3), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.poll();
                assert!(state.dispatch_error.is_none(), "{:?}", state.dispatch_error);
                !state.pending_request
            })
        })
    });
    cx.update(|window, cx| window.render_frame(cx));
}

fn show_tone(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> NodeId {
    let id = view.update(cx, |shell, cx| {
        let id = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::Source)
            .unwrap()
            .0
            .id();
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Tone { frequency_hz: 440. },
            }));
        });
        id
    });
    flush(view, cx);
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
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("inspector-binding-toggle", cx);
        window.render_frame(cx);
    });
    id
}

fn frequency(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> f64 {
    view.read_with(cx, |shell, cx| {
        let Some(InputSelection::Tone { frequency_hz }) =
            shell.state.read(cx).snapshot.graph.node(id).unwrap().source
        else {
            panic!("tone binding unexpectedly replaced")
        };
        frequency_hz
    })
}

fn type_inspector(cx: &mut VisualTestContext, id: NodeId, text: &str) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(format!("tone-precision-{id:?}"));
        let inputs = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            inputs.len(),
            1,
            "Inspector owns exactly one native frequency input"
        );
        assert!(inputs[0].visible());
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
}

fn submit_card(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: Parameter,
    text: &str,
) {
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            let input = shell.node_controls.inputs[&(id, parameter)].clone();
            window.focus(&input.read(cx).focus_handle(cx), cx);
            input.update(cx, |input, cx| {
                input.set_value(text.to_owned(), window, cx);
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    flush(view, cx);
}

#[gpui_kit::test]
fn tone_precision_card_accepts_service_domain_without_changing_knob_travel(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = show_tone(&view, cx);
    for (text, expected) in [
        ("1 Hz", 1.),
        ("23999 Hz", 23999.),
        ("1.234567k Hz", 1234.567),
    ] {
        submit_card(&view, cx, id, Parameter::Frequency, text);
        assert_eq!(frequency(&view, cx, id), expected);
    }
    for text in ["0", "24000", "NaN"] {
        submit_card(&view, cx, id, Parameter::Frequency, text);
        assert_eq!(frequency(&view, cx, id), 1234.567);
        view.read_with(cx, |shell, _| {
            let error = &shell.node_controls.errors[&(id, Parameter::Frequency)];
            assert!(error.contains("24000") && error.contains('<'), "{error}");
        });
    }
    view.read_with(cx, |shell, cx| {
        let node = shell.state.read(cx).snapshot.graph.node(id).unwrap();
        assert_eq!(Parameter::Frequency.range(node), (20., 20_000.));
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn tone_precision_card_level_supports_quiet_db_and_silence(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let id = show_tone(&view, cx);
    for (text, expected) in [
        ("-100 dB", 1e-5),
        ("-120 dB", 1e-6),
        ("24 dB", 10f64.powf(24. / 20.)),
        ("-inf dB", 0.),
    ] {
        submit_card(&view, cx, id, Parameter::Level, text);
        view.read_with(cx, |shell, cx| {
            let gain = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .mixer
                .gain;
            assert!((gain - expected).abs() < 1e-12, "{text}: {gain}");
        });
    }
    for text in ["-120.1 dB", "24.01 dB"] {
        submit_card(&view, cx, id, Parameter::Level, text);
        view.read_with(cx, |shell, cx| {
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(id)
                    .unwrap()
                    .mixer
                    .gain,
                0.
            );
            assert!(
                shell
                    .node_controls
                    .errors
                    .contains_key(&(id, Parameter::Level))
            );
        });
    }
    finish_service(runtime);
}

#[gpui_kit::test]
fn tone_precision_inspector_exact_enter_blur_and_invalid_recovery(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let id = show_tone(&view, cx);
    for (text, expected) in [
        ("432.123456789 Hz", 432.123456789),
        ("1", 1.),
        ("23999", 23999.),
    ] {
        type_inspector(cx, id, text);
        cx.simulate_keystrokes("enter");
        flush(&view, cx);
        assert_eq!(frequency(&view, cx, id), expected);
    }
    for text in ["0", "24000", "invalid"] {
        type_inspector(cx, id, text);
        cx.simulate_keystrokes("enter");
        flush(&view, cx);
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
            window.render_frame(cx);
            assert!(
                window
                    .find(format!("tone-frequency-error-{id:?}"))
                    .visible()
            );
        });
        flush(&view, cx);
        assert_eq!(frequency(&view, cx, id), 23999.);
        view.read_with(cx, |shell, cx| {
            let input = &shell.node_controls.precision.inputs[&(id, Parameter::Frequency)];
            assert_eq!(
                input.read(cx).value().as_str(),
                text,
                "invalid drafts survive blur and synchronization"
            );
        });
    }
    type_inspector(cx, id, "1.234567k Hz");
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        window.render_frame(cx);
    });
    flush(&view, cx);
    assert_eq!(frequency(&view, cx, id), 1234.567);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(format!("tone-frequency-error-{id:?}"))
                .is_none()
        );
        assert!(window.try_find("tone-up").is_none());
        assert!(window.try_find("tone-down").is_none());
        window.click("inspector-binding-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find(format!("tone-precision-{id:?}")).is_none());
        assert!(window.find("inspector-routing-toggle").visible());
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn tone_precision_inspector_has_independent_focus_and_untouched_blur_keeps_card_edit(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = show_tone(&view, cx);
    let card = view.read_with(cx, |shell, _| {
        shell.node_controls.inputs[&(id, Parameter::Frequency)].clone()
    });
    let inspector = view.read_with(cx, |shell, _| {
        shell.node_controls.precision.inputs[&(id, Parameter::Frequency)].clone()
    });
    assert_ne!(card.entity_id(), inspector.entity_id());
    type_inspector(cx, id, "invalid");
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        let focused = window.focused(cx).expect("Inspector input focus");
        assert_ne!(
            focused,
            card.read(cx).focus_handle(cx),
            "card and Inspector must not mount one input twice"
        );
        view.read_with(cx, |shell, cx| assert!(shell.typing_focus(&focused, cx)));
    });
    cx.simulate_keystrokes("ctrl-a delete");
    flush(&view, cx);
    assert_eq!(frequency(&view, cx, id), 440.);
    type_inspector(cx, id, "440");
    cx.simulate_keystrokes("enter");
    flush(&view, cx);
    // Change the card's value while Inspector retains focused, untouched text.
    cx.update(|window, cx| {
        card.update(cx, |input, cx| {
            input.set_value("1000", window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    flush(&view, cx);
    assert_eq!(frequency(&view, cx, id), 1000.);
    cx.update(|window, cx| {
        window.blur(cx);
        window.render_frame(cx);
    });
    flush(&view, cx);
    assert_eq!(
        frequency(&view, cx, id),
        1000.,
        "untouched Inspector text must not restore 440 Hz"
    );
    finish_service(runtime);
}

#[gpui_kit::test]
fn tone_precision_external_tiny_values_survive_untouched_blur_and_binding_changes(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    let id = show_tone(&view, cx);
    for value in [0.0000001, 23999.125] {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::BindSource {
                    node: id,
                    source: InputSelection::Tone {
                        frequency_hz: value,
                    },
                }))
            });
        });
        flush(&view, cx);
        let (input, revision) = view.read_with(cx, |shell, cx| {
            let input = shell.node_controls.precision.inputs[&(id, Parameter::Frequency)].clone();
            let card = &shell.node_controls.inputs[&(id, Parameter::Frequency)];
            assert_eq!(input.read(cx).value().as_str(), value.to_string());
            assert_eq!(card.read(cx).value().as_str(), value.to_string());
            (
                input,
                shell.state.read(cx).snapshot.graph.candidate_revision,
            )
        });
        cx.update(|window, cx| {
            window.focus(&input.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        });
        flush(&view, cx);
        assert_eq!(frequency(&view, cx, id), value);
        view.read_with(cx, |shell, cx| {
            assert_eq!(
                shell.state.read(cx).snapshot.graph.candidate_revision,
                revision
            );
        });
    }
    let old = view.read_with(cx, |shell, _| {
        shell.node_controls.precision.inputs[&(id, Parameter::Frequency)].clone()
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::ClearBinding(id)))
        });
    });
    flush(&view, cx);
    cx.update(|window, cx| {
        old.update(cx, |input, cx| {
            input.set_value("900", window, cx);
            cx.emit(InputEvent::Blur);
        });
    });
    flush(&view, cx);
    view.read_with(cx, |shell, cx| {
        assert!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .source
                .is_none()
        );
        assert!(
            !shell
                .node_controls
                .precision
                .inputs
                .contains_key(&(id, Parameter::Frequency))
        );
        assert!(
            !shell
                .node_controls
                .precision
                .errors
                .contains_key(&(id, Parameter::Frequency))
        );
    });
    finish_service(runtime);
}
