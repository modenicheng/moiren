//! Utility card dispatch and shared native control primitives.

use super::*;
use crate::service::BuiltinState;

mod bus;
mod mapper;
mod meter;
mod rack;
pub(super) mod spectrum;

pub(super) fn content(
    shell: &StudioShell,
    card: &CanvasNode,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    // Paging must not clone the full routing matrix merely to paint one tile.
    if card.kind == NodeKind::ChannelMapper {
        return mapper::content(shell, node, cx);
    }
    let builtin = node.builtin.resolved(card.kind);
    Some(match card.kind {
        NodeKind::Rack => rack::content(shell, node, builtin.rack()?, cx)?,
        NodeKind::Meter => meter::content(shell, node, *builtin.meter()?, cx)?,
        NodeKind::SpectrumAnalyzer => {
            spectrum::content(shell, node.id, *builtin.spectrum()?, "card", cx)
        }
        NodeKind::Bus => bus::content(shell, node.id, cx)?,
        _ => return None,
    })
}

fn builtin_edit(node: NodeId, state: BuiltinState) -> GraphEdit {
    GraphEdit::SetBuiltin { node, state }
}

fn choice(
    id: String,
    label: impl Into<SharedString>,
    selected: bool,
    edit: Option<GraphEdit>,
    cx: &Context<StudioShell>,
) -> ActionButton {
    let label = label.into();
    let button = controls::button(id, t!(label.as_ref()).to_string())
        .density(Density::Compact)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        })
        .disabled(selected || edit.is_none());
    match edit {
        Some(edit) => button.on_click(controls::edit(cx.entity(), edit)),
        None => button,
    }
}
