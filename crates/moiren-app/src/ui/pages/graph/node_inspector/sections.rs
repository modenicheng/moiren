//! The inspector's per-`NodeKind` sections: the source/sink binding pickers
//! over the shared [`binding_choices`] list (issue #119) and the
//! gain/pan/compressor parameter sections. Each function appends its section
//! to the inspector column and hands it back, so [`super::inspector`] stays
//! a linear assembly dispatching by kind.

use rust_i18n::t;

use super::*;
use crate::service::{GraphEdit, InputSelection};
use crate::ui::editors::{EditableValueFamily, gain_db_text, start_value_edit};
use crate::ui::pages::graph::binding_choices;
use crate::ui::presentation::{db_text, step_db};
use crate::ui::widgets::fader::hslider;

pub(super) fn source_section(
    mut view: Stateful<Div>,
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    stored: Option<super::node_catalog::SourceClass>,
    id: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let bound_export = snapshot
        .graph
        .exports
        .iter()
        .find(|export| export.enabled && export.binding == Some(id));
    view = view.child(heading(t!("Source"))).child(label(
        bound_export
            .map(|export| export.name.clone())
            .or_else(|| {
                entry
                    .source
                    .as_ref()
                    .map(|source| crate::ui::presentation::source_name(snapshot, source))
            })
            .unwrap_or_else(|| t!("Not bound · choose a resource").into()),
    ));
    // The one shared choice list (issue #119): the same ordered entries
    // the chip menu offers, rendered as this panel's pick buttons. The
    // class filter (2026-11-13 split) narrows it to this node's class.
    let class = binding_choices::effective_source_class(snapshot, entry, stored);
    let choices = binding_choices::source_choices(snapshot, entry, class);
    let has_resources = choices.iter().any(|choice| {
        matches!(
            choice.target,
            binding_choices::BindingTarget::Input { .. }
                | binding_choices::BindingTarget::PlaybackExport { .. }
                | binding_choices::BindingTarget::Process { .. }
                | binding_choices::BindingTarget::Tone { .. }
        )
    });
    if !has_resources {
        // Literal `t!` keys per class keep the i18n ledger's two-way check
        // working (a runtime-key lookup would need a dynamic exception).
        let empty_copy = match class {
            super::node_catalog::SourceClass::Physical => {
                t!("No physical input is available. Connect or enable a device, then refresh.")
                    .to_string()
            }
            super::node_catalog::SourceClass::Virtual => {
                t!("No virtual device exists yet. Create one on the Devices page, then refresh.")
                    .to_string()
            }
            super::node_catalog::SourceClass::Application => {
                t!("No application is producing audio right now.").to_string()
            }
            // The tone generator is itself the class's resource, so this
            // arm is unreachable; it exists to keep the match exhaustive.
            super::node_catalog::SourceClass::Tone => String::new(),
        };
        if !empty_copy.is_empty() {
            view = view.child(label(empty_copy));
        }
    }
    for choice in choices {
        let is_tone = matches!(choice.target, binding_choices::BindingTarget::Tone { .. });
        view = view.child(binding_choice_button(cx.entity(), choice));
        // The bound tone's frequency editor rides directly under the
        // tone choice that selected it.
        if is_tone && let Some(InputSelection::Tone { frequency_hz }) = entry.source {
            view = view
                .child(detail(&t!("Frequency"), format!("{frequency_hz:.1} Hz")))
                .child(
                    div()
                        .flex()
                        .gap(px(theme::SPACE_1))
                        .child(button("tone-down", "−10 Hz").on_click(edit(
                            cx.entity(),
                            GraphEdit::BindSource {
                                node: id,
                                source: InputSelection::Tone {
                                    frequency_hz: (frequency_hz - 10.).max(1.),
                                },
                            },
                        )))
                        .child(button("tone-up", "+10 Hz").on_click(edit(
                            cx.entity(),
                            GraphEdit::BindSource {
                                node: id,
                                source: InputSelection::Tone {
                                    frequency_hz: (frequency_hz + 10.).min(23999.),
                                },
                            },
                        ))),
                );
        }
    }
    view
}

pub(super) fn sink_section(
    mut view: Stateful<Div>,
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    id: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let bound_export = snapshot
        .graph
        .exports
        .iter()
        .find(|export| export.enabled && export.binding == Some(id));
    view = view.child(heading(t!("Output device"))).child(label(
        bound_export
            .map(|export| export.name.clone())
            .or_else(|| {
                entry
                    .output_endpoint_id
                    .as_ref()
                    .map(|endpoint| crate::ui::presentation::output_name(snapshot, endpoint))
            })
            .unwrap_or_else(|| t!("Not bound · choose an output").into()),
    ));
    // The shared sink choice list, same as the chip menu (issue #119).
    for choice in binding_choices::sink_choices(snapshot, entry) {
        view = view.child(binding_choice_button(cx.entity(), choice));
    }
    view
}

/// The plugin picker section: the shared choice list over the service's
/// plugin library (the same entries the chip menu offers), plus the
/// library's scan failures so exclusions are visible, never silent.
pub(super) fn plugin_section(
    mut view: Stateful<Div>,
    snapshot: &crate::service::AppSnapshot,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let assigned = entry
        .plugin
        .as_ref()
        .and_then(|assignment| snapshot.plugins.get(&assignment.id))
        .map(|plugin| plugin.descriptor.name.clone());
    view = view.child(heading(t!("Plugin"))).child(label(
        assigned.unwrap_or_else(|| t!("Not assigned · choose a plugin").into()),
    ));
    for choice in binding_choices::plugin_choices(snapshot, entry) {
        view = view.child(binding_choice_button(cx.entity(), choice));
    }
    if snapshot.plugins.failures.is_empty() {
        view = view.child(label(t!(
            "%{count} plugin(s) in the library",
            count = snapshot.plugins.len()
        )));
    } else {
        view = view.child(label(t!(
            "%{count} plugin(s), %{failed} excluded by failed scans",
            count = snapshot.plugins.len(),
            failed = snapshot.plugins.failures.len()
        )));
    }
    view
}

pub(super) fn gain_section(
    mut view: Stateful<Div>,
    shell: &StudioShell,
    entry: &crate::service::GraphNodeState,
    id: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Gain")))
        .child(detail_editable(
            &t!("Requested"),
            SharedString::from(format!("inspector-node-gain-{id:?}")),
            db_text(entry.gain),
            shell.mixer.presentation.editing_node_gain == Some(id),
            &shell.mixer.presentation.node_gain_inputs[&id],
            start_value_edit(
                cx.entity(),
                EditableValueFamily::NodeGain,
                id,
                move |graph| {
                    graph
                        .node(id)
                        .map(|node| gain_db_text(node.gain))
                        .unwrap_or_default()
                },
            ),
        ))
        .child(
            div()
                .flex()
                .gap(px(theme::SPACE_1))
                .child(button("node-gain-down", "−1 dB").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetGain {
                        node: id,
                        gain: step_db(entry.gain, -1., 0., 16.),
                    },
                )))
                .child(button("node-gain-up", "+1 dB").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetGain {
                        node: id,
                        // Silence cannot be multiplied into something
                        // audible: the first up-step seeds a finite value.
                        gain: if entry.gain == 0. {
                            0.01
                        } else {
                            step_db(entry.gain, 1., 0., 16.)
                        },
                    },
                )))
                .child(
                    button("node-unity", t!("Unity"))
                        .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 1. })),
                ),
        )
        .child(
            button("node-silence", t!("Silence"))
                .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 0. })),
        );
    view
}

pub(super) fn pan_section(
    mut view: Stateful<Div>,
    shell: &StudioShell,
    entry: &crate::service::GraphNodeState,
    id: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Stereo controls")))
        .child(detail_editable(
            &t!("Balance"),
            SharedString::from(format!("inspector-node-pan-{id:?}")),
            format!("{:+.2}", entry.pan),
            shell.mixer.presentation.editing_node_pan == Some(id),
            &shell.mixer.presentation.node_pan_inputs[&id],
            start_value_edit(
                cx.entity(),
                EditableValueFamily::NodePan,
                id,
                move |graph| {
                    graph
                        .node(id)
                        .map(|node| format!("{:+.2}", node.pan))
                        .unwrap_or_default()
                },
            ),
        ))
        .child(
            div()
                .flex()
                .gap(px(theme::SPACE_1))
                .child(button("node-left", t!("Left")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetPan {
                        node: id,
                        pan: (entry.pan - 0.1).max(-1.),
                    },
                )))
                .child(
                    button("node-center", t!("Center"))
                        .on_click(edit(cx.entity(), GraphEdit::SetPan { node: id, pan: 0. })),
                )
                .child(button("node-right", t!("Right")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetPan {
                        node: id,
                        pan: (entry.pan + 0.1).min(1.),
                    },
                ))),
        )
        .child(stereo_controls(entry, cx));
    view
}

pub(super) fn compressor_section(
    mut view: Stateful<Div>,
    shell: &StudioShell,
    entry: &crate::service::GraphNodeState,
    id: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view.child(heading(t!("Compressor"))).child(label(t!(
        "Linked peak detector · Enter or leave a field to apply"
    )));
    for parameter in crate::service::CompressorParameter::ALL {
        view = view.child(
            div()
                .id(format!("compressor-{:?}", parameter.id()))
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_1))
                .child(detail(
                    &t!(parameter.label()),
                    parameter.display(entry.compressor.value(parameter)),
                ))
                .child(
                    component::input::Input::new(
                        &shell.inspector.compressor.inputs[&(id, parameter)],
                    )
                    .id(format!("compressor-input-{}", parameter.id().0))
                    .bg(theme::controls::RAISED)
                    .text_color(theme::TEXT)
                    .border_color(theme::BORDER),
                )
                .child({
                    let slot = &shell.inspector.compressor.sliders[&(id, parameter)];
                    hslider(&slot.state, &slot.behavior, cx)
                })
                .gpui_test_observe(),
        );
    }
    view
}

/// One shared-list choice rendered as the inspector's pick button
/// (issue #119): the same entry the chip menu offers, under this panel's
/// element-id namespace.
fn binding_choice_button(
    this: Entity<StudioShell>,
    choice: binding_choices::BindingChoice,
) -> impl IntoElement {
    button(choice.target.key(), choice.label)
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                match choice.action.clone() {
                    binding_choices::BindingAction::Edit(edit) => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                    }
                    // The refresh keeps the section mounted: the list
                    // re-renders from the refreshed catalog.
                    binding_choices::BindingAction::Refresh => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::RefreshCatalog));
                    }
                    binding_choices::BindingAction::ScanPlugins => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::ScanPlugins));
                    }
                }
                cx.notify();
            })
        })
}
