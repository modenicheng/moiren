//! The selected-stage workstation is shared with the Inspector.
use super::*;
use moiren_engine::rack::RackSettings;

pub(super) fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: &RackSettings,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    super::super::rack_editor::card(shell, node, settings, cx)
}
