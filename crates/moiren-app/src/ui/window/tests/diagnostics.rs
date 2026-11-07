//! Diagnostic actions must fit the scrollport, not just expose a UIA node.
use super::*;

#[gpui_kit::test]
fn stopped_diagnostics_quit_scrolls_fully_above_the_fixed_footer(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (view, cx, runtime) = test_shell(cx);
    for density in [
        super::super::Density::Comfortable,
        super::super::Density::Compact,
    ] {
        for (width, height) in [(1100., 700.), (1440., 900.)] {
            view.update(cx, |shell, _| {
                shell.density = density;
                shell.settings.global.density = density;
            });
            cx.update(|window, cx| {
                window.resize(gpui_kit::size(px(width), px(height)));
                window.render_frame(cx);
                window.click("open-settings", cx);
                window.click("settings-diagnostics", cx);
                let mut reachable = false;
                for _ in 0..24 {
                    let quit = window.find("quit-application");
                    let bounds = quit.bounds();
                    let viewport = window.find("settings-scroll").bounds();
                    // UIA can report a clipped action as visible and accept
                    // Invoke while GPUI's center click actually hits a footer.
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
                    "Quit must fit above the fixed footer: {density:?} {width}x{height}"
                );
                assert!(window.find("settings-save").visible());
                assert!(window.find("settings-feedback").visible());
                let quit = window.find("quit-application").bounds();
                let feedback = window.find("settings-feedback").bounds();
                assert!(quit.origin.y + quit.size.height <= feedback.origin.y);
                window.click("close-settings", cx);
            });
            view.update(cx, |shell, _| {
                assert_eq!(shell.settings.global.density, density);
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
