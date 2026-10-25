use super::{Duration, NodeKind, Page, StudioShell, reorder};
use crate::ui::widgets::fader::FaderScale;
use crate::ui::window::test_harness::{finish_service, test_shell, wait_until};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Entity, Modifiers, MouseButton, SharedString, TestAppContext, VisualTestContext, point, px,
    size,
};

fn poll(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    wait_until(Duration::from_secs(2), || {
        let pending = view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).pending_request
        });
        cx.update(|window, cx| window.render_frame(cx));
        !pending
    });
}

#[gpui_kit::test]
fn mixer_controls_target_selected_nodes_and_restore_layout(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (source, gain, sink) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let node = |kind| {
            graph
                .nodes()
                .iter()
                .find(|node| node.kind() == kind)
                .unwrap()
                .id()
        };
        (
            node(NodeKind::Source),
            node(NodeKind::Gain),
            node(NodeKind::Sink),
        )
    });
    cx.update(|window, cx| {
        window.click(SharedString::from(format!("strip-mute-{source:?}")), cx);
    });
    poll(&view, cx);
    cx.update(|window, cx| window.click(SharedString::from(format!("strip-mode-{gain:?}")), cx));
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(graph.node(source).unwrap().mixer.muted);
        assert!(!graph.node(gain).unwrap().mixer.muted);
        assert_eq!(graph.node(gain).unwrap().mixer.channel_mode, 1);
        assert_eq!(graph.node(source).unwrap().mixer.channel_mode, 0);
        assert_eq!(graph.active_revision, None);
        reorder(&mut shell.mixer.order, gain, source);
        shell.mixer.width = 208.;
        shell.mixer.dock_width = 420.;
        let document = shell.project_document(cx).unwrap();
        shell.mixer.order.clear();
        shell.mixer.width = 140.;
        shell.apply_project_presentation(&document, cx).unwrap();
        assert_eq!(shell.mixer.order[0], gain);
        assert_eq!(shell.mixer.width, 208.);
        assert_eq!(shell.mixer.dock_width, 420.);
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let dock = window.find("mixer-dock").bounds();
        assert!(
            dock.contains(
                &window
                    .find(SharedString::from(format!("strip-header-{sink:?}")))
                    .bounds()
                    .center()
            )
        );
        let output = window
            .find(SharedString::from(format!("strip-{sink:?}")))
            .bounds();
        assert!((f32::from(output.right() - dock.right())).abs() <= 1.);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn mixer_header_drag_reorders_and_divider_resizes(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let order = view.update(cx, |shell, _| shell.mixer.order.clone());
    let origin = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-header-{:?}", order[1])))
            .bounds()
            .center()
    });
    let target = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("strip-header-{:?}", order[0])))
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        origin + point(px(12.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, _| assert_eq!(shell.mixer.order[0], order[1]));
    cx.update(|window, cx| window.render_frame(cx));
    let handle_position = cx.update(|window, _| window.find("mixer-dock-resize").bounds().center());
    cx.simulate_mouse_down(handle_position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        handle_position - point(px(40.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        handle_position - point(px(40.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    view.update(cx, |shell, _| assert_eq!(shell.mixer.dock_width, 400.));
    finish_service(runtime);
}

#[gpui_kit::test]
fn compact_mixer_keeps_send_controls_and_minimum_width_meters_unclipped(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1024.), px(720.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        shell.mixer.width = 136.;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    cx.update(|window, cx| {
        let sends = window
            .find(SharedString::from(format!("sends-{id:?}")))
            .bounds();
        assert_eq!(sends.size.height, px(80.));
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        let color_bar = window
            .find(SharedString::from(format!("strip-color-{id:?}")))
            .bounds();
        assert!((f32::from(color_bar.left() - strip.left())).abs() <= 1.);
        assert!((f32::from(color_bar.right() - strip.right())).abs() <= 1.);
        let levels = window
            .find(SharedString::from(format!("strip-levels-{id:?}")))
            .bounds();
        let unity = window
            .find(SharedString::from(format!("strip-unity-{id:?}")))
            .bounds();
        let fader = view.read(cx).mixer.faders[&id].read(cx).bounds();
        assert!(
            (f32::from(unity.top() - (fader.top() + fader.size.height * 0.24))).abs() <= 1.,
            "0 dB reference must align with the fader's unity gain position"
        );
        assert!(levels.size.width <= strip.size.width);
        assert!(levels.bottom() <= strip.bottom());
        let readout = window
            .find(SharedString::from(format!("strip-readout-{id:?}")))
            .bounds();
        assert!(readout.bottom() <= levels.top());
        let inputs = window
            .find(SharedString::from(format!("inputs-{id:?}")))
            .bounds();
        assert!(inputs.bottom() <= sends.top());
        assert!(strip.bottom() <= px(720.));
        let footer = window
            .find(SharedString::from(format!("strip-header-{id:?}")))
            .bounds();
        assert!(footer.top() >= levels.bottom());
        let meter_readout = window
            .find(SharedString::from(format!("meter-readout-{id:?}")))
            .bounds();
        assert!(
            meter_readout.bottom() <= footer.top(),
            "peak readout must stay above the channel identity"
        );
    });
    let edges = view.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .edges()
            .iter()
            .filter(|edge| edge.src() == id)
            .map(|edge| edge.id())
            .collect::<Vec<_>>()
    });
    cx.update(|window, _| {
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        for edge in edges {
            let tap = window
                .find(SharedString::from(format!("send-tap-{edge:?}")))
                .bounds();
            assert!(
                tap.right() <= strip.right() - px(6.),
                "tap escaped minimum strip width"
            );
        }
    });
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::AddNode {
                    kind: NodeKind::Gain,
                    channels: 16,
                },
            ));
        });
    });
    poll(&view, cx);
    let surround = view.update(cx, |shell, cx| {
        let id = shell.state.read(cx).snapshot.graph.nodes.last().unwrap().id;
        let first = shell.mixer.order[0];
        reorder(&mut shell.mixer.order, id, first);
        cx.notify();
        id
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let strip = window
            .find(SharedString::from(format!("strip-{surround:?}")))
            .bounds();
        let meter = window
            .find(SharedString::from(format!("meter-reset-{surround:?}")))
            .bounds();
        assert!(meter.left() >= strip.left());
        assert!(
            meter.right() <= strip.right(),
            "actual multichannel bars overflowed the narrow strip"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn routing_actions_preserve_send_gain_select_the_edge_and_keep_apply_explicit(
    cx: &mut TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (edge, gain, dst, ports) = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph.graph;
        let edge = &graph.edges()[0];
        (
            edge.id(),
            edge.params().gain,
            edge.dst(),
            (edge.src_port(), edge.dst_port()),
        )
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("send-mute-{edge:?}")), cx));
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let edge = graph.graph.get_edge(edge).unwrap();
        assert!(edge.params().mute);
        assert_eq!(edge.params().gain, gain);
        assert_eq!(graph.active_revision, None);
    });
    cx.update(|window, cx| window.click(SharedString::from(format!("input-{edge:?}")), cx));
    view.update(cx, |shell, _| {
        assert_eq!(shell.selected_route, Some(ports));
        assert!(shell.inspector_open);
        assert_eq!(shell.selected_node, None);
    });
    // Closing the inspector restores room without changing route identity.
    view.update(cx, |shell, cx| {
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let (slider, bounds) = view.update(cx, |shell, cx| {
        let slider = shell.mixer.faders[&dst].clone();
        let bounds = slider.read(cx).bounds();
        (slider, bounds)
    });
    let minus_six = FaderScale::position(10f64.powf(-6. / 20.));
    let target = point(
        bounds.center().x,
        bounds.top() + bounds.size.height * minus_six,
    );
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let db = 20. * graph.node(dst).unwrap().mixer.gain.log10();
        assert!(
            (db + 6.).abs() < 0.02,
            "shared inverse pointer mapping produced {db} dB"
        );
        assert!(slider.read(cx).value().end().is_finite());
        assert_eq!(graph.active_revision, None);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn balance_drag_targets_its_own_strip_and_double_click_centers_it(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    let bounds = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("balance-track-{id:?}")))
            .bounds()
    });
    let target = point(bounds.left() + bounds.size.width * 0.75, bounds.center().y);
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        bounds.center() + point(px(12.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.render_frame(cx));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        assert!(
            (graph.node(id).unwrap().mixer.balance - 0.5).abs() < 0.02,
            "dragging the first strip must use that strip's balance bounds"
        );
        assert!(
            graph
                .nodes
                .iter()
                .filter(|node| node.id != id)
                .all(|node| node.mixer.balance == 0.),
            "other strips must keep their balance"
        );
        assert_eq!(graph.active_revision, None);
    });
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position: target,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position: target,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    poll(&view, cx);
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(id)
                .unwrap()
                .mixer
                .balance,
            0.
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn send_numeric_edit_and_deleted_edge_cleanup_preserve_shared_layout(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    cx.simulate_resize(size(px(1440.), px(900.)));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let edge = view.update(cx, |shell, cx| {
        shell.state.read(cx).snapshot.graph.graph.edges()[0].id()
    });
    let position = cx.update(|window, _| {
        window
            .find(SharedString::from(format!("send-value-{edge:?}")))
            .bounds()
            .center()
    });
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    cx.update(|window, cx| {
        view.update(cx, |shell, cx| {
            assert_eq!(shell.mixer.editing_send, Some(edge));
            shell.mixer.send_inputs[&edge].update(cx, |input, cx| {
                input.set_value("-18.5", window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    poll(&view, cx);
    let order = view.update(cx, |shell, cx| {
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .graph
            .get_edge(edge)
            .unwrap()
            .params()
            .gain;
        assert!((20. * gain.log10() + 18.5).abs() < 0.00001);
        assert!(shell.mixer.editing_send.is_none());
        shell.mixer.order.clone()
    });
    let toggle = cx.update(|window, _| {
        let bounds = window
            .find(SharedString::from(format!("inputs-{:?}", order[0])))
            .bounds();
        bounds.origin + point(px(40.), px(14.))
    });
    cx.simulate_click(toggle, Modifiers::default());
    cx.update(|window, cx| {
        window.render_frame(cx);
        for id in &order {
            assert_eq!(
                window
                    .find(SharedString::from(format!("inputs-{id:?}")))
                    .bounds()
                    .size
                    .height,
                px(28.)
            );
        }
    });
    view.update(cx, |shell, cx| {
        shell.mixer.editing_send = Some(edge);
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::Disconnect(edge),
            ));
        });
    });
    poll(&view, cx);
    view.update(cx, |shell, _| {
        assert!(shell.mixer.editing_send.is_none());
        assert!(!shell.mixer.send_inputs.contains_key(&edge));
        assert!(!shell.mixer.sends.contains_key(&edge));
    });
    finish_service(runtime);
}
