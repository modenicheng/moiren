//! Independent Mapper cards use the same routing editor as the Inspector.
use super::*;

pub(super) fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    super::super::mapper_editor::card(shell, node, cx)
}
