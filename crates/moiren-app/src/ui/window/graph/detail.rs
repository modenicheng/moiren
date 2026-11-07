//! The per-node detail window (design 18 B4): an in-window overlay over
//! the graph page carrying the node's headline visualization — the EQ's
//! draggable control point, the dynamics family's transfer curve ([`
//! crate::ui::window::graph::headline`]) — plus the identity controls
//! every node shares: tint palette and a pointer to the inspector for the
//! long tail of parameters. Mounted at the shell root next to the settings
//! overlay, outside the cached canvas subtree, so editing never
//! re-projects the graph.

use super::super::controls;
use super::*;
use crate::service::{AppRequest, GraphEdit, NODE_TINT_COUNT};
use crate::ui::theme;
use gpui_kit::{AnyElement, MouseButton, Window, px};
use moiren_core::graph::NodeId;

pub(in crate::ui::window) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let close = this.clone();
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let Some(id) = shell.detail_node else {
        return div().into_any_element();
    };
    let Some(logical) = snapshot.graph.graph.get_node(id).ok() else {
        return div().into_any_element();
    };
    let Some(entry) = snapshot.graph.node(id) else {
        return div().into_any_element();
    };
    let kind = logical.kind();
    let name = crate::ui::presentation::display_label(entry);
    let channels = if logical.channels() == 2 {
        "L/R".to_string()
    } else {
        format!("{} ch", logical.channels())
    };
    let detail = pages::graph_canvas::detail_string(kind, entry, &channels);

    let viewport = window.viewport_size();
    let dialog_width = 480f32.min(f32::from(viewport.width) * 0.95);
    let dialog_height = (f32::from(viewport.height) - 40.).clamp(260., 640.);
    let left = (f32::from(viewport.width) - dialog_width) * 0.5;
    let top = (f32::from(viewport.height) - dialog_height) * 0.5;

    let body: AnyElement = match kind {
        moiren_core::graph::NodeKind::Eq => {
            headline::eq_section(id, entry, cx, headline::HeadlineSizing::Fill)
        }
        moiren_core::graph::NodeKind::Compressor
        | moiren_core::graph::NodeKind::Gate
        | moiren_core::graph::NodeKind::Limiter => {
            headline::dynamics_section(kind, entry, headline::HeadlineSizing::Fill)
        }
        _ => div()
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .text_size(px(theme::TEXT_MD))
            .text_color(theme::TEXT_MUTED)
            .child(detail)
            .into_any_element(),
    };

    let tint_row = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(theme::SPACE_3))
        .pb(px(theme::SPACE_2))
        .child(
            div()
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_FAINT)
                .child(t!("Color")),
        )
        .child(swatches(this.clone(), id, entry.tint));

    div()
        .id("node-detail-page")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_key_down(move |event, _window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                close.update(cx, |shell, cx| {
                    shell.detail_node = None;
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
                    && event.position.x <= px(left + dialog_width)
                    && event.position.y >= px(top)
                    && event.position.y <= px(top + dialog_height);
                if !inside {
                    this.update(cx, |shell, cx| {
                        shell.detail_node = None;
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
            div()
                .id("node-detail-backdrop")
                .absolute()
                .inset_0()
                .bg(theme::OVERLAY_SCRIM)
                .gpui_test_observe(),
        )
        .child(
            div()
                .id("node-detail-dialog")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(dialog_width))
                .h(px(dialog_height))
                .flex()
                .flex_col()
                .rounded(px(theme::RADIUS_LG))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(header(this, &name))
                .child(tint_row)
                .child(
                    div()
                        .id("node-detail-body")
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .flex_col()
                        .child(body),
                )
                .gpui_test_observe(),
        )
        .into_any_element()
}

fn header(this: Entity<StudioShell>, name: &str) -> impl IntoElement {
    use controls::*;
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_2))
        .border_b_1()
        .border_color(theme::HAIRLINE)
        .child(
            div()
                .text_size(px(theme::TEXT_LG))
                .text_color(theme::TEXT)
                .child(name.to_owned()),
        )
        .child(
            button("node-detail-close", t!("Close")).on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.detail_node = None;
                    cx.notify();
                })
            }),
        )
}

/// The tint palette plus the automatic (category) swatch.
fn swatches(this: Entity<StudioShell>, node: NodeId, current: Option<u8>) -> impl IntoElement {
    let mut row = div().flex().items_center().gap(px(4.));
    let auto = div()
        .id("node-tint-auto")
        .size(px(16.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(if current.is_none() {
            theme::TEXT
        } else {
            theme::BORDER_STRONG
        })
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
                    cx.notify();
                })
            }
        });
    row = row.child(auto);
    for index in 0..NODE_TINT_COUNT {
        let tint = theme::node_tint(index).expect("palette covers the count");
        let selected = current == Some(index);
        row = row.child(
            div()
                .id(format!("node-tint-{index}"))
                .size(px(16.))
                .rounded(px(theme::RADIUS_SM))
                .bg(tint)
                .when(selected, |v| v.border_1().border_color(theme::TEXT))
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
                            cx.notify();
                        })
                    }
                }),
        );
    }
    row
}
