//! The binding chip's dropdown (design 18 B3): a popover under a source or
//! sink card's chip offering everything the inspector's picker does —
//! clear, tone, physical endpoints, exports, processes — so rebinding IO
//! never leaves the canvas. The entry list itself is the one shared
//! builder both surfaces render ([`super::binding_choices`], issue #119).
//! Mounted at the shell root outside the cached canvas subtree, like the
//! detail window; the chip itself is canvas paint, so the menu hangs from
//! the rect the hit test reported.

use super::binding_choices::{BindingAction, BindingChoice};
use super::*;
use crate::service::AppRequest;
use crate::ui::shell::controls;
use crate::ui::theme;
use crate::ui::widgets::actions::ButtonVariant;
use crate::ui::widgets::overlay_scrollbar::scroll_region;
use gpui_kit::base::ScrollbarAxis;
use gpui_kit::{AnyElement, Bounds, MouseButton, Pixels, Window, px};
use moiren_core::graph::NodeId;

/// An open endpoint menu: the card it binds and the chip rect it hangs
/// from, in window pixels.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct BindingMenu {
    pub node: NodeId,
    pub anchor: Bounds<Pixels>,
}

const MENU_WIDTH: f32 = 260.;
const MENU_PAD: f32 = 4.;
/// A small-size action button's row height, for placing and flipping.
const ROW_H: f32 = 30.;
const MAX_HEIGHT: f32 = 340.;

/// Drops a menu whose card no longer exists (deleted or undone away) so
/// the occluding layer never outlives its anchor.
pub(crate) fn sync(shell: &mut StudioShell, cx: &Context<StudioShell>) {
    if let Some(menu) = shell.graph_view.binding_menu
        && shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(menu.node)
            .is_none()
    {
        shell.graph_view.binding_menu = None;
    }
}

/// One shared-list entry as a menu row button, under the menu's `chip-`
/// element-id namespace.
fn row(this: Entity<StudioShell>, choice: BindingChoice) -> impl IntoElement {
    let id = SharedString::from(format!("chip-{}", choice.target.key()));
    controls::button(id, choice.label.clone())
        .variant(if choice.current {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
        .full_width()
        .justify_start()
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                match choice.action.clone() {
                    BindingAction::Edit(edit) => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                        shell.graph_view.binding_menu = None;
                    }
                    // The list re-renders from the refreshed catalog; the
                    // menu stays open for the pick.
                    BindingAction::Refresh => shell
                        .state
                        .update(cx, |state, _| state.dispatch(AppRequest::RefreshCatalog)),
                }
                cx.notify();
            })
        })
}

pub(crate) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let Some(menu) = shell.graph_view.binding_menu else {
        return div().into_any_element();
    };
    let Some(choices) = binding_choices::choices(&shell.state.read(cx).snapshot, menu.node) else {
        return div().into_any_element();
    };
    // Hang the menu under the chip, flipping above it when the window
    // bottom is too close, and keep it inside the window sideways.
    let viewport = window.viewport_size();
    let (view_width, view_height) = (f32::from(viewport.width), f32::from(viewport.height));
    let width = MENU_WIDTH.max(f32::from(menu.anchor.size.width));
    let height = (choices.len() as f32 * ROW_H + 2. * MENU_PAD).min(MAX_HEIGHT);
    let left = f32::from(menu.anchor.origin.x)
        .min(view_width - width - 8.)
        .max(8.);
    let below = f32::from(menu.anchor.origin.y + menu.anchor.size.height) + 4.;
    let top = if below + height > view_height - 8. {
        (f32::from(menu.anchor.origin.y) - 4. - height).max(8.)
    } else {
        below
    };
    let rows = choices.into_iter().map(|choice| row(this.clone(), choice));
    let close = this.clone();
    div()
        .id("binding-menu-page")
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .on_key_down(move |event, _window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                close.update(cx, |shell, cx| {
                    shell.graph_view.binding_menu = None;
                    cx.notify();
                });
                cx.stop_propagation();
            }
        })
        .on_any_mouse_down({
            let this = this.clone();
            move |event, _window, cx| {
                cx.stop_propagation();
                let inside = event.position.x >= px(left)
                    && event.position.x <= px(left + width)
                    && event.position.y >= px(top)
                    && event.position.y <= px(top + height);
                if !inside {
                    this.update(cx, |shell, cx| {
                        shell.graph_view.binding_menu = None;
                        cx.notify();
                    });
                }
            }
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            // The menu box clips; the scrolling happens on an inner container
            // so the bar overlay can pin beside it as a sibling.
            div()
                .id("binding-menu")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .max_h(px(height))
                .flex()
                .flex_col()
                .rounded(px(theme::RADIUS_MD))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .overflow_hidden()
                .child(
                    scroll_region(
                        "scrollbar-binding-menu",
                        &shell.graph_view.binding_menu_scroll,
                        ScrollbarAxis::Vertical,
                        div()
                            .id("binding-menu-scroll")
                            .max_h(px(height))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .p(px(MENU_PAD))
                            .track_scroll(&shell.graph_view.binding_menu_scroll)
                            .restrict_scroll_to_axis()
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .max_h(px(height)),
                )
                .gpui_test_observe(),
        )
        .into_any_element()
}
