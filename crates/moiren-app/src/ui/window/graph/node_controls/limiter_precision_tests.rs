//! Limiter acceptance; declare in node_controls after per-node UI hookup.
use super::{Parameter, StudioShell};
use crate::service::{
    AppRequest, BuiltinState, GraphEdit, RackEdit, RackParameter, ServiceContext, ServiceMailbox,
};
use crate::ui::window::Camera;
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Focusable, Modifiers, Role, TestAppContext, VisualTestContext, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::{
    processor::LimiterSettings,
    rack::{RackProcessor, RackRoute, RackSettings, RackSlot},
};

const FIELDS: [(Parameter, RackParameter, &str, f64); 3] = [
    (
        Parameter::LimitInput,
        RackParameter::LimitInput,
        "7.125 dB",
        7.125,
    ),
    (
        Parameter::LimitThreshold,
        RackParameter::LimitThreshold,
        "-3.625 dB",
        -3.625,
    ),
    (
        Parameter::LimitRelease,
        RackParameter::LimitRelease,
        "321.875 ms",
        321.875,
    ),
];
const EXPECTED: LimiterSettings = LimiterSettings {
    input_gain_db: 7.125,
    threshold_db: -3.625,
    release_ms: 321.875,
};

struct ManualService {
    context: ServiceContext,
    mailbox: ServiceMailbox,
}

impl ManualService {
    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> Vec<GraphEdit> {
        let mut edits = Vec::new();
        // Consume subscriptions too: they share the real mailbox with edits.
        while let Some(request) = self.mailbox.try_dispatch(&mut self.context.signals) {
            match request {
                Some(AppRequest::EditGraph(edit)) => {
                    self.context.graph.edit(edit.clone()).unwrap();
                    edits.push(edit);
                }
                Some(AppRequest::RefreshCatalog) | None => {}
                other => panic!("unexpected Limiter fixture request: {other:?}"),
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
    rack: Option<RackSettings>,
) -> (
    Entity<StudioShell>,
    &mut VisualTestContext,
    ManualService,
    NodeId,
) {
    let mut context = ServiceContext::default();
    let kind = if rack.is_some() {
        NodeKind::Rack
    } else {
        NodeKind::Limiter
    };
    context
        .graph
        .edit(GraphEdit::AddNode { kind, channels: 2 })
        .unwrap();
    let id = context.graph.snapshot().nodes.last().unwrap().id;
    if let Some(rack) = rack {
        context
            .graph
            .edit(GraphEdit::SetBuiltin {
                node: id,
                state: BuiltinState::Rack(rack),
            })
            .unwrap();
    }
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
            shell.inspector_open = true;
            shell.selected_node = Some(id);
            shell.selected_nodes.clear();
            shell.selected_nodes.insert(id);
            if kind == NodeKind::Rack {
                shell.node_controls.rack.selected.insert(id, 7);
            }
            cx.notify();
        });
        window.render_frame(cx);
    });
    settle_panels(cx);
    (view, cx, ManualService { context, mailbox }, id)
}

fn root(id: NodeId, parameter: Parameter) -> String {
    format!("inspector-precision-{id:?}-{parameter:?}")
}

fn intent(node: NodeId, parameter: RackParameter, value: f64) -> GraphEdit {
    GraphEdit::SetBuiltinParameter {
        node,
        parameter,
        value,
    }
}

fn native_type(cx: &mut VisualTestContext, root: String, text: &str, enter: bool) {
    let point = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(root);
        assert!(root.visible());
        let inputs: Vec<_> = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|input| {
                input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
            })
            .collect();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].visible());
        assert!(inputs[0].bounds().left() >= root.bounds().left());
        assert!(inputs[0].bounds().right() <= root.bounds().right());
        inputs[0].bounds().center()
    });
    cx.simulate_click(point, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
    if enter {
        cx.simulate_keystrokes("enter");
    }
    cx.run_until_parked();
}

fn blur(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.blur(cx);
        view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
    });
    cx.run_until_parked();
}

fn settings(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> LimiterSettings {
    view.read_with(cx, |shell, cx| {
        *shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(id)
            .unwrap()
            .builtin
            .limiter()
            .unwrap()
    })
}

#[test]
fn limiter_precision_fields_use_independent_intents_and_exact_engine_domains() {
    let mut service = ServiceContext::default();
    service
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Limiter,
            channels: 2,
        })
        .unwrap();
    let snapshot = service.graph.snapshot();
    let node = snapshot.nodes.last().unwrap();
    // Generate every edit from one old projection: sibling values must survive.
    for (parameter, expected, _, value) in FIELDS {
        let edit = parameter
            .edit_precision(value, NodeKind::Limiter, node)
            .unwrap();
        assert_eq!(edit, intent(node.id, expected, value));
        service.graph.edit(edit).unwrap();
    }
    assert_eq!(
        service
            .graph
            .snapshot()
            .node(node.id)
            .unwrap()
            .builtin
            .limiter(),
        Some(&EXPECTED)
    );
    for ((parameter, _, _, _), (min, max)) in
        FIELDS
            .into_iter()
            .zip([(-60., 60.), (-60., 0.), (0., 5000.)])
    {
        assert_eq!(parameter.range(node), (min, max));
        for value in [min, max] {
            service
                .graph
                .edit(
                    parameter
                        .edit_precision(value, NodeKind::Limiter, node)
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                parameter.value(
                    NodeKind::Limiter,
                    service.graph.snapshot().node(node.id).unwrap()
                ),
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
                    .edit_precision(value, NodeKind::Limiter, node)
                    .is_none(),
                "{parameter:?}: {value} must not be clamped"
            );
        }
    }
}

#[gpui_kit::test]
fn limiter_precision_three_native_fields_submit_exact_enter_and_blur(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, None);
    for (index, (parameter, expected, text, value)) in FIELDS.into_iter().enumerate() {
        native_type(cx, root(id, parameter), text, index != 1);
        view.read_with(cx, |shell, cx| {
            let inspector = &shell.node_controls.precision.inputs[&(id, parameter)];
            let card = &shell.node_controls.inputs[&(id, parameter)];
            assert_ne!(inspector.entity_id(), card.entity_id());
            assert!(shell.typing_focus(&inspector.read(cx).focus_handle(cx), cx));
        });
        blur(&view, cx);
        assert_eq!(service.drain(&view, cx), [intent(id, expected, value)]);
    }
    assert_eq!(settings(&view, cx, id), EXPECTED);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("limiter-precision-{id:?}")).visible());
    });
}

#[gpui_kit::test]
fn limiter_precision_queued_card_and_inspector_fields_preserve_pending_reversal(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, None);
    let before = settings(&view, cx, id);
    let parameter = Parameter::LimitInput;
    native_type(
        cx,
        format!("card-param-{id:?}-{parameter:?}"),
        FIELDS[0].2,
        true,
    );
    for (parameter, _, text, _) in &FIELDS[1..] {
        native_type(cx, root(id, *parameter), text, true);
    }
    blur(&view, cx);
    assert_eq!(
        settings(&view, cx, id),
        before,
        "the mailbox has not been drained"
    );
    view.read_with(cx, |shell, cx| {
        for (parameter, _, text, _) in &FIELDS[1..] {
            assert_eq!(
                shell.node_controls.precision.inputs[&(id, *parameter)]
                    .read(cx)
                    .value()
                    .as_str(),
                *text
            );
        }
    });
    native_type(
        cx,
        root(id, Parameter::LimitThreshold),
        &format!("{} dB", before.threshold_db),
        true,
    );
    let mut expected_edits: Vec<_> = FIELDS
        .map(|(_, parameter, _, value)| intent(id, parameter, value))
        .into();
    expected_edits.push(intent(
        id,
        RackParameter::LimitThreshold,
        before.threshold_db,
    ));
    assert_eq!(service.drain(&view, cx), expected_edits);
    assert_eq!(
        settings(&view, cx, id),
        LimiterSettings {
            threshold_db: before.threshold_db,
            ..EXPECTED
        }
    );
}

#[gpui_kit::test]
fn limiter_precision_invalid_native_drafts_survive_sync_and_retry_on_blur(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, None);
    for ((parameter, expected, recovery, value), invalid) in
        FIELDS.into_iter().zip(["60.01 dB", "0.01 dB", "NaN"])
    {
        let before = settings(&view, cx, id);
        native_type(cx, root(id, parameter), invalid, true);
        blur(&view, cx);
        assert!(service.drain(&view, cx).is_empty());
        assert_eq!(settings(&view, cx, id), before);
        view.read_with(cx, |shell, cx| {
            assert_eq!(
                shell.node_controls.precision.inputs[&(id, parameter)]
                    .read(cx)
                    .value()
                    .as_str(),
                invalid
            );
            assert!(
                shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            );
        });
        native_type(cx, root(id, parameter), recovery, false);
        blur(&view, cx);
        assert_eq!(service.drain(&view, cx), [intent(id, expected, value)]);
        view.read_with(cx, |shell, _| {
            assert!(
                !shell
                    .node_controls
                    .precision
                    .errors
                    .contains_key(&(id, parameter))
            )
        });
    }
    assert_eq!(settings(&view, cx, id), EXPECTED);
}

#[gpui_kit::test]
fn limiter_precision_rack_projection_keeps_leaf_units_siblings_routes_and_epoch(
    cx: &mut TestAppContext,
) {
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
                processor: RackProcessor::Limiter(Default::default()),
            },
        ],
        routes: vec![RackRoute {
            input_port: 0,
            output_port: 0,
            slots: vec![3, 7],
        }],
    };
    let (view, cx, mut service, id) = fixture(cx, Some(original.clone()));
    let epoch = service.context.graph.snapshot().rack_epochs[&id];
    for (parameter, _, text, _) in FIELDS {
        native_type(
            cx,
            format!("rack-Inspector-{id:?}-field-7-Scalar({parameter:?})"),
            text,
            true,
        );
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
            )
        });
    }
    assert_eq!(
        service.drain(&view, cx),
        FIELDS.map(|(_, parameter, _, value)| GraphEdit::EditRack {
            node: id,
            epoch,
            edit: RackEdit::SetParameter {
                slot: 7,
                parameter,
                value
            },
        })
    );
    let mut expected = original;
    expected.slots[1].processor = RackProcessor::Limiter(EXPECTED);
    view.read_with(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert_eq!(graph.rack_epochs[&id], epoch);
        assert_eq!(graph.node(id).unwrap().builtin.rack(), Some(&expected));
    });
}
