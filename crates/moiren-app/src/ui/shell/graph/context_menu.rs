//! The canvas context menu (design alignment round 2026-11-08): over a
//! node it offers the tint palette plus the identity actions — delete,
//! inspector, rename — and over the background it offers arrange. Same
//! dismissal chrome as the settings ValueMenu and the session popover: an
//! occluding shell-root page closes on any press outside the menu (the
//! menu claims its own subtree) and Escape closes it — on the graph page
//! via the canvas cancel path that `keep_graph_focus` keeps alive.

use super::super::controls;
use super::*;
use crate::service::{AppRequest, GraphEdit, NODE_TINT_COUNT};
use crate::ui::theme;
use gpui_kit::{AnyElement, MouseButton, Window, px};

const MENU_WIDTH: f32 = 176.;

pub(in crate::ui::shell) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let Some(menu) = shell.context_menu else {
        return div().into_any_element();
    };
    let this = cx.entity();
    let viewport = window.viewport_size();
    let width = MENU_WIDTH.min(f32::from(viewport.width) - 16.);
    let left = px(menu.at().0)
        .min(px(f32::from(viewport.width) - width - 8.))
        .max(px(8.));
    let top = px(menu.at().1)
        .min(px(f32::from(viewport.height) - 8.))
        .max(px(8.));

    let close = this.clone();
    let mut body = match menu {
        ContextMenu::Node { node, .. } => {
            // The palette sits in the menu itself: one right click reaches
            // the swatches, no second click to unfold them first.
            let mut rows = color_row(this.clone(), node, shell, cx);
            rows.push(
                div()
                    .h(px(1.))
                    .w_full()
                    .bg(theme::BORDER)
                    .into_any_element(),
            );
            rows.extend(action_rows(this.clone(), node));
            rows
        }
        ContextMenu::Background { .. } => arrange_row(this.clone()),
    };
    let mut list = div()
        .id("context-menu")
        .absolute()
        .left(left)
        .top(top)
        .w(px(width))
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(6.))
        .rounded(px(theme::RADIUS_MD))
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .bg(theme::SURFACE)
        .occlude()
        .gpui_test_observe()
        // Claim the press so the occluding page behind never sees it —
        // inside clicks must not close the menu that owns them.
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation());
    list = list.children(body.drain(..));

    div()
        .id("context-menu-page")
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .on_key_down({
            let close = close.clone();
            move |event, _window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    close.update(cx, |shell, cx| {
                        shell.context_menu = None;
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            }
        })
        // Reached only by presses outside the menu — the menu claims its
        // own subtree — so every press here closes it.
        .on_any_mouse_down(move |_, _, cx| {
            close.update(cx, |shell, cx| {
                shell.context_menu = None;
                cx.notify();
            });
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(list)
        .into_any_element()
}

impl ContextMenu {
    /// The window-space anchor point.
    pub(in crate::ui::shell) fn at(&self) -> (f32, f32) {
        match self {
            Self::Node { at, .. } | Self::Background { at } => *at,
        }
    }
}

/// The identity actions over a node: delete, inspector, rename — the same
/// effects the detail window and the inspector offered, now reachable in
/// place. The tint palette rides above them, straight in the menu.
fn action_rows(this: Entity<StudioShell>, node: moiren_core::graph::NodeId) -> Vec<AnyElement> {
    use controls::*;
    vec![
        button("context-menu-delete", t!("Delete node"))
            .full_width()
            .justify_start()
            .variant(ButtonVariant::Ghost)
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.state.update(cx, |state, _| {
                            state.dispatch(AppRequest::EditGraph(GraphEdit::RemoveNode(node)))
                        });
                        shell.context_menu = None;
                        cx.notify();
                    });
                }
            }),
        button("context-menu-inspector", t!("Inspector"))
            .full_width()
            .justify_start()
            .variant(ButtonVariant::Ghost)
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.select_single_node(node);
                        shell.open_inspector();
                        shell.context_menu = None;
                        cx.notify();
                    });
                }
            }),
        button("context-menu-rename", t!("rename"))
            .full_width()
            .justify_start()
            .variant(ButtonVariant::Ghost)
            .on_click({
                move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        shell.select_single_node(node);
                        shell.open_inspector();
                        shell.begin_node_rename(node, window, cx);
                        shell.context_menu = None;
                        cx.notify();
                    });
                }
            }),
    ]
    .into_iter()
    .map(|row| row.into_any_element())
    .collect()
}

/// The tint palette plus the automatic (category) swatch, moved over from
/// the retired detail window: picking a swatch applies it and closes the
/// menu.
fn color_row(
    this: Entity<StudioShell>,
    node: moiren_core::graph::NodeId,
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> Vec<AnyElement> {
    let current: Option<u8> = shell
        .state
        .read(cx)
        .snapshot
        .graph
        .node(node)
        .and_then(|entry| entry.tint);
    let mut row = div()
        .id("context-menu-colors")
        .flex()
        .items_center()
        .gap(px(4.));
    row = row.child(
        div()
            .id("node-tint-auto")
            .size(px(16.))
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(if current.is_none() {
                theme::TEXT
            } else {
                theme::BORDER_STRONG
            })
            .gpui_test_observe()
            .when(current.is_none(), |v| {
                v.bg(theme::SURFACE_4).child(
                    div()
                        .m(px(3.))
                        .size(px(8.))
                        .rounded(px(2.))
                        .bg(theme::PRIMARY.opacity(0.5)),
                )
            })
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.state.update(cx, |state, _| {
                            state.dispatch(AppRequest::EditGraph(GraphEdit::SetNodeTint {
                                node,
                                tint: None,
                            }))
                        });
                        shell.context_menu = None;
                        cx.notify();
                    });
                }
            }),
    );
    for index in 0..NODE_TINT_COUNT {
        let tint = theme::node_tint(index).expect("palette covers the count");
        let selected = current == Some(index);
        row = row.child(
            div()
                .id(SharedString::from(format!("node-tint-{index}")))
                .size(px(16.))
                .rounded(px(theme::RADIUS_SM))
                .bg(tint)
                .when(selected, |v| v.border_1().border_color(theme::TEXT))
                .gpui_test_observe()
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(GraphEdit::SetNodeTint {
                                    node,
                                    tint: Some(index),
                                }))
                            });
                            shell.context_menu = None;
                            cx.notify();
                        });
                    }
                }),
        );
    }
    vec![row.into_any_element()]
}

/// The background action: arrange the graph into DAG layers and fit the
/// camera (item 3's trigger).
fn arrange_row(this: Entity<StudioShell>) -> Vec<AnyElement> {
    use controls::*;
    vec![
        button("context-menu-arrange", t!("Arrange"))
            .full_width()
            .justify_start()
            .variant(ButtonVariant::Ghost)
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.arrange_graph(cx);
                    shell.context_menu = None;
                    cx.notify();
                });
            })
            .into_any_element(),
    ]
}
