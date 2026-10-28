//! Native acceptance and deterministic mailbox tests for independent Mappers.
use crate::service::{
    AppHandle, AppRequest, BuiltinState, GraphNodeState, ServiceContext, ServiceMailbox,
};
use crate::ui::pages::graph_canvas::CanvasWorld;
use crate::ui::window::{Camera, StudioShell};
use crate::ui::{UiState, settle_panels, shell_with_state};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, Pixels, Role, TestAppContext, VisualTestContext, px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::processor::ChannelMapperSettings;

mod lifecycle;

struct Service {
    context: ServiceContext,
    handle: AppHandle,
    mailbox: ServiceMailbox,
}

impl Service {
    fn publish(&self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
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
    }

    fn drain(&mut self, view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
        while let Some(request) = self.mailbox.try_request() {
            match request {
                AppRequest::EditGraph(edit) => {
                    self.context.graph.edit(edit).unwrap();
                }
                AppRequest::UndoGraph => {
                    assert!(self.context.graph.undo());
                }
                AppRequest::RedoGraph => {
                    assert!(self.context.graph.redo());
                }
                AppRequest::RefreshCatalog => {}
                other => panic!("unexpected Mapper fixture request: {other:?}"),
            }
        }
        self.publish(view, cx);
    }

    fn settings(&self, id: NodeId) -> ChannelMapperSettings {
        self.context
            .graph
            .snapshot()
            .node(id)
            .unwrap()
            .builtin
            .resolved(NodeKind::ChannelMapper)
            .channel_mapper()
            .unwrap()
            .clone()
    }
}

fn fixture(
    cx: &mut TestAppContext,
    inputs: usize,
    outputs: usize,
) -> (Entity<StudioShell>, &mut VisualTestContext, Service, NodeId) {
    let mut context = ServiceContext::default();
    let snapshot = context.graph.snapshot();
    let mut graph = snapshot.graph;
    let id = graph.create_channel_mapper(inputs, outputs).unwrap();
    let mut nodes = snapshot.nodes;
    nodes.push(GraphNodeState {
        id,
        label: "Mapper".into(),
        gain: 1.,
        pan: 0.,
        mono: false,
        swap: false,
        mixer: Default::default(),
        compressor: Default::default(),
        builtin: BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: (0..outputs).map(|output| output % inputs).collect(),
            matrix: Vec::new(),
        }),
        source: None,
        output_endpoint_id: None,
    });
    context.graph.replace(graph, nodes).unwrap();
    // No consumer thread: admission, queued return-to-snapshot values and Busy
    // are deterministic without replacing the real mailbox or graph validation.
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let state = cx.new(|_| UiState::new(handle.clone(), handle.try_snapshot().unwrap()));
    let (view, cx) = shell_with_state(cx, state);
    cx.update(|window, cx| {
        window.resize(size(px(1600.), px(1000.)));
        window.activate_window();
        view.update(cx, |shell, cx| {
            shell.settings_open = false;
            shell.sidebar_open = false;
            shell.inspector_open = true;
            shell.selected_node = Some(id);
            shell.selected_nodes.clear();
            shell.selected_nodes.insert(id);
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
            cx.notify();
        });
        window.render_frame(cx);
    });
    settle_panels(cx);
    frames(cx);
    (
        view,
        cx,
        Service {
            context,
            handle,
            mailbox,
        },
        id,
    )
}

fn frames(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.render_frame(cx));
    }
}

fn root(surface: &str, id: NodeId) -> String {
    format!("mapper-{surface}-{id:?}")
}

fn cell(surface: &str, id: NodeId, output: usize, input: usize) -> String {
    format!("{}-weight-{output}-{input}", root(surface, id))
}

fn click(cx: &mut VisualTestContext, id: impl Into<String>) {
    let id = id.into();
    let point = cx.update(|window, cx| {
        window.render_frame(cx);
        let target = window.find(id);
        assert!(
            target.visible(),
            "control must be visible before interaction"
        );
        target.bounds().center()
    });
    cx.simulate_click(point, Modifiers::default());
    frames(cx);
}

fn type_cell(
    cx: &mut VisualTestContext,
    surface: &str,
    id: NodeId,
    output: usize,
    input: usize,
    text: &str,
    submit: bool,
) {
    let name = cell(surface, id, output, input);
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        let wrapper = window.find(name);
        let inputs = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|entry| {
                entry.role() == Some(Role::TextInput) && entry.path().starts_with(wrapper.path())
            })
            .collect::<Vec<_>>();
        assert_eq!(inputs.len(), 1, "one native input per routing weight");
        assert!(inputs[0].visible());
        inputs[0].bounds().center()
    });
    cx.simulate_click(position, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
    if submit {
        cx.simulate_keystrokes("enter");
    }
    frames(cx);
}

fn weight(service: &Service, id: NodeId, output: usize, input: usize) -> f64 {
    let settings = service.settings(id);
    if let Some(row) = settings.matrix.get(output) {
        row[input]
    } else {
        f64::from(u8::from(settings.mapping[output] == input))
    }
}

#[gpui_kit::test]
fn mapper_native_card_and_inspector_mount_independent_exact_fields(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, 2, 2);
    cx.update(|window, _| {
        let snapshots = gpui_kit::base::test_support::snapshots(window);
        for parent in [
            format!("node-controls-{id:?}"),
            "inspector-parameters".into(),
        ] {
            let parent = window.find(parent);
            let count = snapshots
                .iter()
                .filter(|entry| {
                    entry.role() == Some(Role::TextInput) && entry.path().starts_with(parent.path())
                })
                .count();
            assert_eq!(
                count, 4,
                "each 2x2 surface needs four native precision inputs"
            );
        }
    });
    type_cell(cx, "card", id, 0, 1, "-2.5", true);
    type_cell(cx, "inspector", id, 1, 0, "3.25", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 1), -2.5);
    assert_eq!(weight(&service, id, 1, 0), 3.25);
    assert_eq!(weight(&service, id, 0, 0), 1.);
}

#[gpui_kit::test]
fn mapper_native_rectangular_precision_uses_actual_ports_and_finite_signed_domain(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 3, 2);
    type_cell(cx, "card", id, 0, 2, "-2.5", true);
    type_cell(cx, "inspector", id, 1, 1, "1.234567890123e-8", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 2), -2.5);
    assert_eq!(weight(&service, id, 1, 1), 1.234567890123e-8);
    for text in ["NaN", "inf", "-inf", "1e999"] {
        type_cell(cx, "inspector", id, 1, 1, text, true);
        service.drain(&view, cx);
        assert_eq!(weight(&service, id, 1, 1), 1.234567890123e-8);
        cx.update(|window, _| {
            assert!(
                window
                    .find(format!("{}-error", cell("inspector", id, 1, 1)))
                    .visible()
            );
        });
    }
    let before = service.settings(id);
    for preset in ["identity", "swap"] {
        click(cx, format!("{}-{preset}", root("card", id)));
    }
    service.drain(&view, cx);
    assert_eq!(
        service.settings(id),
        before,
        "rectangular presets must not dispatch"
    );
}

#[gpui_kit::test]
fn mapper_native_paging_is_surface_local_and_retains_invalid_draft(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    type_cell(cx, "card", id, 0, 0, "invalid-weight", true);
    click(cx, format!("{}-input-next", root("card", id)));
    type_cell(cx, "card", id, 0, 4, "2.75", true);
    type_cell(cx, "inspector", id, 0, 0, "-1.5", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 4), 2.75);
    assert_eq!(weight(&service, id, 0, 0), -1.5);
    click(cx, format!("{}-input-prev", root("card", id)));
    cx.update(|window, _| {
        assert!(
            window
                .find(format!("{}-error", cell("card", id, 0, 0)))
                .visible()
        );
    });
    type_cell(cx, "card", id, 0, 0, "-3.125", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), -3.125);
}

#[gpui_kit::test]
fn mapper_native_page_departure_commits_valid_draft_once(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, 8, 8);
    type_cell(cx, "card", id, 0, 0, "2.125", false);
    click(cx, format!("{}-input-next", root("card", id)));
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 2.125);
    let revision = service.context.graph.snapshot().candidate_revision;
    frames(cx);
    service.drain(&view, cx);
    assert_eq!(
        service.context.graph.snapshot().candidate_revision,
        revision
    );
}

#[gpui_kit::test]
fn mapper_native_queued_return_and_preset_clicks_keep_final_intent(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, 2, 2);
    type_cell(cx, "card", id, 0, 0, "2", true);
    type_cell(cx, "card", id, 0, 0, "1", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 0), 1.);
    for preset in ["identity", "swap", "identity"] {
        click(cx, format!("{}-{preset}", root("card", id)));
    }
    service.drain(&view, cx);
    assert_eq!(service.settings(id), ChannelMapperSettings::identity(2));
}

#[gpui_kit::test]
fn mapper_native_busy_preserves_draft_for_unchanged_retry(cx: &mut TestAppContext) {
    let (view, cx, mut service, id) = fixture(cx, 2, 2);
    loop {
        match service.handle.try_request(AppRequest::RefreshCatalog) {
            Ok(()) => {}
            Err(crate::service::DispatchError::Busy) => break,
            other => panic!("unexpected mailbox fill result: {other:?}"),
        }
    }
    type_cell(cx, "card", id, 0, 1, "-2.5", true);
    cx.update(|window, _| {
        assert!(
            window
                .find(format!("{}-error", cell("card", id, 0, 1)))
                .visible()
        );
    });
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 1), 0.);
    cx.simulate_keystrokes("enter");
    frames(cx);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 0, 1), -2.5);
}

#[gpui_kit::test]
fn mapper_native_32_channel_last_page_edits_without_camera_or_selection_changes(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut service, id) = fixture(cx, 32, 32);
    let before = view.read_with(cx, |shell, _| {
        (
            shell.camera.pan_x,
            shell.camera.pan_y,
            shell.camera.zoom,
            shell.selected_node,
        )
    });
    for _ in 0..7 {
        click(cx, format!("{}-input-next", root("card", id)));
        click(cx, format!("{}-output-next", root("card", id)));
    }
    type_cell(cx, "card", id, 31, 31, "-2.75", true);
    type_cell(cx, "inspector", id, 0, 0, "3.125", true);
    service.drain(&view, cx);
    assert_eq!(weight(&service, id, 31, 31), -2.75);
    assert_eq!(weight(&service, id, 0, 0), 3.125);
    view.read_with(cx, |shell, _| {
        assert_eq!(
            (
                shell.camera.pan_x,
                shell.camera.pan_y,
                shell.camera.zoom,
                shell.selected_node
            ),
            before
        );
    });
}

#[gpui_kit::test]
fn mapper_native_multichannel_card_has_bounded_width_and_intrinsic_height(cx: &mut TestAppContext) {
    let (view, cx, _service, id) = fixture(cx, 32, 32);
    view.read_with(cx, |shell, cx| {
        let world = CanvasWorld::measured(
            &shell.state.read(cx).snapshot,
            &shell.node_positions,
            &shell.node_controls.heights,
        );
        let card = world
            .nodes
            .iter()
            .find(|card| card.node_id == Some(id))
            .unwrap();
        assert_eq!(card.width(), 516., "324px matrix + 176px meter + 16px gap");
        assert!(
            card.height() < 520.,
            "paging must bound high-channel card height"
        );
    });
    cx.update(|window, _| {
        let parent = window.find(format!("node-controls-{id:?}")).bounds();
        let inspector = window.find("inspector-parameters").bounds();
        for (surface, bounds) in [("card", parent), ("inspector", inspector)] {
            let editor = window.find(root(surface, id)).bounds();
            let tolerance: Pixels = px(0.5);
            assert!(editor.left() >= bounds.left() - tolerance);
            assert!(editor.right() <= bounds.right() + tolerance);
        }
        let meter = window.find(format!("card-meter-{id:?}")).bounds();
        assert_eq!(meter.size.width, px(176.));
        assert!(meter.top() >= parent.top() - px(0.5));
        assert!(meter.bottom() <= parent.bottom() + px(0.5));
    });
}
