//! Delay-only native precision acceptance; service publication stays deterministic.
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
    processor::DelaySettings,
    rack::{RackProcessor, RackSettings, RackSlot},
};

const FIELDS: [(Parameter, &str, f64); 3] = [
    (Parameter::DelayTime, "37.125 ms", 37.125),
    (Parameter::DelayFeedback, "-12.5 %", -0.125),
    (Parameter::DelayMix, "37.5 %", 0.375),
];

fn initial() -> DelaySettings {
    DelaySettings {
        delay_ms: 20.,
        feedback: 0.25,
        mix: 0.5,
        max_delay_ms: 80.,
    }
}

fn set_expected(settings: &mut DelaySettings, parameter: Parameter, value: f64) {
    match parameter {
        Parameter::DelayTime => settings.delay_ms = value,
        Parameter::DelayFeedback => settings.feedback = value,
        Parameter::DelayMix => settings.mix = value,
        _ => panic!("not a Delay parameter: {parameter:?}"),
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
                AppRequest::LoadProject(document) => {
                    self.context.load_project(document).unwrap();
                }
                AppRequest::RefreshCatalog => {}
                other => panic!("unexpected stopped Delay request: {other:?}"),
            }
        }
        self.mailbox
            .publish(crate::service::snapshot(&self.context));
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.sync_node_controls(window, cx);
            });
            window.render_frame(cx);
        });
        cx.run_until_parked();
    }
}

fn configured(kind: NodeKind, builtin: BuiltinState) -> (ServiceContext, NodeId) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode { kind, channels: 2 })
        .unwrap();
    let id = context.graph.snapshot().node_of_kind(kind).unwrap().0.id();
    context
        .graph
        .edit(GraphEdit::SetBuiltin {
            node: id,
            state: builtin,
        })
        .unwrap();
    (context, id)
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
    let (context, id) = configured(NodeKind::Delay, BuiltinState::Delay(initial()));
    let (view, cx, service) = show(cx, context, id);
    (view, cx, service, id)
}

fn settings(view: &Entity<StudioShell>, cx: &mut VisualTestContext, id: NodeId) -> DelaySettings {
    view.read_with(cx, |shell, cx| {
        *shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(id)
            .unwrap()
            .builtin
            .delay()
            .unwrap()
    })
}

fn root(id: NodeId, parameter: Parameter, inspector: bool) -> String {
    if inspector {
        format!("inspector-precision-{id:?}-{parameter:?}")
    } else {
        format!("card-param-{id:?}-{parameter:?}")
    }
}

fn input(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    id: NodeId,
    parameter: Parameter,
    inspector: bool,
) -> Entity<InputState> {
    view.read_with(cx, |shell, _| {
        let inputs = if inspector {
            &shell.node_controls.precision.inputs
        } else {
            &shell.node_controls.inputs
        };
        inputs
            .get(&(id, parameter))
            .expect("surface-local Delay input")
            .clone()
    })
}

fn native_type(root_id: String, text: &str, cx: &mut VisualTestContext) {
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let root = window.find(root_id);
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
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
}

#[gpui_kit::test]
fn delay_precision_three_native_fields_are_exact_and_independent_from_card(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    let mut expected = initial();
    for (index, (parameter, text, value)) in FIELDS.into_iter().enumerate() {
        let inspector = input(&view, cx, id, parameter, true);
        let card = input(&view, cx, id, parameter, false);
        assert_ne!(inspector.entity_id(), card.entity_id());
        view.read_with(cx, |shell, cx| {
            assert!(shell.typing_focus(&inspector.read(cx).focus_handle(cx), cx));
            assert!(shell.typing_focus(&card.read(cx).focus_handle(cx), cx));
        });
        native_type(root(id, parameter, true), text, cx);
        cx.update(|window, cx| assert!(inspector.read(cx).focus_handle(cx).is_focused(window)));
        if index == 1 {
            cx.update(|window, cx| window.blur(cx));
        } else {
            cx.simulate_keystrokes("enter");
        }
        cx.run_until_parked();
        service.drain(&view, cx);
        set_expected(&mut expected, parameter, value);
        assert_eq!(settings(&view, cx, id), expected);
    }
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("delay-precision-{id:?}")).visible());
        assert!(window.try_find("delay-time-up").is_none());
        assert!(window.try_find("delay-feedback-down").is_none());
    });
}

#[gpui_kit::test]
fn delay_precision_queued_fields_merge_on_both_surfaces_and_keep_custom_capacity(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    for inspector in [false, true] {
        let before = settings(&view, cx, id);
        let fields = if inspector {
            [
                (Parameter::DelayTime, "61.625 ms", 61.625),
                (Parameter::DelayFeedback, "25 %", 0.25),
                (Parameter::DelayMix, "12.5 %", 0.125),
            ]
        } else {
            FIELDS
        };
        let mut expected = before;
        for (parameter, text, value) in fields {
            native_type(root(id, parameter, inspector), text, cx);
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            assert_eq!(
                settings(&view, cx, id),
                before,
                "do not poll between queued fields"
            );
            set_expected(&mut expected, parameter, value);
        }
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |shell, cx| shell.sync_node_controls(window, cx));
        });
        cx.run_until_parked();
        for (parameter, text, _) in fields {
            let input = input(&view, cx, id, parameter, inspector);
            cx.update(|_, cx| assert_eq!(input.read(cx).value().as_str(), text));
        }
        service.drain(&view, cx);
        assert_eq!(settings(&view, cx, id), expected);
    }
}

#[gpui_kit::test]
fn delay_precision_feedback_endpoints_and_wet_percent_match_native_rack_adapter(
    cx: &mut TestAppContext,
) {
    let cases = [
        (
            Parameter::DelayFeedback,
            RackParameter::DelayFeedback,
            "-99.9 %",
            -0.999,
        ),
        (
            Parameter::DelayFeedback,
            RackParameter::DelayFeedback,
            "99.9 %",
            0.999,
        ),
        (
            Parameter::DelayMix,
            RackParameter::DelayMix,
            "37.5 %",
            0.375,
        ),
        (Parameter::DelayMix, RackParameter::DelayMix, "0 %", 0.),
        (Parameter::DelayMix, RackParameter::DelayMix, "100 %", 1.),
        (Parameter::DelayTime, RackParameter::DelayTime, "80 ms", 80.),
    ];
    let (mut standalone, id) = configured(NodeKind::Delay, BuiltinState::Delay(initial()));
    let snapshot = standalone.graph.snapshot();
    let node = snapshot.node(id).unwrap();
    let mut expected = initial();
    for (parameter, typed, text, value) in cases {
        let parsed = super::parameter::parse(text, parameter.unit()).unwrap();
        let edit = parameter
            .edit_precision(parsed, NodeKind::Delay, node)
            .unwrap();
        assert_eq!(
            edit,
            GraphEdit::SetBuiltinParameter {
                node: id,
                parameter: typed,
                value
            }
        );
        standalone.graph.edit(edit).unwrap();
        set_expected(&mut expected, parameter, value);
        assert_eq!(
            *standalone
                .graph
                .snapshot()
                .node(id)
                .unwrap()
                .builtin
                .delay()
                .unwrap(),
            expected
        );
    }
    let original = RackSettings {
        slots: vec![
            RackSlot {
                id: 7,
                bypass: true,
                processor: RackProcessor::Delay(initial()),
            },
            RackSlot {
                id: 3,
                bypass: false,
                processor: RackProcessor::Gain { initial: 0.25 },
            },
        ],
        ..Default::default()
    };
    let (context, rack_id) = configured(NodeKind::Rack, BuiltinState::Rack(original.clone()));
    let epoch = context.graph.snapshot().rack_epochs[&rack_id];
    let (view, cx, mut service) = show(cx, context, rack_id);
    let mut expected = original;
    let mut delay = initial();
    for (parameter, _, text, value) in cases {
        native_type(
            format!("rack-Inspector-{rack_id:?}-field-7-Scalar({parameter:?})"),
            text,
            cx,
        );
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        service.drain(&view, cx);
        set_expected(&mut delay, parameter, value);
        expected.slots[0].processor = RackProcessor::Delay(delay);
        view.read_with(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            assert_eq!(graph.rack_epochs[&rack_id], epoch);
            assert_eq!(graph.node(rack_id).unwrap().builtin.rack(), Some(&expected));
        });
    }
}

#[gpui_kit::test]
fn delay_precision_invalid_limits_preserve_drafts_without_clamping_and_recover(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx);
    for (parameter, invalid, recovery, value) in [
        (
            Parameter::DelayTime,
            "80.00000000000003 ms",
            "37.125 ms",
            37.125,
        ),
        (Parameter::DelayTime, "-0.001 ms", "20 ms", 20.),
        (
            Parameter::DelayFeedback,
            "99.90000000000002 %",
            "-12.5 %",
            -0.125,
        ),
        (
            Parameter::DelayFeedback,
            "-99.90000000000002 %",
            "12.5 %",
            0.125,
        ),
        (Parameter::DelayMix, "100.001 %", "37.5 %", 0.375),
        (Parameter::DelayMix, "-0.001 %", "50 %", 0.5),
        (Parameter::DelayTime, "NaN", "61.625 ms", 61.625),
    ] {
        let input = input(&view, cx, id, parameter, true);
        let before = settings(&view, cx, id);
        native_type(root(id, parameter, true), invalid, cx);
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.blur(cx));
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
        native_type(root(id, parameter, true), recovery, cx);
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
fn delay_precision_pending_load_blocks_old_native_inputs_and_delayed_blur(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx);
    let card = input(&view, cx, id, Parameter::DelayFeedback, false);
    let inspector = input(&view, cx, id, Parameter::DelayMix, true);
    let expected = DelaySettings {
        delay_ms: 120.,
        feedback: -0.75,
        mix: 0.875,
        max_delay_ms: 160.,
    };
    view.update(cx, |shell, cx| {
        let mut graph = (*shell.state.read(cx).snapshot.graph).clone();
        graph
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap()
            .builtin = BuiltinState::Delay(expected);
        let document =
            crate::project::ProjectDocument::from_graph("Replacement Delay", &graph).unwrap();
        assert_eq!(
            document
                .restore_graph()
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.builtin.delay().is_some())
                .unwrap()
                .id,
            id
        );
        let request = shell.state.update(cx, |state, _| {
            state
                .try_dispatch(AppRequest::LoadProject(document.clone()))
                .unwrap()
        });
        shell.pending_project = Some((request, document, "replacement-delay.moiren.json".into()));
        shell.project_busy = true;
    });
    for (parameter, surface, text) in [
        (Parameter::DelayFeedback, false, "99.9 %"),
        (Parameter::DelayMix, true, "0 %"),
    ] {
        native_type(root(id, parameter, surface), text, cx);
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.blur(cx));
        cx.run_until_parked();
    }
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected);
    for retired in [card, inspector] {
        cx.update(|_, cx| retired.update(cx, |_, cx| cx.emit(InputEvent::Blur)));
    }
    cx.run_until_parked();
    service.drain(&view, cx);
    assert_eq!(settings(&view, cx, id), expected);
}
