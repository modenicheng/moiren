//! Gate-only acceptance with real native inputs and a manually drained service.
use super::{Parameter, StudioShell};
use crate::service::{
    AppRequest, BuiltinState, GraphEdit, RackParameter, ServiceContext, ServiceMailbox,
};
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Focusable, Modifiers, Role, TestAppContext, VisualTestContext, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::{
    processor::GateSettings,
    rack::{RackProcessor, RackRoute, RackSettings, RackSlot},
};

const FIELDS: [(Parameter, RackParameter, &str, f64); 5] = [
    (
        Parameter::GateThreshold,
        RackParameter::GateThreshold,
        "-37.25 dB",
        -37.25,
    ),
    (
        Parameter::GateRange,
        RackParameter::GateRange,
        "-73.5 dB",
        -73.5,
    ),
    (
        Parameter::GateAttack,
        RackParameter::GateAttack,
        "13.75 ms",
        13.75,
    ),
    (
        Parameter::GateHold,
        RackParameter::GateHold,
        "42.125 ms",
        42.125,
    ),
    (
        Parameter::GateRelease,
        RackParameter::GateRelease,
        "321.875 ms",
        321.875,
    ),
];

fn expected_settings() -> GateSettings {
    GateSettings {
        threshold_db: -37.25,
        range_db: -73.5,
        attack_ms: 13.75,
        hold_ms: 42.125,
        release_ms: 321.875,
    }
}

fn set_expected(settings: &mut GateSettings, parameter: Parameter, value: f64) {
    match parameter {
        Parameter::GateThreshold => settings.threshold_db = value,
        Parameter::GateRange => settings.range_db = value,
        Parameter::GateAttack => settings.attack_ms = value,
        Parameter::GateHold => settings.hold_ms = value,
        Parameter::GateRelease => settings.release_ms = value,
        _ => panic!("not a Gate parameter: {parameter:?}"),
    }
}

struct ManualService {
    context: ServiceContext,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        while let Some(request) = self.mailbox.try_request() {
            match request {
                AppRequest::EditGraph(edit) => {
                    self.context.graph.edit(edit).unwrap();
                }
                AppRequest::RefreshCatalog => {}
                other => panic!("unexpected stopped Gate request: {other:?}"),
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
    }
}

fn show(
    cx: &mut TestAppContext,
    context: ServiceContext,
    id: NodeId,
) -> (Entity<StudioShell>, &mut VisualTestContext, ManualService) {
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
        if shell.state.read(cx).snapshot.graph.kind_of(id) == Some(NodeKind::Rack) {
            shell.node_controls.rack.selected.insert(id, 7);
        }
        cx.notify();
    });
    settle_panels(cx);
    (view, cx, ManualService { context, mailbox })
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
    context
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Gate,
            channels: 2,
        })
        .unwrap();
    let id = context
        .graph
        .snapshot()
        .node_of_kind(NodeKind::Gate)
        .unwrap()
        .0
        .id();
    let (view, cx, service) = show(cx, context, id);
    (view, cx, service, id)
}

fn settings(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> GateSettings {
    view.read_with(cx, |shell, cx| {
        *shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(id)
            .unwrap()
            .builtin
            .gate()
            .unwrap()
    })
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

fn inspector_input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: Parameter,
) -> Entity<InputState> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(format!("inspector-precision-{id:?}-{parameter:?}"));
        assert!(root.visible());
        let inputs = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].visible());
        assert!(inputs[0].bounds().left() >= root.bounds().left());
        assert!(inputs[0].bounds().right() <= root.bounds().right());
    });
    view.read_with(cx, |shell, _| {
        shell.node_controls.precision.inputs[&(id, parameter)].clone()
    })
}

fn native_type(id: NodeId, parameter: Parameter, text: &str, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find(format!("inspector-precision-input-{id:?}-{parameter:?}"))
            .bounds()
            .center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
}

#[test]
fn gate_precision_parameter_edits_are_independent_service_fields() {
    let mut graph = crate::service::ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Gate,
            channels: 2,
        })
        .unwrap();
    let snapshot = graph.snapshot();
    let (_, node) = snapshot.node_of_kind(NodeKind::Gate).unwrap();
    // All five intents deliberately originate from the same old projection.
    for (parameter, expected, _, value) in FIELDS {
        let edit = parameter
            .edit_precision(value, NodeKind::Gate, node)
            .unwrap();
        assert_eq!(
            edit,
            GraphEdit::SetBuiltinParameter {
                node: node.id,
                parameter: expected,
                value,
            },
        );
        graph.edit(edit).unwrap();
    }
    assert_eq!(
        *graph
            .snapshot()
            .node(node.id)
            .unwrap()
            .builtin
            .gate()
            .unwrap(),
        expected_settings()
    );
}

#[test]
fn gate_precision_domains_accept_exact_endpoints_without_clamping_invalid_values() {
    let mut graph = crate::service::ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Gate,
            channels: 2,
        })
        .unwrap();
    let snapshot = graph.snapshot();
    let (_, node) = snapshot.node_of_kind(NodeKind::Gate).unwrap();
    for (parameter, _, _, _) in FIELDS {
        let (min, max) = if matches!(parameter, Parameter::GateThreshold | Parameter::GateRange) {
            (-120., 0.)
        } else {
            (0., 5000.)
        };
        for value in [min, max] {
            let edit = parameter
                .edit_precision(value, NodeKind::Gate, node)
                .expect("inclusive Gate precision endpoint");
            graph.edit(edit).unwrap();
            assert_eq!(
                parameter.value(NodeKind::Gate, graph.snapshot().node(node.id).unwrap()),
                value
            );
        }
        for value in [
            min - 0.001,
            max + 0.001,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert!(
                parameter
                    .edit_precision(value, NodeKind::Gate, node)
                    .is_none(),
                "{parameter:?}: invalid value {value} must not be clamped"
            );
        }
    }
}

#[gpui_kit::test]
fn gate_precision_inspector_five_exact_fields_use_separate_native_inputs(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let mut expected = settings(&view, cx, id);
    for (index, (parameter, _, text, value)) in FIELDS.into_iter().enumerate() {
        let input = inspector_input(&view, cx, id, parameter);
        view.read_with(cx, |shell, cx| {
            let card = &shell.node_controls.inputs[&(id, parameter)];
            assert_ne!(card.entity_id(), input.entity_id());
            assert!(shell.typing_focus(&input.read(cx).focus_handle(cx), cx));
            assert!(shell.typing_focus(&card.read(cx).focus_handle(cx), cx));
        });
        native_type(id, parameter, text, cx);
        cx.update(|window, cx| assert!(input.read(cx).focus_handle(cx).is_focused(window)));
        if index % 2 == 0 {
            cx.simulate_keystrokes("enter");
        } else {
            cx.update(|window, cx| window.blur(cx));
        }
        cx.run_until_parked();
        service.drain(&view, cx);
        set_expected(&mut expected, parameter, value);
        assert_eq!(settings(&view, cx, id), expected);
    }
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("gate-precision-{id:?}")).visible());
        assert!(window.try_find("gate-threshold-up").is_none());
        assert!(window.try_find("gate-attack-down").is_none());
    });
}

#[gpui_kit::test]
fn gate_precision_card_queued_siblings_merge_without_full_state_replacement(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let original = settings(&view, cx, id);
    for (parameter, _, text, _) in FIELDS {
        let input = view.read_with(cx, |shell, _| {
            shell.node_controls.inputs[&(id, parameter)].clone()
        });
        enter(&input, text, cx);
        assert_eq!(settings(&view, cx, id), original);
    }
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected_settings());
}

#[gpui_kit::test]
fn gate_precision_inspector_queued_siblings_preserve_pending_and_last_intent(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let original = settings(&view, cx, id);
    for (parameter, _, text, _) in FIELDS {
        let input = inspector_input(&view, cx, id, parameter);
        enter(&input, text, cx);
        assert_eq!(settings(&view, cx, id), original);
    }
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
    });
    cx.run_until_parked();
    view.read_with(cx, |shell, cx| {
        for (parameter, _, text, _) in FIELDS {
            assert_eq!(
                shell.node_controls.precision.inputs[&(id, parameter)]
                    .read(cx)
                    .value()
                    .as_str(),
                text
            );
        }
    });
    let threshold = inspector_input(&view, cx, id, Parameter::GateThreshold);
    enter(&threshold, "-40 dB", cx);
    service.drain(&view, cx);
    assert_eq!(
        settings(&view, cx, id),
        GateSettings {
            threshold_db: original.threshold_db,
            ..expected_settings()
        }
    );
}

#[gpui_kit::test]
fn gate_precision_invalid_drafts_recover_and_keep_delete_undo_in_native_input(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    for (parameter, invalid, recovery, value) in [
        (Parameter::GateThreshold, "0.01 dB", "-37.25 dB", -37.25),
        (
            Parameter::GateThreshold,
            "-120.01 dB",
            "-38.125 dB",
            -38.125,
        ),
        (Parameter::GateRange, "0.01 dB", "-73.5 dB", -73.5),
        (Parameter::GateRange, "-120.01 dB", "-74.25 dB", -74.25),
        (Parameter::GateAttack, "-0.01 ms", "13.75 ms", 13.75),
        (Parameter::GateHold, "5000.01 ms", "42.125 ms", 42.125),
        (Parameter::GateRelease, "NaN", "321.875 ms", 321.875),
    ] {
        let input = inspector_input(&view, cx, id, parameter);
        let before = settings(&view, cx, id);
        native_type(id, parameter, invalid, cx);
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        });
        cx.run_until_parked();
        service.drain(&view, cx);
        assert_eq!(settings(&view, cx, id), before);
        view.read_with(cx, |shell, cx| {
            assert_eq!(input.read(cx).value().as_str(), invalid);
            assert!(
                shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            );
        });
        cx.update(|window, cx| {
            window.focus(&input.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
        });
        cx.simulate_keystrokes("ctrl-a delete");
        service.drain(&view, cx);
        assert_eq!(
            settings(&view, cx, id),
            before,
            "Delete must not remove the Gate"
        );
        cx.simulate_keystrokes("ctrl-z");
        service.drain(&view, cx);
        assert_eq!(
            settings(&view, cx, id),
            before,
            "Undo must stay inside the input"
        );
        native_type(id, parameter, recovery, cx);
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
            assert_eq!(input.read(cx).value().as_str(), recovery);
        });
        cx.run_until_parked();
        service.drain(&view, cx);
        let mut expected = before;
        set_expected(&mut expected, parameter, value);
        assert_eq!(settings(&view, cx, id), expected);
        view.read_with(cx, |shell, _| {
            assert!(
                !shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            );
        });
    }
}

#[gpui_kit::test]
fn gate_precision_rack_native_adapter_preserves_siblings_routes_bypass_and_units(
    cx: &mut TestAppContext,
) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Rack,
            channels: 2,
        })
        .unwrap();
    let id = context
        .graph
        .snapshot()
        .node_of_kind(NodeKind::Rack)
        .unwrap()
        .0
        .id();
    let original = RackSettings {
        slots: vec![
            RackSlot {
                id: 3,
                bypass: false,
                processor: RackProcessor::Gain { initial: 0.25 },
            },
            RackSlot {
                id: 7,
                bypass: true,
                processor: RackProcessor::Gate(Default::default()),
            },
            RackSlot {
                id: 11,
                bypass: false,
                processor: RackProcessor::Pan {
                    initial: -0.25,
                    mono: true,
                    swap: false,
                },
            },
        ],
        routes: vec![RackRoute {
            input_port: 0,
            output_port: 0,
            slots: vec![3, 7, 11],
        }],
    };
    context
        .graph
        .edit(GraphEdit::SetBuiltin {
            node: id,
            state: BuiltinState::Rack(original.clone()),
        })
        .unwrap();
    let epoch = context.graph.snapshot().rack_epochs[&id];
    let (view, cx, mut service) = show(cx, context, id);
    for (parameter, _, text, _) in FIELDS {
        let position = cx.update(|window, cx| {
            window.render_frame(cx);
            let root = window.find(format!(
                "rack-Inspector-{id:?}-field-7-Scalar({parameter:?})"
            ));
            assert!(root.visible());
            let inputs = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .filter(|input| {
                    input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
                })
                .collect::<Vec<_>>();
            assert_eq!(inputs.len(), 1);
            assert!(inputs[0].visible());
            inputs[0].bounds().center()
        });
        cx.simulate_click(position, Modifiers::default());
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(text);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        view.read_with(cx, |shell, cx| {
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(id)
                    .unwrap()
                    .builtin
                    .rack(),
                Some(&original)
            );
        });
    }
    service.drain(&view, cx);
    let mut expected = original;
    expected.slots[1].processor = RackProcessor::Gate(expected_settings());
    view.read_with(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.rack_epochs[&id], epoch);
        assert_eq!(graph.node(id).unwrap().builtin.rack(), Some(&expected));
    });
}
