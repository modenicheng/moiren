//! The binding chip's dropdown (design 18 B3): a popover under a source or
//! sink card's chip offering everything the inspector's picker does —
//! clear, tone, physical endpoints, exports, processes — so rebinding IO
//! never leaves the canvas. Mounted at the shell root outside the cached
//! canvas subtree, like the detail window; the chip itself is canvas
//! paint, so the menu hangs from the rect the hit test reported.

use super::super::controls;
use super::*;
use crate::service::{AppRequest, ExportDirection, GraphEdit, InputSelection};
use crate::ui::theme;
use crate::ui::widgets::actions::ButtonVariant;
use gpui_kit::{AnyElement, Bounds, MouseButton, Pixels, Window, px};
use moiren_core::graph::{NodeId, NodeKind};

/// An open endpoint menu: the card it binds and the chip rect it hangs
/// from, in window pixels.
#[derive(Clone, Copy, PartialEq)]
pub(in crate::ui::window) struct BindingMenu {
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
pub(in crate::ui::window) fn sync(shell: &mut StudioShell, cx: &Context<StudioShell>) {
    if let Some(menu) = shell.binding_menu
        && shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(menu.node)
            .is_none()
    {
        shell.binding_menu = None;
    }
}

enum Action {
    Edit(GraphEdit),
    Refresh,
}

struct Item {
    id: SharedString,
    label: String,
    action: Action,
    current: bool,
}

/// The same choices the inspector lists for the kind, in the same order.
fn items(snapshot: &crate::service::AppSnapshot, id: NodeId) -> Option<Vec<Item>> {
    let logical = snapshot.graph.graph.get_node(id).ok()?;
    let entry = snapshot.graph.node(id)?;
    let bound_export = snapshot
        .graph
        .exports
        .iter()
        .find(|export| export.enabled && export.binding == Some(id));
    let mut items = Vec::new();
    match logical.kind() {
        NodeKind::Source => {
            if entry.source.is_some() || bound_export.is_some() {
                items.push(Item {
                    id: "chip-unbind".into(),
                    label: t!("Clear binding").to_string(),
                    action: Action::Edit(GraphEdit::ClearBinding(id)),
                    current: false,
                });
            }
            items.push(Item {
                id: "chip-bind-tone".into(),
                label: t!("Tone generator · 440 Hz").to_string(),
                action: Action::Edit(GraphEdit::BindSource {
                    node: id,
                    source: InputSelection::Tone { frequency_hz: 440. },
                }),
                current: matches!(entry.source, Some(InputSelection::Tone { .. })),
            });
            for row in crate::ui::state::input_rows(
                &snapshot.catalog,
                moiren_engine::audio_config::AudioBackend::Wasapi,
            ) {
                items.push(Item {
                    id: format!("chip-bind-input-{}", row.endpoint_id).into(),
                    label: row.name.clone(),
                    current: matches!(
                        &entry.source,
                        Some(InputSelection::Physical { endpoint_id }) if *endpoint_id == row.endpoint_id
                    ),
                    action: Action::Edit(GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Physical {
                            endpoint_id: row.endpoint_id,
                        },
                    }),
                });
            }
            for export in
                snapshot.graph.exports.iter().filter(|export| {
                    export.enabled && export.direction == ExportDirection::Playback
                })
            {
                items.push(Item {
                    id: format!("chip-bind-playback-export-{}", export.endpoint_id).into(),
                    label: export.name.clone(),
                    action: Action::Edit(GraphEdit::BindExport {
                        endpoint_id: export.endpoint_id,
                        node: Some(id),
                    }),
                    current: export.binding == Some(id),
                });
            }
            for row in &snapshot.catalog.processes {
                items.push(Item {
                    id: format!("chip-bind-process-{}", row.pid).into(),
                    label: row.name.clone(),
                    current: matches!(
                        &entry.source,
                        Some(InputSelection::Process { pid, .. }) if *pid == row.pid
                    ),
                    action: Action::Edit(GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Process {
                            pid: row.pid,
                            creation_time_100ns: row.creation_time_100ns,
                        },
                    }),
                });
            }
            items.push(Item {
                id: "chip-refresh".into(),
                label: t!("Refresh sources").to_string(),
                action: Action::Refresh,
                current: false,
            });
        }
        NodeKind::Sink => {
            if entry.output_endpoint_id.is_some() || bound_export.is_some() {
                items.push(Item {
                    id: "chip-unbind".into(),
                    label: t!("Clear binding").to_string(),
                    action: Action::Edit(GraphEdit::ClearBinding(id)),
                    current: false,
                });
            }
            for row in crate::ui::state::output_rows(
                &snapshot.catalog,
                moiren_engine::audio_config::AudioBackend::Wasapi,
            ) {
                items.push(Item {
                    id: format!("chip-bind-output-{}", row.endpoint_id).into(),
                    label: row.name.clone(),
                    current: entry.output_endpoint_id.as_deref() == Some(row.endpoint_id.as_str()),
                    action: Action::Edit(GraphEdit::BindOutput {
                        node: id,
                        endpoint_id: row.endpoint_id,
                    }),
                });
            }
            for export in
                snapshot.graph.exports.iter().filter(|export| {
                    export.enabled && export.direction == ExportDirection::Microphone
                })
            {
                items.push(Item {
                    id: format!("chip-bind-microphone-export-{}", export.endpoint_id).into(),
                    label: export.name.clone(),
                    action: Action::Edit(GraphEdit::BindExport {
                        endpoint_id: export.endpoint_id,
                        node: Some(id),
                    }),
                    current: export.binding == Some(id),
                });
            }
        }
        _ => return None,
    }
    Some(items)
}

pub(in crate::ui::window) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let Some(menu) = shell.binding_menu else {
        return div().into_any_element();
    };
    let Some(items) = items(&shell.state.read(cx).snapshot, menu.node) else {
        return div().into_any_element();
    };
    // Hang the menu under the chip, flipping above it when the window
    // bottom is too close, and keep it inside the window sideways.
    let viewport = window.viewport_size();
    let (view_width, view_height) = (f32::from(viewport.width), f32::from(viewport.height));
    let width = MENU_WIDTH.max(f32::from(menu.anchor.size.width));
    let height = (items.len() as f32 * ROW_H + 2. * MENU_PAD).min(MAX_HEIGHT);
    let left = f32::from(menu.anchor.origin.x)
        .min(view_width - width - 8.)
        .max(8.);
    let below = f32::from(menu.anchor.origin.y + menu.anchor.size.height) + 4.;
    let top = if below + height > view_height - 8. {
        (f32::from(menu.anchor.origin.y) - 4. - height).max(8.)
    } else {
        below
    };
    let rows = items.into_iter().map(|item| {
        let this = this.clone();
        controls::button(item.id, item.label)
            .variant(if item.current {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Ghost
            })
            .full_width()
            .justify_start()
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    match &item.action {
                        Action::Edit(edit) => {
                            let edit = edit.clone();
                            shell
                                .state
                                .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                            shell.binding_menu = None;
                        }
                        // The list re-renders from the refreshed catalog;
                        // the menu stays open for the pick.
                        Action::Refresh => shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::RefreshCatalog)),
                    }
                    cx.notify();
                })
            })
    });
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
                    shell.binding_menu = None;
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
                        shell.binding_menu = None;
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
                .id("binding-menu")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .max_h(px(height))
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(px(MENU_PAD))
                .rounded(px(theme::RADIUS_MD))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .overflow_y_scroll()
                .children(rows)
                .gpui_test_observe(),
        )
        .into_any_element()
}
