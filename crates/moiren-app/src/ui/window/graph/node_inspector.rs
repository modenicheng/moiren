use rust_i18n::t;

use super::*;
use crate::service::{GraphEdit, InputSelection};
use crate::ui::presentation::{db_text, dbfs_text, step_db};
use crate::ui::widgets::fader::hslider;
use crate::ui::widgets::meters::compact_horizontal_meter;
use crate::ui::window::mixer::{EditableValueFamily, gain_db_text, start_value_edit};
use controls::*;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
    rack::{RackProcessor, RackSettings, RackSlot},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};

mod binding;
mod builtin;
mod diagnostics;
mod identity;
mod parameters;
mod routing;
mod sections;
mod strip;
#[cfg(test)]
mod tests;

use sections::{Disclosure, Section, body, section};

/// Secondary editor: identity, full parameters, routing and actual observation.
pub(in crate::ui::window) fn inspector(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let selected = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|node| shell.selected_node == Some(node.id()));
    let view = panel("graph-inspector")
        .gap(px(theme::SPACE_2))
        .child(heading(t!("Inspector")));
    let Some(node) = selected else {
        return view
            .child(
                div()
                    .id("inspector-empty")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::SPACE_4))
                    .child(
                        div()
                            .id("inspector-empty-art")
                            .flex()
                            .flex_none()
                            .w(px(64.))
                            .h(px(64.))
                            .items_center()
                            .justify_center()
                            .rounded(px(theme::RADIUS_LG))
                            .bg(theme::SURFACE_3)
                            .child(
                                component::Icon::default()
                                    .path("icons/zoom-scan.svg")
                                    .size(px(28.))
                                    .text_color(theme::TEXT_FAINT),
                            )
                            .gpui_test_observe(),
                    )
                    .child(label(t!("Select a node to inspect its properties.")))
                    .gpui_test_observe(),
            )
            .gpui_test_observe()
            .into_any_element();
    };
    let id = node.id();
    let Some(entry) = snapshot.graph.node(id) else {
        return view
            .child(label(t!("Unavailable for this node")))
            .into_any_element();
    };
    view.child(identity::identity(shell, node, entry, cx))
        .when(
            matches!(node.kind(), NodeKind::Source | NodeKind::Sink),
            |view| view.child(binding::binding(shell, node, entry, cx)),
        )
        .when(has_advanced_parameters(node.kind()), |view| {
            view.child(advanced_node_controls(shell, node, entry, cx))
        })
        .child(routing::routing(shell, node, cx))
        .child(strip::strip(shell, node, entry, cx))
        .child(diagnostics::node_observation(shell, node, cx))
        .child(
            section("inspector-actions", t!("Node actions")).child(
                button("remove-node", t!("Delete node"))
                    .variant(ButtonVariant::Danger)
                    .full_width()
                    .on_click(edit(cx.entity(), GraphEdit::RemoveNode(id))),
            ),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn has_advanced_parameters(kind: NodeKind) -> bool {
    !matches!(kind, NodeKind::Source | NodeKind::Sink | NodeKind::Bus)
}

/// Reuses shell-owned editors without depending on any node-card implementation.
/// Render at most once per node on a surface: input entities are intentionally shared.
pub(super) fn advanced_node_controls(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    parameters::parameters(shell, node, entry, cx)
}
