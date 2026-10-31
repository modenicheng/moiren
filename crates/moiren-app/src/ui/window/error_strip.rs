//! The cached error-strip panel (design 17 Tier C). Its contents derive
//! entirely from the published snapshot — faults, dispatch and catalog
//! errors — so publish identity is the whole key: the strip rebuilds only
//! when the service actually reported something new, instead of cloning the
//! fault list and rescanning all node ports on every shell notification.

use std::sync::Arc;

use super::panel::{CachedPanel, PanelHost, PublishRef, cached_splice};
use super::*;

pub(in crate::ui::window) struct ErrorStripPanel;

/// Whether the strip has anything to show; the shell splices the panel only
/// then, so an idle session renders no strip element at all.
pub(in crate::ui::window) fn has_faults(shell: &StudioShell, cx: &App) -> bool {
    let state = shell.state.read(cx);
    !state.snapshot.graph.faults.is_empty()
        || state.dispatch_error.is_some()
        || state.snapshot.error.is_some()
        || state.snapshot.catalog_error.is_some()
}

#[derive(PartialEq)]
pub(in crate::ui::window) struct ErrorStripKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
}

impl CachedPanel for ErrorStripPanel {
    type Owner = StudioShell;
    type Key = ErrorStripKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        Self::Key {
            snapshot: PublishRef::new(Arc::clone(&shell.state.read(cx).snapshot)),
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        build(shell, cx).into_any_element()
    }
}

pub(in crate::ui::window) type ErrorStripHost = PanelHost<ErrorStripPanel>;

/// The panel's cached splice; mounted only while [`has_faults`] holds.
pub(in crate::ui::window) fn for_shell(shell: &StudioShell, _window: &Window) -> AnyElement {
    cached_splice(shell.error_strip.clone(), |entity| {
        entity
            .cached(StyleRefinement::default().w_full().flex_none())
            .into_any_element()
    })
}

/// The strip's tree, moved verbatim out of the shell root render.
fn build(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let state = shell.state.read(cx);
    let faults = state.snapshot.graph.faults.clone();
    let other = state
        .dispatch_error
        .as_ref()
        .or(state.snapshot.error.as_ref())
        .or(state.snapshot.catalog_error.as_ref())
        .cloned();
    let log_path = crate::logging::log_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| t!("Application file logging is unavailable").into());
    let this = cx.entity();
    let graph = state.snapshot.graph.clone();
    let mut list = div()
        .id("error-strip")
        .flex_none()
        .flex()
        .flex_col()
        .px(px(theme::SPACE_4))
        .py(px(theme::SPACE_2))
        .bg(theme::soft(theme::DANGER))
        .text_size(px(theme::TEXT_SM));
    for (index, fault) in faults.iter().enumerate() {
        let subject = fault.subject;
        let location = match subject {
            crate::service::GraphSubject::Node(node) => {
                graph.node(node).map(|node| node.label.clone())
            }
            crate::service::GraphSubject::Route(source, destination) => {
                let port_label = |port: &moiren_core::graph::PortId| {
                    graph
                        .graph
                        .nodes()
                        .iter()
                        .find(|node| {
                            node.inputs()
                                .iter()
                                .chain(node.outputs())
                                .any(|candidate| candidate.id() == *port)
                        })
                        .map(|node| node.id())
                        .and_then(|owner| graph.node(owner).map(|node| node.label.clone()))
                };
                let source = port_label(&source);
                let destination = port_label(&destination);
                match (source, destination) {
                    (Some(source), Some(destination)) => Some(format!("{source} → {destination}")),
                    _ => None,
                }
            }
            crate::service::GraphSubject::Graph => None,
        };
        let mut row = div()
            .id(SharedString::from(format!("fault-{index}")))
            .flex()
            .flex_col()
            .gap(px(1.))
            .when(subject != crate::service::GraphSubject::Graph, |row| {
                row.cursor_pointer().on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.inspector_open = true;
                            match subject {
                                crate::service::GraphSubject::Node(node) => {
                                    shell.selected_route = None;
                                    shell.selected_node = Some(node);
                                }
                                crate::service::GraphSubject::Route(source, destination) => {
                                    shell.selected_node = None;
                                    shell.selected_route = Some((source, destination));
                                }
                                crate::service::GraphSubject::Graph => {}
                            }
                            cx.notify();
                        });
                    }
                })
            });
        row = row
            .child(
                div()
                    .text_color(theme::DANGER)
                    .font_weight(FontWeight::SEMIBOLD)
                    // Causes and hints are English catalog keys
                    // produced by the service; translation happens
                    // here so a locale switch re-renders stored
                    // faults, and unknown prose passes through.
                    .child(t!("Error: %{cause}", cause = t!(fault.cause.as_str()))),
            )
            .when_some(location, |row, location| {
                row.child(
                    div()
                        .text_color(theme::TEXT)
                        .child(t!("Channel: %{location}", location = location.as_str())),
                )
            })
            .when_some(fault.hint.as_deref(), |row, hint: &str| {
                row.child(
                    div()
                        .text_color(theme::TEXT)
                        .child(t!("Hint: %{hint}", hint = t!(hint))),
                )
            });
        list = list.child(row.gpui_test_observe());
    }
    if let Some(error) = other {
        // Dispatch/persistence diagnostics may be plain prose; a
        // catalog miss renders the source text unchanged.
        let message = t!(error.as_str()).into_owned();
        list = list.child(
            div()
                .id("error-strip-message")
                .text_color(theme::DANGER)
                .child(message)
                .gpui_test_observe(),
        );
    }
    list.child(
        div()
            .pt(px(2.))
            .text_color(theme::TEXT)
            .child(t!("Diagnostics: %{log_path}", log_path = log_path)),
    )
    .gpui_test_observe()
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;
