use super::{ServiceRuntime, StudioShell, UiState, pages, theme};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, point, px};

#[gpui_kit::test]
fn graph_uses_full_canvas_and_wire_selection_opens_real_route_inspector(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    });
    let position = view.update(cx, |shell, cx| {
        let (width, height) = shell.canvas_size.expect("canvas has completed layout");
        assert!(
            width > 500. && height > 300.,
            "the drawing surface must fill the workspace"
        );
        assert!(
            shell.camera.zoom > 0.5,
            "Fit must leave this four-node graph readable"
        );
        let world = pages::graph_canvas::CanvasWorld::positioned(
            &shell.state.read(cx).snapshot,
            &shell.node_positions,
        );
        let wire = &world.wires[0];
        point(
            px(if shell.sidebar_open {
                theme::LIBRARY_WIDTH
            } else {
                0.
            } + shell.camera.pan_x
                + (wire.from.0 + wire.to.0) * 0.5 * shell.camera.zoom),
            px(48. + shell.camera.pan_y + wire.from.1 * shell.camera.zoom),
        )
    });
    cx.update(|window, cx| {
        window.click_at("graph-page", position, cx);
        assert!(window.find("disconnect-route").visible());
    });
    view.update(cx, |shell, cx| {
        assert!(shell.selected_route.is_some());
        assert!(shell.selected_node.is_none());
        assert_eq!(
            shell.state.read(cx).snapshot.graph.graph.edges().len(),
            3,
            "selection never mutates topology"
        );
    });
    handle.request_exit();
    runtime.join().unwrap();
}
