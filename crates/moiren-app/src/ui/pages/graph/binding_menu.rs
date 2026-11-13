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
use crate::ui::widgets::anchored_popover;
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use crate::ui::widgets::overlay_scrollbar::scroll_region;
use gpui_kit::base::ScrollbarAxis;
use gpui_kit::{AnyElement, Bounds, Pixels, Window, px};
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
                    // The list re-renders from the replaced plugin library
                    // once the sandboxed scan lands.
                    BindingAction::ScanPlugins => shell
                        .state
                        .update(cx, |state, _| state.dispatch(AppRequest::ScanPlugins)),
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
    let Some(choices) = binding_choices::choices(
        &shell.state.read(cx).snapshot,
        menu.node,
        shell.graph_view.source_classes.get(&menu.node).copied(),
    ) else {
        return div().into_any_element();
    };
    // Hang the menu under the chip, flipping above it when the window
    // bottom is too close, and keep it inside the window sideways.
    let viewport = window.viewport_size();
    let width = MENU_WIDTH.max(f32::from(menu.anchor.size.width));
    let height = (choices.len() as f32 * ROW_H + 2. * MENU_PAD).min(MAX_HEIGHT);
    let placed = anchored_popover::under_anchor(menu.anchor, width, height, viewport);
    let rows = choices.into_iter().map(|choice| row(this.clone(), choice));
    let close = this.clone();
    occluding_page(
        "binding-menu-page",
        true,
        move |cx| {
            close.update(cx, |shell, cx| {
                shell.graph_view.binding_menu = None;
                cx.notify();
            })
        },
        claim_presses(
            // The menu box clips; the scrolling happens on an inner container
            // so the bar overlay can pin beside it as a sibling.
            div()
                .id("binding-menu")
                .absolute()
                .left(px(placed.left))
                .top(px(placed.top))
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
                            .gap(px(theme::SPACE_0_5))
                            .p(px(MENU_PAD))
                            .track_scroll(&shell.graph_view.binding_menu_scroll)
                            .restrict_scroll_to_axis()
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .max_h(px(height)),
                )
                .gpui_test_observe(),
        ),
    )
}
