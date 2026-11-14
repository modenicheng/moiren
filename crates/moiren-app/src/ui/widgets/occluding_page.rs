//! The shared full-screen dismiss layer behind an anchored popover
//! (design 21 §3.1): a transparent page stacked over the whole window,
//! above every workspace content and under its popover. Any press that
//! reaches the page — everything outside the popover subtree — dismisses
//! it; the popover claims its own subtree's presses via [`claim_presses`]
//! so an inside press never closes the popover that owns it. That
//! subtree-claim shape is the one outside-press semantics every popover
//! shares; no popover needs bounds math mirrored between its placement and
//! its dismissal. Mouse-up, move and scroll are swallowed too, so the
//! workspace underneath never reacts to a gesture aimed at a popover.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// Claims a popover subtree's presses: the press stops at the popover and
/// never reaches the [`occluding_page`] behind it, so inside clicks cannot
/// close the popover that owns them.
pub(crate) fn claim_presses<E: InteractiveElement>(element: E) -> E {
    element.on_any_mouse_down(|_, _, cx| cx.stop_propagation())
}

/// The full-screen dismiss layer for an open popover, mounted at the shell
/// root. `id` is the page's element id; `dismiss` closes the popover state
/// it guards; `escape_closes` wires the same close to the Escape key for
/// popovers that own their keyboard dismissal (a popover whose host page
/// peels it on Escape itself — the settings value menu — passes `false`);
/// `popover` is the claimed popover subtree ([`claim_presses`]).
pub(crate) fn occluding_page(
    id: &'static str,
    escape_closes: bool,
    dismiss: impl Fn(&mut App) + Clone + 'static,
    popover: impl IntoElement,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .when(escape_closes, |page| {
            let dismiss = dismiss.clone();
            page.on_key_down(move |event, _window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    dismiss(cx);
                    cx.stop_propagation();
                }
            })
        })
        // Reached only by presses outside the popover subtree — the popover
        // claims its own — so every press here dismisses the layer.
        .on_any_mouse_down(move |_, _, cx| dismiss(cx))
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(popover)
        .into_any_element()
}
