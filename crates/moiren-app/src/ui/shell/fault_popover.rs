//! The fault detail popover: hangs under a faulted object's "!" marker — a
//! graph card badge, a mixer strip badge, or the status bar's error summary —
//! and lists what is wrong and how to fix it. This replaces the old
//! full-width error strip: located faults surface on the object itself, and
//! this popover only carries the details on demand. Mounted at the shell
//! root outside the cached pages, on the shared anchored-popover placement
//! and the shared occluding-page dismissal.

use super::*;
use crate::service::GraphSubject;
use crate::ui::theme;
use crate::ui::widgets::anchored_popover;
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use gpui_kit::{AnyElement, Bounds, Pixels, Window, px};
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
    let Some(popover) = shell.chrome.fault_popover else {
        return;
    };
    let state = shell.state.read(cx);
    let alive = match popover.node {
        Some(node) => state.snapshot.graph.node_faulted(node),
        None => has_unlocated(state),
    };
    if !alive {
        shell.chrome.fault_popover = None;
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
/// Fault causes and snapshot errors are English catalog keys — plain prose
/// passes through unchanged; the typed dispatch failure translates through
/// [`crate::ui::presentation::failure_text`].
pub(crate) fn unlocated_summary(state: &UiState) -> Option<String> {
    state
        .snapshot
        .graph
        .faults
        .iter()
        .find(|fault| fault.subject == GraphSubject::Graph)
        .map(|fault| t!(fault.cause.as_str()).into_owned())
        .or_else(|| unlocated_diagnostic(state))
}

/// The non-located diagnostics minus the located graph faults: the stored
/// dispatch failure first, then the snapshot's error strings.
fn unlocated_diagnostic(state: &UiState) -> Option<String> {
    state
        .dispatch_error
        .as_ref()
        .map(crate::ui::presentation::failure_text)
        .or_else(|| {
            state
                .snapshot
                .error
                .as_ref()
                .or(state.snapshot.catalog_error.as_ref())
                .map(|error| t!(error.as_str()).into_owned())
        })
}

pub(crate) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let close = cx.entity();
    let Some(popover) = shell.chrome.fault_popover else {
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
            if let Some(error) = unlocated_diagnostic(state) {
                rows.push(fault_row(error, None));
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
    let width = WIDTH.max(f32::from(popover.anchor.size.width));
    let height = MAX_HEIGHT.min(rows.len() as f32 * 34. + 2. * PAD + 40.);
    let placed = anchored_popover::under_anchor(popover.anchor, width, height, viewport);
    occluding_page(
        "fault-popover-page",
        true,
        move |cx| {
            close.update(cx, |shell, cx| {
                shell.chrome.fault_popover = None;
                cx.notify();
            })
        },
        claim_presses(
            div()
                .id("fault-popover")
                .absolute()
                .left(px(placed.left))
                .top(px(placed.top))
                .w(px(width))
                .max_h(px(height))
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_1_5))
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
        ),
    )
}

/// One cause row with its optional hint, the error strip's old row moved
/// here verbatim minus the location line (the popover hangs on the object
/// that already identifies it).
fn fault_row(cause: String, hint: Option<String>) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_col()
        // 1 px: hairline-tight stack — the cause and its hint read as one
        // fault, not two lines of text.
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
