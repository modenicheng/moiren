//! Secondary channel-strip controls share existing input and slider state.
use super::*;

pub(super) fn strip(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = node.id();
    let content = body()
        .child(detail_editable(
            &t!("Fader"),
            SharedString::from(format!("inspector-fader-{id:?}")),
            db_text(entry.mixer.gain),
            shell.mixer.editing_value == Some(id),
            &shell.mixer.value_inputs[&id],
            start_value_edit(cx.entity(), EditableValueFamily::Fader, id, move |graph| {
                graph
                    .node(id)
                    .map(|node| gain_db_text(node.mixer.gain))
                    .unwrap_or_default()
            }),
        ))
        .child(hslider(
            &shell.mixer.faders[&id].state,
            &shell.mixer.faders[&id].behavior,
            cx,
        ))
        .when(node.channels() == 2, |view| {
            view.child(crate::ui::window::mixer::strip_controls::balance(
                shell,
                entry,
                node.channels(),
                cx,
            ))
        })
        .child(crate::ui::window::mixer::node_controls(
            entry,
            node.channels(),
            cx,
        ));
    let mut summary = format!(
        "{} / {}",
        db_text(entry.mixer.gain),
        entry.mixer.mode_label(node.channels())
    );
    for (enabled, label) in [
        (entry.mixer.muted, "Mute"),
        (entry.mixer.solo, "Solo"),
        (entry.mixer.dimmed, "Dim"),
    ] {
        if enabled {
            summary.push_str(&format!(" / {}", t!(label)));
        }
    }
    Disclosure::new(Section::Strip, id, content)
        .summary(summary)
        .into_any_element()
}
