//! Surface-local Rack editors; the service remains the owner of stage values.

pub(in crate::ui::window::graph::node_controls) use super::parameters::Field;
use super::parameters::{self, Projection};
use crate::service::{AppRequest, GraphEdit, GraphNodeState, GraphSnapshot, RackEdit};
use crate::ui::widgets::fader::SliderBehavior;
use crate::ui::window::{
    StudioShell,
    mixer::{SliderEdit, SliderSlot, commit_closure, sync_slider_range},
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext, Context, Entity, FocusHandle, Focusable, Window};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window::graph::node_controls) enum Surface {
    Card,
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window::graph::node_controls) struct Key {
    pub surface: Surface,
    pub node: NodeId,
    pub epoch: u64,
    pub slot: u32,
    pub field: Field,
}

#[derive(Default)]
pub(in crate::ui::window::graph::node_controls) struct RackEditorState {
    pub selected: BTreeMap<NodeId, u32>,
    pub inputs: BTreeMap<Key, Entity<InputState>>,
    pub baselines: BTreeMap<Key, String>,
    pub errors: BTreeMap<Key, String>,
    pub sliders: BTreeMap<Key, SliderSlot>,
    /// Zero-based (output row page, input column page), four cells per axis.
    pub mapper_pages: BTreeMap<(Surface, NodeId), (usize, usize)>,
    /// Retained invalid drafts are not active controls or keyboard owners.
    active: BTreeSet<Key>,
}

impl RackEditorState {
    pub(in crate::ui::window::graph::node_controls) fn selected_slot(
        &self,
        node: NodeId,
        settings: &RackSettings,
    ) -> Option<u32> {
        self.selected
            .get(&node)
            .copied()
            .filter(|id| settings.slots.iter().any(|slot| slot.id == *id))
            .or_else(|| settings.slots.first().map(|slot| slot.id))
    }

    pub(in crate::ui::window::graph::node_controls) fn owns_focus(
        &self,
        focused: &FocusHandle,
        cx: &App,
    ) -> bool {
        self.inputs.iter().any(|(key, input)| {
            self.active.contains(key) && input.read(cx).focus_handle(cx) == *focused
        })
    }
}

struct Stage<'a> {
    owner: &'a GraphNodeState,
    slot: &'a RackSlot,
    inputs: usize,
    outputs: usize,
}

fn resolve_stage(graph: &GraphSnapshot, node: NodeId, epoch: u64, slot: u32) -> Option<Stage<'_>> {
    if graph.rack_epochs.get(&node).copied()? != epoch || graph.kind_of(node)? != NodeKind::Rack {
        return None;
    }
    let owner = graph.node(node)?;
    let settings = owner.builtin.rack()?;
    let slot = settings
        .slots
        .iter()
        .find(|candidate| candidate.id == slot)?;
    let topology = graph.graph.get_node(node).ok()?;
    let inputs = topology
        .inputs()
        .iter()
        .map(|port| port.channels())
        .collect::<Vec<_>>();
    let outputs = topology
        .outputs()
        .iter()
        .map(|port| port.channels())
        .collect::<Vec<_>>();
    let layout = settings.resolve_layout(&inputs, &outputs)?;
    let inputs = layout.slots.get(&slot.id)?.input_channels;
    // A bypassed Mapper's effective route width is NOT its configured matrix
    // width; editing must retain its dormant rows until it can be enabled.
    let outputs = slot.processor.output_channels(inputs).ok()?;
    Some(Stage {
        owner,
        slot,
        inputs,
        outputs,
    })
}

fn field_value(stage: &Stage<'_>, field: Field) -> Option<f64> {
    match field {
        Field::Scalar(_) => Projection::new(stage.owner, &stage.slot.processor)?.value(field),
        Field::Weight { input, output } => {
            let RackProcessor::ChannelMapper(settings) = &stage.slot.processor else {
                return None;
            };
            parameters::weight(settings, stage.inputs, stage.outputs, input, output)
        }
    }
}

/// Resolve again for every event. Stable IDs plus the service epoch prevent
/// old closures from editing replacement slots, including same-type ID reuse.
fn edit_for(graph: &GraphSnapshot, key: Key, value: f64) -> Option<GraphEdit> {
    let stage = resolve_stage(graph, key.node, key.epoch, key.slot)?;
    let edit = match key.field {
        Field::Scalar(_) => {
            let projection = Projection::new(stage.owner, &stage.slot.processor)?;
            let (parameter, value) = projection.scalar_intent(key.field, value)?;
            RackEdit::SetParameter {
                slot: key.slot,
                parameter,
                value,
            }
        }
        Field::Weight { input, output } => {
            field_value(&stage, key.field)?;
            if !parameters::valid_weight(value) {
                return None;
            }
            RackEdit::SetMatrixWeight {
                slot: key.slot,
                input,
                output,
                value,
            }
        }
    };
    Some(GraphEdit::EditRack {
        node: key.node,
        epoch: key.epoch,
        edit,
    })
}

fn dirty(text: &str, baseline: Option<&String>) -> bool {
    baseline.is_none_or(|baseline| baseline != text)
}

fn commit_input(
    shell: &mut StudioShell,
    input: &Entity<InputState>,
    key: Key,
    cx: &mut Context<StudioShell>,
) {
    // A detached field may still deliver Blur after switching stage/page.
    if shell
        .node_controls
        .rack
        .inputs
        .get(&key)
        .map(|input| input.entity_id())
        != Some(input.entity_id())
    {
        return;
    }
    let text = input.read(cx).value().to_string();
    if !dirty(&text, shell.node_controls.rack.baselines.get(&key)) {
        shell.node_controls.rack.errors.remove(&key);
        cx.notify();
        return;
    }
    let graph = &shell.state.read(cx).snapshot.graph;
    let Some(stage) = resolve_stage(graph, key.node, key.epoch, key.slot) else {
        return;
    };
    let value = key.field.parse(&text);
    let request = value.and_then(|value| edit_for(graph, key, value));
    let Some(request) = request else {
        let message = match key.field {
            Field::Scalar(_) => Projection::new(stage.owner, &stage.slot.processor)
                .and_then(|projection| projection.range(key.field))
                .map(|(min, max)| format!("{min:.2} .. {max:.2} {}", key.field.unit()))
                .unwrap_or_else(|| String::from("Unavailable for this stage")),
            Field::Weight { .. } => String::from("Enter a finite routing weight"),
        };
        shell.node_controls.rack.errors.insert(key, message);
        cx.notify();
        return;
    };
    // A queued knob edit may not yet appear in the snapshot. A genuinely
    // edited text field must still submit its final value behind that request.
    let changed = value != field_value(&stage, key.field) || shell.state.read(cx).pending_request;
    let sent = !changed
        || shell
            .state
            .update(cx, |state, _| {
                state.try_dispatch(AppRequest::EditGraph(request))
            })
            .is_ok();
    if sent {
        shell.node_controls.rack.baselines.insert(key, text);
        shell.node_controls.rack.errors.remove(&key);
    } else {
        shell
            .node_controls
            .rack
            .errors
            .insert(key, String::from("Edit could not be sent; try again"));
    }
    cx.notify();
}

fn commit_outgoing(
    shell: &mut StudioShell,
    next_active: &BTreeSet<Key>,
    cx: &mut Context<StudioShell>,
) {
    let state = &shell.node_controls.rack;
    let outgoing = state
        .active
        .difference(next_active)
        .filter_map(|key| {
            let input = state.inputs.get(key)?;
            (state.errors.contains_key(key)
                || dirty(input.read(cx).value().as_str(), state.baselines.get(key)))
            .then(|| (*key, input.clone()))
        })
        .collect::<Vec<_>>();
    // Flush once on departure, before pruning. Hidden panes may never emit
    // Blur; retrying retained drafts every render would resend failed edits.
    for (key, input) in outgoing {
        commit_input(shell, &input, key, cx);
    }
}

/// Both surfaces keep independent input/focus/drag state for the selected
/// stable slot; changing which pane is visible never reuses an input entity.
pub(in crate::ui::window::graph::node_controls) fn sync(
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let mut active = BTreeSet::new();
    let mut rows = Vec::new();
    let mut resets = BTreeMap::new();
    let mut fields = Vec::new();
    let mut racks = BTreeSet::new();
    for owner in &graph.nodes {
        let Some(settings) = owner.builtin.rack() else {
            continue;
        };
        let Some(epoch) = graph.rack_epochs.get(&owner.id).copied() else {
            continue;
        };
        if graph.kind_of(owner.id) != Some(NodeKind::Rack) {
            continue;
        }
        racks.insert(owner.id);
        let Some(slot) = shell.node_controls.rack.selected_slot(owner.id, settings) else {
            continue;
        };
        let Some(stage) = resolve_stage(&graph, owner.id, epoch, slot) else {
            continue;
        };
        let projection = Projection::new(stage.owner, &stage.slot.processor);
        for surface in [Surface::Card, Surface::Inspector] {
            let visible = if let Some(projection) = &projection {
                projection.fields(surface)
            } else {
                let page = shell
                    .node_controls
                    .rack
                    .mapper_pages
                    .entry((surface, owner.id))
                    .or_default();
                let (clamped, fields) =
                    parameters::weight_fields(stage.inputs, stage.outputs, *page);
                *page = clamped;
                fields
            };
            for field in visible {
                let key = Key {
                    surface,
                    node: owner.id,
                    epoch,
                    slot,
                    field,
                };
                let value = match &projection {
                    Some(projection) => projection.value(field),
                    None => field_value(&stage, field),
                };
                let Some(value) = value else { continue };
                active.insert(key);
                fields.push((key, value));
                if let Some(projection) = &projection {
                    let Some(position) = projection.position(field) else {
                        continue;
                    };
                    resets.insert(key, projection.reset(field).unwrap_or(position));
                    rows.push((
                        key,
                        position,
                        Box::new(move |position, graph: &GraphSnapshot| {
                            let stage = resolve_stage(graph, key.node, key.epoch, key.slot)?;
                            let projection = Projection::new(stage.owner, &stage.slot.processor)?;
                            let value = projection.value_at_position(key.field, position)?;
                            edit_for(graph, key, value).map(AppRequest::EditGraph)
                        }) as SliderEdit,
                    ));
                }
            }
        }
    }
    commit_outgoing(shell, &active, cx);
    let state = &mut shell.node_controls.rack;
    state.selected.retain(|node, slot| {
        racks.contains(node)
            && graph
                .node(*node)
                .and_then(|node| node.builtin.rack())
                .is_some_and(|settings| {
                    settings.slots.iter().any(|candidate| candidate.id == *slot)
                })
    });
    state
        .mapper_pages
        .retain(|(_, node), _| racks.contains(node));
    state.inputs.retain(|key, input| {
        active.contains(key)
            || ((state.errors.contains_key(key)
                || dirty(input.read(cx).value().as_str(), state.baselines.get(key)))
                && resolve_stage(&graph, key.node, key.epoch, key.slot)
                    .and_then(|stage| field_value(&stage, key.field))
                    .is_some())
    });
    state
        .baselines
        .retain(|key, _| state.inputs.contains_key(key));
    state.errors.retain(|key, _| state.inputs.contains_key(key));
    state.sliders.retain(|key, _| active.contains(key));
    state.active = active;
    for (key, value) in fields {
        let text = format!("{value:.2}");
        state.baselines.entry(key).or_insert_with(|| text.clone());
        let input = state.inputs.entry(key).or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
            cx.subscribe(&input, move |shell, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    commit_input(shell, &input, key, cx);
                }
            })
            .detach();
            input
        });
        if !input.read(cx).focus_handle(cx).is_focused(window)
            && !state.errors.contains_key(&key)
            && !dirty(input.read(cx).value().as_str(), state.baselines.get(&key))
        {
            if input.read(cx).value().as_str() != text {
                input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            }
            state.baselines.insert(key, text);
        }
    }
    sync_slider_range(
        window,
        cx,
        &shell.state,
        &mut state.sliders,
        rows,
        [0., 1., 0.001],
        |key, ui, edit, slider| SliderBehavior {
            step: Rc::new(|value, direction| (value + direction as f32 * 0.001).clamp(0., 1.)),
            reset: resets.get(key).copied().unwrap_or(0.),
            commit: commit_closure(&ui, &edit, &slider),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::super::super::parameter::Parameter;
    use super::*;
    use moiren_engine::rack::{RackProcessor, RackSlot};

    fn fixture(
        settings: RackSettings,
        inputs: &[usize],
        outputs: &[usize],
    ) -> (crate::service::ServiceGraph, NodeId) {
        let mut service = crate::service::ServiceGraph::default();
        let mut snapshot = service.snapshot();
        let node = snapshot.graph.create_rack(inputs, outputs).unwrap();
        let mut owner = snapshot.nodes[1].clone();
        owner.id = node;
        owner.gain = 1.;
        owner.builtin = crate::service::BuiltinState::Rack(settings);
        snapshot.nodes.push(owner);
        service.replace(snapshot.graph, snapshot.nodes).unwrap();
        (service, node)
    }

    fn key(service: &crate::service::ServiceGraph, node: NodeId, slot: u32, field: Field) -> Key {
        Key {
            surface: Surface::Card,
            node,
            epoch: service.snapshot().rack_epochs[&node],
            slot,
            field,
        }
    }

    #[test]
    fn typed_intents_merge_different_fields_and_reordered_slots() {
        use crate::service::RackParameter;
        let settings = RackSettings {
            slots: vec![
                RackSlot {
                    id: 7,
                    bypass: false,
                    processor: RackProcessor::Eq(Default::default()),
                },
                RackSlot {
                    id: 3,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 1. },
                },
            ],
            ..Default::default()
        };
        let (mut service, node) = fixture(settings, &[2], &[2]);
        let frequency = key(&service, node, 7, Field::Scalar(Parameter::EqFrequency));
        let gain = Key {
            field: Field::Scalar(Parameter::EqGain),
            ..frequency
        };
        let snapshot = service.snapshot();
        let frequency_edit = edit_for(&snapshot, frequency, 2200.).unwrap();
        let gain_edit = edit_for(&snapshot, gain, -6.).unwrap();
        assert_eq!(
            gain_edit,
            GraphEdit::EditRack {
                node,
                epoch: gain.epoch,
                edit: RackEdit::SetParameter {
                    slot: 7,
                    parameter: RackParameter::EqGain,
                    value: -6.
                }
            }
        );
        service.edit(frequency_edit).unwrap();
        service
            .edit(GraphEdit::EditRack {
                node,
                epoch: gain.epoch,
                edit: RackEdit::Move {
                    slot: 7,
                    direction: 1,
                },
            })
            .unwrap();
        service.edit(gain_edit).unwrap();
        let snapshot = service.snapshot();
        let rack = snapshot.node(node).unwrap().builtin.rack().unwrap();
        assert_eq!(rack.slots[0].id, 3);
        assert_eq!(rack.slots[0].processor, RackProcessor::Gain { initial: 1. });
        let RackProcessor::Eq(eq) = &rack.slots[1].processor else {
            panic!("EQ")
        };
        assert_eq!(eq.frequency_hz, 2200.);
        assert_eq!(eq.gain_db, -6.);
        assert_eq!(eq.q, 1.);
        assert!(edit_for(&snapshot, gain, -3.).is_some());
        assert!(
            edit_for(
                &snapshot,
                Key {
                    field: Field::Scalar(Parameter::Level),
                    ..gain
                },
                0.
            )
            .is_none()
        );
    }

    #[test]
    fn replaced_rack_and_reused_slot_id_cannot_receive_old_editor_events() {
        let (mut service, node) = fixture(settings(), &[2], &[2]);
        let key = key(&service, node, 7, Field::Scalar(Parameter::Level));
        let stale = edit_for(&service.snapshot(), key, -6.).unwrap();
        service
            .edit(GraphEdit::SetBuiltin {
                node,
                state: crate::service::BuiltinState::Rack(settings()),
            })
            .unwrap();
        assert!(edit_for(&service.snapshot(), key, -6.).is_none());
        assert!(service.edit(stale).is_err());
        service.edit(GraphEdit::RemoveNode(node)).unwrap();
        assert!(edit_for(&service.snapshot(), key, -6.).is_none());
    }

    #[test]
    fn bypassed_mapper_edits_its_configured_rows_not_effective_route_width() {
        use moiren_engine::processor::ChannelMapperSettings;
        let (mut service, node) = fixture(
            RackSettings {
                slots: vec![RackSlot {
                    id: 9,
                    bypass: true,
                    processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                        mapping: vec![0, 1, 0],
                        matrix: Vec::new(),
                    }),
                }],
                ..Default::default()
            },
            &[2],
            &[2],
        );
        let key = key(
            &service,
            node,
            9,
            Field::Weight {
                input: 1,
                output: 2,
            },
        );
        let snapshot = service.snapshot();
        let stage = resolve_stage(&snapshot, node, key.epoch, 9).unwrap();
        assert_eq!((stage.inputs, stage.outputs), (2, 3));
        service
            .edit(edit_for(&snapshot, key, -2.5).unwrap())
            .unwrap();
        let snapshot = service.snapshot();
        let slot = &snapshot.node(node).unwrap().builtin.rack().unwrap().slots[0];
        assert!(slot.bypass);
        let RackProcessor::ChannelMapper(mapper) = &slot.processor else {
            panic!("Mapper")
        };
        assert_eq!(
            mapper.matrix,
            vec![vec![1., 0.], vec![0., 1.], vec![1., -2.5]]
        );
        assert!(
            edit_for(
                &snapshot,
                Key {
                    field: Field::Weight {
                        input: 2,
                        output: 2
                    },
                    ..key
                },
                1.
            )
            .is_none()
        );
        assert!(edit_for(&snapshot, key, f64::INFINITY).is_none());
    }

    fn node() -> NodeId {
        crate::service::ServiceGraph::default().snapshot().nodes[0].id
    }

    fn settings() -> RackSettings {
        RackSettings {
            slots: [7, 3]
                .map(|id| RackSlot {
                    id,
                    bypass: false,
                    processor: RackProcessor::Gain { initial: 1. },
                })
                .to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn selection_uses_stable_identity_and_falls_back_in_vector_order() {
        let node = node();
        let mut state = RackEditorState::default();
        let mut settings = settings();
        assert_eq!(state.selected_slot(node, &settings), Some(7));
        state.selected.insert(node, 3);
        settings.slots.swap(0, 1);
        assert_eq!(state.selected_slot(node, &settings), Some(3));
        settings.slots.remove(0);
        assert_eq!(state.selected_slot(node, &settings), Some(7));
        settings.slots.clear();
        assert_eq!(state.selected_slot(node, &settings), None);
    }

    #[test]
    fn surface_epoch_slot_and_field_are_distinct_editor_identities() {
        let key = Key {
            surface: Surface::Card,
            node: node(),
            epoch: 4,
            slot: 7,
            field: Field::Scalar(Parameter::Level),
        };
        let keys = std::collections::BTreeSet::from([
            key,
            Key {
                surface: Surface::Inspector,
                ..key
            },
            Key { epoch: 5, ..key },
            Key { slot: 3, ..key },
            Key {
                field: Field::Weight {
                    input: 0,
                    output: 0,
                },
                ..key
            },
        ]);
        assert_eq!(keys.len(), 5);
    }

    #[test]
    fn unchanged_numeric_draft_does_not_become_dirty_when_snapshot_changes() {
        let mut baseline = String::from("0.00");
        assert!(!dirty("0.00", Some(&baseline)));
        assert!(dirty("3.125", Some(&baseline)));
        baseline = String::from("3.125");
        assert!(!dirty("3.125", Some(&baseline)));
        assert!(dirty("invalid", Some(&baseline)));
    }

    #[cfg(feature = "gpui-test")]
    #[gpui_kit::test]
    fn rack_inputs_are_surface_local_and_dirty_blur_never_replays_knob_value(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::ui::window::test_harness::{finish_service, test_shell, wait_until};
        use std::time::Duration;
        let (view, cx, runtime) = test_shell(cx);
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: NodeKind::Rack,
                    channels: 2,
                }))
            });
        });
        wait_until(Duration::from_secs(3), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node_of_kind(NodeKind::Rack)
                    .is_some()
            })
        });
        let node = view.update(cx, |shell, cx| {
            let node = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node_of_kind(NodeKind::Rack)
                .unwrap()
                .0
                .id();
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                    node,
                    state: crate::service::BuiltinState::Rack(RackSettings {
                        slots: vec![RackSlot {
                            id: 7,
                            bypass: false,
                            processor: RackProcessor::Eq(Default::default()),
                        }],
                        ..Default::default()
                    }),
                }))
            });
            node
        });
        let flush = |cx: &mut gpui_kit::VisualTestContext| {
            wait_until(Duration::from_secs(3), || {
                view.update(cx, |shell, cx| {
                    shell.state.update(cx, |state, _| {
                        state.poll();
                        !state.pending_request
                    })
                })
            })
        };
        flush(cx);
        let (key, card, inspector) = cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                sync(shell, window, cx);
                let key = Key {
                    surface: Surface::Card,
                    node,
                    epoch: shell.state.read(cx).snapshot.graph.rack_epochs[&node],
                    slot: 7,
                    field: Field::Scalar(Parameter::EqGain),
                };
                let card = shell.node_controls.rack.inputs[&key].clone();
                let inspector = shell.node_controls.rack.inputs[&Key {
                    surface: Surface::Inspector,
                    ..key
                }]
                    .clone();
                assert_ne!(card.entity_id(), inspector.entity_id());
                assert!(
                    shell
                        .node_controls
                        .rack
                        .owns_focus(&card.read(cx).focus_handle(cx), cx)
                );
                assert!(
                    shell
                        .node_controls
                        .rack
                        .owns_focus(&inspector.read(cx).focus_handle(cx), cx)
                );
                window.focus(&card.read(cx).focus_handle(cx), cx);
                (key, card, inspector)
            })
        });
        view.update(cx, |shell, cx| {
            let edit = edit_for(&shell.state.read(cx).snapshot.graph, key, 6.25).unwrap();
            shell
                .state
                .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
        });
        flush(cx);
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                sync(shell, window, cx);
                assert_eq!(card.read(cx).value().as_str(), "0.00");
                assert_eq!(inspector.read(cx).value().as_str(), "6.25");
                card.update(cx, |_, cx| cx.emit(InputEvent::Blur));
            })
        });
        flush(cx);
        view.update(cx, |shell, cx| {
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(node)
                    .unwrap()
                    .builtin
                    .rack()
                    .unwrap()
                    .slots[0]
                    .processor,
                RackProcessor::Eq(moiren_engine::processor::EqSettings {
                    gain_db: 6.25,
                    ..Default::default()
                })
            );
        });
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                card.update(cx, |input, cx| input.set_value("not a number", window, cx));
                commit_input(shell, &card, key, cx);
                window.blur(cx);
                sync(shell, window, cx);
                assert_eq!(card.read(cx).value().as_str(), "not a number");
                assert!(shell.node_controls.rack.errors.contains_key(&key));
                assert_eq!(inspector.read(cx).value().as_str(), "6.25");
                card.update(cx, |input, cx| input.set_value("3.125", window, cx));
                sync(shell, window, cx);
                assert_eq!(card.read(cx).value().as_str(), "3.125");
                commit_input(shell, &card, key, cx);
                assert!(!shell.node_controls.rack.errors.contains_key(&key));
                assert_eq!(shell.node_controls.rack.baselines[&key], "3.125");
            })
        });
        flush(cx);
        view.update(cx, |shell, cx| {
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(node)
                    .unwrap()
                    .builtin
                    .rack()
                    .unwrap()
                    .slots[0]
                    .processor,
                RackProcessor::Eq(moiren_engine::processor::EqSettings {
                    gain_db: 3.125,
                    ..Default::default()
                })
            );
        });
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                let mut snapshot = (*shell.state.read(cx).snapshot).clone();
                std::sync::Arc::make_mut(&mut snapshot.graph)
                    .rack_epochs
                    .insert(node, key.epoch + 1);
                shell.state.update(cx, |state, _| {
                    state.snapshot = std::sync::Arc::new(snapshot)
                });
                sync(shell, window, cx);
                assert!(!shell.node_controls.rack.inputs.contains_key(&key));
                assert!(!shell.node_controls.rack.baselines.contains_key(&key));
                assert!(!shell.node_controls.rack.sliders.contains_key(&key));
                assert_ne!(
                    shell.node_controls.rack.inputs[&Key {
                        epoch: key.epoch + 1,
                        ..key
                    }]
                        .entity_id(),
                    card.entity_id()
                );
            })
        });
        finish_service(runtime);
    }
}
