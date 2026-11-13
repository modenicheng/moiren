//! The one ordered binding choice list for source and sink nodes (issue
//! #119, design 21 §3.4). The canvas chip's dropdown (`binding_menu`) and
//! the node inspector's Source/Output sections render the same entries
//! from here, so the two surfaces can no longer drift apart in content or
//! order — they previously kept two ~130-line copies in step by comment
//! alone. Endpoint rows follow the snapshot's active backend
//! (`snapshot.audio_settings.backend`) rather than a hardcoded one, so an
//! ASIO session lists its drivers on both surfaces.
//!
//! A source's list is class-filtered (2026-11-13 OBS-style split): the
//! template a node was created from — or, once bound, its live binding —
//! picks one of [`SourceClass`], and the list offers only that class's
//! resources plus the shared Clear/Refresh chrome.

use super::*;
use crate::service::{
    AppSnapshot, ExportDirection, ExportIntent, GraphEdit, GraphNodeState, InputSelection,
};
use moiren_core::graph::{NodeId, NodeKind};

use super::node_catalog::SourceClass;

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
/// `stored` is the presentation-tier source class remembered for the node
/// (spawn template or restored project intent), when the shell has one.
pub(crate) fn choices(
    snapshot: &AppSnapshot,
    id: NodeId,
    stored: Option<SourceClass>,
) -> Option<Vec<BindingChoice>> {
    let kind = snapshot.graph.graph.get_node(id).ok()?.kind();
    let entry = snapshot.graph.node(id)?;
    Some(match kind {
        NodeKind::Source => {
            let class = effective_source_class(snapshot, entry, stored);
            source_choices(snapshot, entry, class)
        }
        NodeKind::Sink => sink_choices(snapshot, entry),
        _ => return None,
    })
}

/// The class a source node currently presents. The stored class is the
/// single source of truth for what a source node *is*: once a node carries
/// one (spawned from a template, restored from a project, or materialized
/// by the render sync), the live binding can never re-class it — the
/// binding only names *where* the node is wired. The binding-derived
/// fallback below exists solely for nodes that predate the split (the
/// default rig, older projects) on the frames before the render sync
/// materializes their class, and it freezes the moment that happens.
pub(crate) fn effective_source_class(
    snapshot: &AppSnapshot,
    entry: &GraphNodeState,
    stored: Option<SourceClass>,
) -> SourceClass {
    if let Some(class) = stored {
        return class;
    }
    match entry.source.as_ref() {
        Some(InputSelection::Process { .. }) => SourceClass::Application,
        Some(InputSelection::Physical { endpoint_id }) => {
            if crate::ui::state::is_virtual_endpoint(&snapshot.catalog, endpoint_id) {
                SourceClass::Virtual
            } else {
                SourceClass::Physical
            }
        }
        Some(InputSelection::Tone { .. }) => SourceClass::Tone,
        None => SourceClass::Physical,
    }
}

/// The source picker's ordered list for one class: clear (only when
/// something is bound), the class's resource rows, refresh. The tone
/// generator is a class of its own, so it appears only on tone nodes.
pub(crate) fn source_choices(
    snapshot: &AppSnapshot,
    entry: &GraphNodeState,
    class: SourceClass,
) -> Vec<BindingChoice> {
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
    if class == SourceClass::Tone {
        choices.push(BindingChoice {
            target: BindingTarget::Tone { frequency_hz: 440. },
            label: t!("Tone generator · 440 Hz").to_string(),
            current: matches!(entry.source, Some(InputSelection::Tone { .. })),
            action: BindingAction::Edit(GraphEdit::BindSource {
                node: id,
                source: InputSelection::Tone { frequency_hz: 440. },
            }),
        });
    }
    if class == SourceClass::Physical {
        for row in crate::ui::state::input_rows(&snapshot.catalog, snapshot.audio_settings.backend)
        {
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
    }
    if class == SourceClass::Virtual {
        // Managed virtual devices first (the export names), then any owned
        // endpoint row no enabled export currently claims — its Windows
        // device is present but not offered as an export row.
        let claimed: std::collections::BTreeSet<&str> = snapshot
            .graph
            .exports
            .iter()
            .filter(|export| export.enabled)
            .filter_map(|export| {
                snapshot
                    .catalog
                    .vad_endpoints
                    .get(&export.endpoint_id)
                    .map(String::as_str)
            })
            .collect();
        for export in playback_exports(snapshot) {
            // The export row is current either through its own binding or
            // when the node holds a direct binding to the endpoint the
            // export's device currently owns — the raw row is deduped away,
            // so this row carries the marker for both.
            let owned_endpoint = snapshot.catalog.vad_endpoints.get(&export.endpoint_id);
            let current = export.binding == Some(id)
                || matches!(
                    (&entry.source, owned_endpoint),
                    (Some(InputSelection::Physical { endpoint_id }), Some(owned))
                        if endpoint_id == owned
                );
            choices.push(BindingChoice {
                target: BindingTarget::PlaybackExport {
                    endpoint_id: export.endpoint_id.to_string(),
                },
                label: export.name.clone(),
                current,
                action: BindingAction::Edit(GraphEdit::BindExport {
                    endpoint_id: export.endpoint_id,
                    node: Some(id),
                }),
            });
        }
        for row in
            crate::ui::state::virtual_input_rows(&snapshot.catalog, snapshot.audio_settings.backend)
        {
            if claimed.contains(row.endpoint_id.as_str()) {
                continue;
            }
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
    }
    if class == SourceClass::Application {
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
    use super::{BindingChoice, SourceClass, effective_source_class, source_choices};
    use crate::service::InputSelection;
    use moiren_core::graph::NodeKind;
    use moiren_engine::audio_config::AudioBackend;

    fn snapshot(backend: AudioBackend) -> crate::service::AppSnapshot {
        let mut context = crate::service::ServiceContext::default();
        context.audio_settings.backend = backend;
        let virtual_id = moiren_engine::node::VirtualEndpointId::from_bytes([0x42; 16]);
        context.catalog = std::sync::Arc::new(crate::service::DeviceCatalog {
            inputs: vec![
                crate::service::DeviceRow {
                    endpoint_id: "microphone".into(),
                    name: "Microphone".into(),
                },
                crate::service::DeviceRow {
                    endpoint_id: "vad-endpoint".into(),
                    name: "VAD device".into(),
                },
            ],
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
            vad_endpoints: [(virtual_id, "vad-endpoint".into())].into_iter().collect(),
            ..Default::default()
        });
        let mut snapshot = crate::service::snapshot(&context);
        std::sync::Arc::make_mut(&mut snapshot.graph).exports =
            vec![crate::service::ExportIntent {
                endpoint_id: virtual_id,
                direction: crate::service::ExportDirection::Playback,
                name: "Game sink".into(),
                channels: 2,
                enabled: true,
                binding: None,
            }];
        snapshot
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

    fn unbound_entry(snapshot: &mut crate::service::AppSnapshot) -> crate::service::GraphNodeState {
        let entry = source_entry(snapshot).clone();
        let owned = std::sync::Arc::make_mut(&mut snapshot.graph);
        owned
            .nodes
            .iter_mut()
            .find(|state| state.id == entry.id)
            .unwrap()
            .source = None;
        source_entry(snapshot).clone()
    }

    /// Design 23 §6 (audit fact F10): the engine constrains application
    /// sources by the input node's own device options, not by the output
    /// backend — the picker lists applications under ASIO too, and the old
    /// devices-page tab that claimed otherwise is gone.
    #[test]
    fn source_choices_list_applications_under_asio_as_well() {
        let snapshot = snapshot(AudioBackend::Asio);
        let choices = source_choices(&snapshot, source_entry(&snapshot), SourceClass::Application);
        let labels = labels(&choices);
        assert!(labels.contains(&"Player"), "ASIO lists the application row");
        assert!(
            !labels.contains(&"ASIO interface"),
            "ASIO drivers belong to the physical class only"
        );
        assert!(!labels.contains(&"Microphone"), "WASAPI-only rows stay out");
    }

    #[test]
    fn source_choices_list_physical_inputs_and_processes_under_wasapi() {
        let snapshot = snapshot(AudioBackend::Wasapi);
        let physical = source_choices(&snapshot, source_entry(&snapshot), SourceClass::Physical);
        let physical_labels = labels(&physical);
        assert!(physical_labels.contains(&"Microphone"));
        assert!(
            !physical_labels.contains(&"Player"),
            "processes are another class"
        );
        assert!(!physical_labels.contains(&"ASIO interface"));
        assert!(
            !physical_labels.contains(&"VAD device"),
            "virtual endpoints stay out of the physical class"
        );
        let application =
            source_choices(&snapshot, source_entry(&snapshot), SourceClass::Application);
        // The default rig's source stays tone-bound, so its Clear entry
        // leads even under another class's list.
        assert_eq!(
            labels(&application),
            vec!["Clear binding", "Player", "Refresh sources"]
        );
    }

    /// The virtual class lists the managed export row plus any owned
    /// endpoint row no enabled export claims — never physical or process
    /// rows.
    #[test]
    fn virtual_class_lists_exports_and_unclaimed_vad_rows_only() {
        {
            let mut snapshot = snapshot(AudioBackend::Wasapi);
            // Detach the export from the endpoint: the raw VAD row then
            // stands alone, which is the dedupe's other half.
            let entry = unbound_entry(&mut snapshot);
            std::sync::Arc::make_mut(&mut snapshot.graph).exports = Vec::new();
            let choices = source_choices(&snapshot, &entry, SourceClass::Virtual);
            assert_eq!(
                labels(&choices),
                vec!["VAD device", "Refresh sources"],
                "an unclaimed owned endpoint is offered directly"
            );
        }
        let mut with_export = snapshot(AudioBackend::Wasapi);
        let entry = unbound_entry(&mut with_export);
        let choices = source_choices(&with_export, &entry, SourceClass::Virtual);
        assert_eq!(
            labels(&choices),
            vec!["Game sink", "Refresh sources"],
            "the export row claims its endpoint; the raw row does not repeat it"
        );
        // Binding the raw endpoint of the same device is current-marker
        // compatible: an export binding and a Physical binding are distinct
        // targets for one device.
        let bound = crate::service::GraphNodeState {
            source: Some(InputSelection::Physical {
                endpoint_id: "vad-endpoint".into(),
            }),
            ..entry.clone()
        };
        let choices = source_choices(&with_export, &bound, SourceClass::Virtual);
        assert!(choices.iter().any(|choice| choice.current));
    }

    /// The bound-to text resolves a VAD endpoint through the virtual rows,
    /// so a virtual-class source names its device instead of reading
    /// "Unavailable input".
    #[test]
    fn source_name_resolves_virtual_endpoints() {
        let snapshot = snapshot(AudioBackend::Wasapi);
        assert_eq!(
            crate::ui::presentation::source_name(
                &snapshot,
                &InputSelection::Physical {
                    endpoint_id: "vad-endpoint".into(),
                },
            ),
            "VAD device",
            "owned endpoints resolve through the virtual rows"
        );
        assert_eq!(
            crate::ui::presentation::source_name(
                &snapshot,
                &InputSelection::Physical {
                    endpoint_id: "vanished".into(),
                },
            ),
            "Unavailable input",
        );
    }

    /// The tone class keeps the generator picker; the physical class drops
    /// it — a tone node and a mic node no longer offer each other's
    /// resources (the OBS-style split).
    #[test]
    fn tone_class_keeps_the_generator_and_other_classes_drop_it() {
        let snapshot = snapshot(AudioBackend::Wasapi);
        let entry = source_entry(&snapshot);
        let tone = source_choices(&snapshot, entry, SourceClass::Tone);
        assert_eq!(
            labels(&tone),
            vec![
                "Clear binding",
                "Tone generator · 440 Hz",
                "Refresh sources"
            ]
        );
        let physical = source_choices(&snapshot, entry, SourceClass::Physical);
        assert!(!labels(&physical).contains(&"Tone generator · 440 Hz"));
    }

    /// The stored class is the single source of truth for a source node's
    /// presentation identity: once present, no binding — not even one from
    /// another class, e.g. restored by undo or an older project — can
    /// re-class the node. The binding only infers the class for nodes that
    /// predate the split, until the render sync materializes it.
    #[test]
    fn effective_class_follows_the_stored_intent_before_the_binding() {
        let snapshot = snapshot(AudioBackend::Wasapi);
        let base = source_entry(&snapshot).clone();
        for (source, stored) in [
            (
                Some(InputSelection::Physical {
                    endpoint_id: "vad-endpoint".into(),
                }),
                SourceClass::Physical,
            ),
            (
                Some(InputSelection::Physical {
                    endpoint_id: "microphone".into(),
                }),
                SourceClass::Virtual,
            ),
            (
                Some(InputSelection::Process {
                    pid: 42,
                    creation_time_100ns: 7,
                }),
                SourceClass::Tone,
            ),
        ] {
            let entry = crate::service::GraphNodeState {
                source: source.clone(),
                ..base.clone()
            };
            // A live binding never overrides the stored identity.
            assert_eq!(
                effective_source_class(&snapshot, &entry, Some(stored)),
                stored,
                "binding {source:?} must not re-class a {stored:?} node"
            );
        }
        // Absent classes infer once from the binding — the bootstrap for
        // the default rig and pre-split projects.
        for (source, expected) in [
            (
                Some(InputSelection::Physical {
                    endpoint_id: "vad-endpoint".into(),
                }),
                SourceClass::Virtual,
            ),
            (
                Some(InputSelection::Physical {
                    endpoint_id: "microphone".into(),
                }),
                SourceClass::Physical,
            ),
            (
                Some(InputSelection::Process {
                    pid: 42,
                    creation_time_100ns: 7,
                }),
                SourceClass::Application,
            ),
            (
                Some(InputSelection::Tone { frequency_hz: 440. }),
                SourceClass::Tone,
            ),
            (None, SourceClass::Physical),
        ] {
            let entry = crate::service::GraphNodeState {
                source: source.clone(),
                ..base.clone()
            };
            assert_eq!(
                effective_source_class(&snapshot, &entry, None),
                expected,
                "an absent class with binding {source:?} must bootstrap as {expected:?}"
            );
        }
    }
}
