use super::{Duration, NodeKind, Page, StudioShell, reorder};
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
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
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
    cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
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
    cx.update(|window, _| window.resize(size(px(1024.), px(720.))));
    view.update(cx, |shell, cx| {
        shell.presentation.page = Page::Mixer;
        shell.inspector_open = false;
        shell.mixer.width = 136.;
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    let id = view.update(cx, |shell, _| shell.mixer.order[0]);
    cx.update(|window, _| {
        let sends = window
            .find(SharedString::from(format!("sends-{id:?}")))
            .bounds();
        assert_eq!(sends.size.height, px(130.));
        let strip = window
            .find(SharedString::from(format!("strip-{id:?}")))
            .bounds();
        let levels = window
            .find(SharedString::from(format!("strip-levels-{id:?}")))
            .bounds();
        assert!(levels.size.width <= strip.size.width);
        assert!(levels.bottom() <= strip.bottom());
        let readout = window
            .find(SharedString::from(format!("strip-readout-{id:?}")))
            .bounds();
        assert!(readout.top() >= levels.bottom());
    });
    finish_service(runtime);
}
