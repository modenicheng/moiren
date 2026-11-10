//! The one ordered binding choice list for source and sink nodes (issue
//! #119, design 21 §3.4). The canvas chip's dropdown (`binding_menu`) and
//! the node inspector's Source/Output sections render the same entries
//! from here, so the two surfaces can no longer drift apart in content or
//! order — they previously kept two ~130-line copies in step by comment
//! alone. Endpoint rows follow the snapshot's active backend
//! (`snapshot.audio_settings.backend`) rather than a hardcoded one, so an
//! ASIO session lists its drivers on both surfaces.

use super::*;
use crate::service::{
    AppSnapshot, ExportDirection, ExportIntent, GraphEdit, GraphNodeState, InputSelection,
};
use moiren_core::graph::{NodeId, NodeKind};

/// What picking a choice does: dispatch one graph edit, or reload the
/// catalog the list is built from. The refresh keeps the picker mounted;
/// an edit also closes the chip menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BindingAction {
    Edit(GraphEdit),
    Refresh,
}

/// What one choice binds. Also the stable identity both surfaces derive
/// their element ids from (the menu prefixes its `chip-` namespace), so
/// the same choice is addressable under each surface's own ids.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BindingTarget {
    Clear,
    Tone { frequency_hz: f64 },
    Input { endpoint_id: String },
    PlaybackExport { endpoint_id: String },
    Process { pid: u32, creation_time_100ns: u64 },
    Refresh,
    Output { endpoint_id: String },
    MicrophoneExport { endpoint_id: String },
}

impl BindingTarget {
    /// The choice's stable key, unique within one node's list.
    pub(crate) fn key(&self) -> SharedString {
        match self {
            Self::Clear => "unbind".into(),
            Self::Tone { .. } => "bind-tone".into(),
            Self::Input { endpoint_id } => format!("bind-input-{endpoint_id}").into(),
            Self::PlaybackExport { endpoint_id } => {
                format!("bind-playback-export-{endpoint_id}").into()
            }
            Self::Process { pid, .. } => format!("bind-process-{pid}").into(),
            Self::Refresh => "refresh".into(),
            Self::Output { endpoint_id } => format!("bind-output-{endpoint_id}").into(),
            Self::MicrophoneExport { endpoint_id } => {
                format!("bind-microphone-export-{endpoint_id}").into()
            }
        }
    }
}

/// One rebinding choice: what it binds, the user-visible label both
/// surfaces render for it, and whether it is the node's current binding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BindingChoice {
    pub target: BindingTarget,
    pub label: String,
    pub current: bool,
    pub action: BindingAction,
}

/// The chip menu's entries for a node: its kind's picker list, or `None`
/// for kinds that carry no endpoint binding (the menu never opens).
pub(crate) fn choices(snapshot: &AppSnapshot, id: NodeId) -> Option<Vec<BindingChoice>> {
    let kind = snapshot.graph.graph.get_node(id).ok()?.kind();
    let entry = snapshot.graph.node(id)?;
    Some(match kind {
        NodeKind::Source => source_choices(snapshot, entry),
        NodeKind::Sink => sink_choices(snapshot, entry),
        _ => return None,
    })
}

/// The source picker's ordered list: clear (only when something is bound),
/// tone generator, physical inputs, playback exports, processes, refresh.
pub(crate) fn source_choices(snapshot: &AppSnapshot, entry: &GraphNodeState) -> Vec<BindingChoice> {
    let id = entry.id;
    let mut choices = Vec::new();
    if entry.source.is_some() || bound_export(snapshot, id).is_some() {
        choices.push(BindingChoice {
            target: BindingTarget::Clear,
            label: t!("Clear binding").to_string(),
            current: false,
            action: BindingAction::Edit(GraphEdit::ClearBinding(id)),
        });
    }
    choices.push(BindingChoice {
        target: BindingTarget::Tone { frequency_hz: 440. },
        label: t!("Tone generator · 440 Hz").to_string(),
        current: matches!(entry.source, Some(InputSelection::Tone { .. })),
        action: BindingAction::Edit(GraphEdit::BindSource {
            node: id,
            source: InputSelection::Tone { frequency_hz: 440. },
        }),
    });
    for row in crate::ui::state::input_rows(&snapshot.catalog, snapshot.audio_settings.backend) {
        choices.push(BindingChoice {
            target: BindingTarget::Input {
                endpoint_id: row.endpoint_id.clone(),
            },
            label: row.name.clone(),
            current: matches!(
                &entry.source,
                Some(InputSelection::Physical { endpoint_id }) if *endpoint_id == row.endpoint_id
            ),
            action: BindingAction::Edit(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Physical {
                    endpoint_id: row.endpoint_id,
                },
            }),
        });
    }
    for export in playback_exports(snapshot) {
        choices.push(BindingChoice {
            target: BindingTarget::PlaybackExport {
                endpoint_id: export.endpoint_id.to_string(),
            },
            label: export.name.clone(),
            current: export.binding == Some(id),
            action: BindingAction::Edit(GraphEdit::BindExport {
                endpoint_id: export.endpoint_id,
                node: Some(id),
            }),
        });
    }
    for row in &snapshot.catalog.processes {
        choices.push(BindingChoice {
            target: BindingTarget::Process {
                pid: row.pid,
                creation_time_100ns: row.creation_time_100ns,
            },
            label: row.name.clone(),
            current: matches!(
                &entry.source,
                Some(InputSelection::Process { pid, .. }) if *pid == row.pid
            ),
            action: BindingAction::Edit(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Process {
                    pid: row.pid,
                    creation_time_100ns: row.creation_time_100ns,
                },
            }),
        });
    }
    choices.push(BindingChoice {
        target: BindingTarget::Refresh,
        label: t!("Refresh sources").to_string(),
        current: false,
        action: BindingAction::Refresh,
    });
    choices
}

/// The sink picker's ordered list: clear (only when something is bound),
/// physical outputs, microphone exports.
pub(crate) fn sink_choices(snapshot: &AppSnapshot, entry: &GraphNodeState) -> Vec<BindingChoice> {
    let id = entry.id;
    let mut choices = Vec::new();
    if entry.output_endpoint_id.is_some() || bound_export(snapshot, id).is_some() {
        choices.push(BindingChoice {
            target: BindingTarget::Clear,
            label: t!("Clear binding").to_string(),
            current: false,
            action: BindingAction::Edit(GraphEdit::ClearBinding(id)),
        });
    }
    for row in crate::ui::state::output_rows(&snapshot.catalog, snapshot.audio_settings.backend) {
        choices.push(BindingChoice {
            target: BindingTarget::Output {
                endpoint_id: row.endpoint_id.clone(),
            },
            label: row.name.clone(),
            current: entry.output_endpoint_id.as_deref() == Some(row.endpoint_id.as_str()),
            action: BindingAction::Edit(GraphEdit::BindOutput {
                node: id,
                endpoint_id: row.endpoint_id,
            }),
        });
    }
    for export in microphone_exports(snapshot) {
        choices.push(BindingChoice {
            target: BindingTarget::MicrophoneExport {
                endpoint_id: export.endpoint_id.to_string(),
            },
            label: export.name.clone(),
            current: export.binding == Some(id),
            action: BindingAction::Edit(GraphEdit::BindExport {
                endpoint_id: export.endpoint_id,
                node: Some(id),
            }),
        });
    }
    choices
}

/// The enabled export bound to the node, if any: an export claims a node's
/// binding away from the picker's own selections.
fn bound_export(snapshot: &AppSnapshot, id: NodeId) -> Option<&ExportIntent> {
    snapshot
        .graph
        .exports
        .iter()
        .find(|export| export.enabled && export.binding == Some(id))
}

fn playback_exports(snapshot: &AppSnapshot) -> impl Iterator<Item = &ExportIntent> {
    snapshot
        .graph
        .exports
        .iter()
        .filter(|export| export.enabled && export.direction == ExportDirection::Playback)
}

fn microphone_exports(snapshot: &AppSnapshot) -> impl Iterator<Item = &ExportIntent> {
    snapshot
        .graph
        .exports
        .iter()
        .filter(|export| export.enabled && export.direction == ExportDirection::Microphone)
}

#[cfg(test)]
mod tests {
    // No `use super::*` here: the glob re-exports gpui_kit's `test`, which
    // shadows the std #[test] attribute (the PR #28 trap).
    use super::{BindingChoice, source_choices};
    use moiren_core::graph::NodeKind;
    use moiren_engine::audio_config::AudioBackend;

    fn snapshot(backend: AudioBackend) -> crate::service::AppSnapshot {
        let mut context = crate::service::ServiceContext::default();
        context.audio_settings.backend = backend;
        context.catalog = std::sync::Arc::new(crate::service::DeviceCatalog {
            inputs: vec![crate::service::DeviceRow {
                endpoint_id: "microphone".into(),
                name: "Microphone".into(),
            }],
            outputs: vec![crate::service::DeviceRow {
                endpoint_id: "speakers".into(),
                name: "Speakers".into(),
            }],
            asio_drivers: vec![crate::service::DeviceRow {
                endpoint_id: "asio:{driver}".into(),
                name: "ASIO interface".into(),
            }],
            processes: vec![crate::service::ProcessRow {
                pid: 42,
                creation_time_100ns: 7,
                name: "Player".into(),
            }],
            ..Default::default()
        });
        crate::service::snapshot(&context)
    }

    fn labels(choices: &[BindingChoice]) -> Vec<&str> {
        choices.iter().map(|choice| choice.label.as_str()).collect()
    }

    fn source_entry(snapshot: &crate::service::AppSnapshot) -> &crate::service::GraphNodeState {
        let node = snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Source)
            .unwrap();
        snapshot
            .graph
            .nodes
            .iter()
            .find(|entry| entry.id == node.id())
            .unwrap()
    }

    /// Design 23 §6 (audit fact F10): the engine constrains application
    /// sources by the input node's own device options, not by the output
    /// backend — the picker lists applications under ASIO too, and the old
    /// devices-page tab that claimed otherwise is gone.
    #[test]
    fn source_choices_list_applications_under_asio_as_well() {
        let snapshot = snapshot(AudioBackend::Asio);
        let choices = source_choices(&snapshot, source_entry(&snapshot));
        let labels = labels(&choices);
        assert!(labels.contains(&"Player"), "ASIO lists the application row");
        assert!(
            labels.contains(&"ASIO interface"),
            "ASIO lists its drivers for the input direction"
        );
        assert!(!labels.contains(&"Microphone"), "WASAPI-only rows stay out");
    }

    #[test]
    fn source_choices_list_physical_inputs_and_processes_under_wasapi() {
        let snapshot = snapshot(AudioBackend::Wasapi);
        let choices = source_choices(&snapshot, source_entry(&snapshot));
        let labels = labels(&choices);
        assert!(labels.contains(&"Microphone"));
        assert!(labels.contains(&"Player"));
        assert!(!labels.contains(&"ASIO interface"));
    }
}
