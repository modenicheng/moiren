//! The fault detail popover: hangs under a faulted object's "!" marker — a
//! graph card badge, a mixer strip badge, or the status bar's error summary —
//! and lists what is wrong and how to fix it. This replaces the old
//! full-width error strip: located faults surface on the object itself, and
//! this popover only carries the details on demand. Mounted at the shell
//! root outside the cached pages, copying the binding menu's placement and
//! dismissal.

use super::*;
use crate::service::GraphSubject;
use crate::ui::theme;
use gpui_kit::{AnyElement, Bounds, MouseButton, Pixels, Window, px};
use moiren_core::graph::NodeId;

/// An open fault popover: the node whose faults are listed, or `None` for
/// the graph-wide group (graph-subject faults plus the non-located dispatch,
/// session and catalog diagnostics), and the rect it hangs from in window
/// pixels.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct FaultPopover {
    pub node: Option<NodeId>,
    pub anchor: Bounds<Pixels>,
}

const WIDTH: f32 = 320.;
const PAD: f32 = 10.;
const MAX_HEIGHT: f32 = 320.;

/// Drops a popover whose content group vanished (faults clear on a
/// successful edit, or the faulted node was deleted) so the occluding layer
/// never outlives its reason.
pub(crate) fn sync(shell: &mut StudioShell, cx: &Context<StudioShell>) {
    let Some(popover) = shell.fault_popover else {
        return;
    };
    let state = shell.state.read(cx);
    let alive = match popover.node {
        Some(node) => state.snapshot.graph.node_faulted(node),
        None => has_unlocated(state),
    };
    if !alive {
        shell.fault_popover = None;
    }
}

/// Whether the graph-wide popover still has anything to list.
pub(crate) fn has_unlocated(state: &UiState) -> bool {
    state
        .snapshot
        .graph
        .faults
        .iter()
        .any(|fault| fault.subject == GraphSubject::Graph)
        || state.dispatch_error.is_some()
        || state.snapshot.error.is_some()
        || state.snapshot.catalog_error.is_some()
}

/// The status bar's red summary: the first unlocated message, translated.
/// Causes are English catalog keys; plain prose passes through unchanged.
pub(crate) fn unlocated_summary(state: &UiState) -> Option<String> {
    state
        .snapshot
        .graph
        .faults
        .iter()
        .find(|fault| fault.subject == GraphSubject::Graph)
        .map(|fault| t!(fault.cause.as_str()).into_owned())
        .or_else(|| {
            state
                .dispatch_error
                .as_ref()
                .or(state.snapshot.error.as_ref())
                .or(state.snapshot.catalog_error.as_ref())
                .map(|error| t!(error.as_str()).into_owned())
        })
}

pub(crate) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let Some(popover) = shell.fault_popover else {
        return div().into_any_element();
    };
    let state = shell.state.read(cx);
    let log_path = crate::logging::log_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| t!("Application file logging is unavailable").into());
    // One row per fault: translated cause in danger red, hint underneath.
    // (Causes and hints are English catalog keys produced by the service;
    // translation happens here so a locale switch re-renders stored faults.)
    let rows: Vec<AnyElement> = match popover.node {
        Some(node) => state
            .snapshot
            .graph
            .faults_for_node(node)
            .into_iter()
            .map(|fault| {
                fault_row(
                    t!("Error: %{cause}", cause = t!(fault.cause.as_str())).into_owned(),
                    fault
                        .hint
                        .as_deref()
                        .map(|hint| t!("Hint: %{hint}", hint = t!(hint)).into_owned()),
                )
            })
            .collect(),
        None => {
            let mut rows: Vec<AnyElement> = state
                .snapshot
                .graph
                .faults
                .iter()
                .filter(|fault| fault.subject == GraphSubject::Graph)
                .map(|fault| {
                    fault_row(
                        t!("Error: %{cause}", cause = t!(fault.cause.as_str())).into_owned(),
                        fault
                            .hint
                            .as_deref()
                            .map(|hint| t!("Hint: %{hint}", hint = t!(hint)).into_owned()),
                    )
                })
                .collect();
            // The non-located diagnostics, first Some wins (the error
            // strip's old precedence).
            if let Some(error) = state
                .dispatch_error
                .as_ref()
                .or(state.snapshot.error.as_ref())
                .or(state.snapshot.catalog_error.as_ref())
            {
                rows.push(fault_row(t!(error.as_str()).into_owned(), None));
            }
            rows
        }
    };
    if rows.is_empty() {
        return div().into_any_element();
    }
    // Hang the popover under the anchor, flipping above it when the window
    // bottom is too close, and keep it inside the window sideways.
    let viewport = window.viewport_size();
    let (view_width, view_height) = (f32::from(viewport.width), f32::from(viewport.height));
    let width = WIDTH.max(f32::from(popover.anchor.size.width));
    let height = MAX_HEIGHT.min(rows.len() as f32 * 34. + 2. * PAD + 40.);
    let left = f32::from(popover.anchor.origin.x)
        .min(view_width - width - 8.)
        .max(8.);
    let below = f32::from(popover.anchor.origin.y + popover.anchor.size.height) + 4.;
    let top = if below + height > view_height - 8. {
        (f32::from(popover.anchor.origin.y) - 4. - height).max(8.)
    } else {
        below
    };
    let close = this.clone();
    div()
        .id("fault-popover-page")
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .on_key_down(move |event, _window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                close.update(cx, |shell, cx| {
                    shell.fault_popover = None;
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
                        shell.fault_popover = None;
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
                .id("fault-popover")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .max_h(px(height))
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(PAD))
                .rounded(px(theme::RADIUS_MD))
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .overflow_y_scroll()
                .child(
                    div()
                        .text_size(px(theme::TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::TEXT_MUTED)
                        .child(t!("Error details").to_string()),
                )
                .children(rows)
                .child(
                    div()
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::TEXT_FAINT)
                        .child(t!("Diagnostics: %{log_path}", log_path = log_path).to_string()),
                )
                .gpui_test_observe(),
        )
        .into_any_element()
}

/// One cause row with its optional hint, the error strip's old row moved
/// here verbatim minus the location line (the popover hangs on the object
/// that already identifies it).
fn fault_row(cause: String, hint: Option<String>) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .text_size(px(theme::TEXT_SM))
        .child(
            div()
                .text_color(theme::DANGER)
                .font_weight(FontWeight::SEMIBOLD)
                .child(cause),
        );
    if let Some(hint) = hint {
        row = row.child(div().text_color(theme::TEXT).child(hint));
    }
    row.into_any_element()
}
