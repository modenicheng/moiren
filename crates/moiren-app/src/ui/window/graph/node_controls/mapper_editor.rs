//! One routing editor, with independent card and Inspector draft/focus state.
use super::*;
use crate::service::{BuiltinState, GraphSnapshot, RequestId};
use moiren_engine::processor::ChannelMapperSettings;

mod model;
mod render;
pub(super) use model::{Editors, sync};

type DisclosureToggle = Box<dyn Fn(bool, &mut Window, &mut App)>;

pub(super) fn card(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    render::content(shell, node.id, model::Surface::Card, cx)
}

pub(in crate::ui::window::graph) fn inspector(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    render::content(shell, node.id, model::Surface::Inspector, cx).unwrap_or_else(|| {
        note("Invalid channel mapping; inspect the node settings.").into_any_element()
    })
}

impl super::NodeControls {
    pub(in crate::ui::window::graph) fn mapper_disclosure_open(&self, node: NodeId) -> bool {
        self.mapper.disclosure_open(node)
    }

    pub(in crate::ui::window::graph) fn mapper_disclosure_toggle(
        &self,
        node: NodeId,
        cx: &Context<StudioShell>,
    ) -> DisclosureToggle {
        let target = self.mapper.target(node);
        let this = cx.entity();
        Box::new(move |open, window, cx| {
            if let Some(target) = &target {
                this.update(cx, |shell, cx| {
                    model::toggle_disclosure(shell, target, open, window, cx);
                });
            }
        })
    }

    /// Call only after history/replacement request admission, before a delayed
    /// Blur can target reused IDs. New inputs wait for that request's snapshot.
    pub(in crate::ui::window) fn retire_mapper_until(&mut self, request: RequestId) {
        self.mapper.retire_until(request);
    }
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests;
