//! Diagnostic actions must fit the scrollport, not just expose a UIA node.
use super::*;

#[gpui_kit::test]
fn stopped_diagnostics_quit_scrolls_fully_inside_the_workspace(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    for density in [
        super::super::Density::Comfortable,
        super::super::Density::Compact,
    ] {
        for (width, height) in [(1100., 700.), (1440., 900.)] {
            view.update(cx, |shell, _| {
                shell.density = density;
            });
            cx.update(|window, cx| {
                window.resize(gpui_kit::size(px(width), px(height)));
                window.render_frame(cx);
                window.click("open-settings", cx);
                window.click("settings-nav-diagnostics", cx);
                let mut reachable = false;
                for _ in 0..24 {
                    let quit = window.find("quit-application");
                    let bounds = quit.bounds();
                    let viewport = window.find("settings-scroll").bounds();
                    // A visible accessibility node can still be clipped at
                    // the scroll boundary and refuse an ordinary click.
                    if quit.visible()
                        && bounds.origin.y >= viewport.origin.y
                        && bounds.origin.y + bounds.size.height
                            <= viewport.origin.y + viewport.size.height
                    {
                        reachable = true;
                        break;
                    }
                    window.scroll(
                        "settings-scroll",
                        gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-180.))),
                        cx,
                    );
                    window.render_frame(cx);
                }
                assert!(
                    reachable,
                    "Quit must fit inside the scrollport: {density:?} {width}x{height}"
                );
                assert!(window.try_find("settings-save").is_none());
                assert!(window.try_find("settings-feedback").is_none());
                window.click("close-settings", cx);
            });
            view.update(cx, |shell, _| {
                assert_eq!(shell.density, density);
            });
        }
    }
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell.state.read(cx).snapshot.phase,
            crate::service::SessionPhase::Idle,
            "revealing diagnostics must not start an audio owner"
        );
    });
    finish_service(runtime);
}
