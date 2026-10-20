use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportResourceState {
    Unavailable,
    Absent,
    Creating,
    Present,
    Removing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportObservation {
    pub intent: ExportIntent,
    /// Monotonic control-adapter observation sequence, including after removal.
    pub generation: u64,
    pub resource: ExportResourceState,
    pub allow_default: bool,
    pub gate_open: bool,
    /// Reported only after Engine confirms the graph that supplies this resource.
    pub active_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportAction {
    Create {
        intent: ExportIntent,
        allow_default: bool,
    },
    Remove {
        endpoint_id: VirtualEndpointId,
    },
    SetDefaultEligibility {
        endpoint_id: VirtualEndpointId,
        allow_default: bool,
    },
    SetGate {
        endpoint_id: VirtualEndpointId,
        open: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReconcilePlan {
    pub candidate_revision: u64,
    pub available: bool,
    pub actions: Vec<ExportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportStatus {
    pub intent: ExportIntent,
    pub resource: ExportResourceState,
    pub allow_default: bool,
    pub gate_open: bool,
    pub active_revision: Option<u64>,
}

pub(super) fn same_resource(a: &ExportIntent, b: &ExportIntent) -> bool {
    a.endpoint_id == b.endpoint_id
        && a.direction == b.direction
        && a.name == b.name
        && a.channels == b.channels
}

fn confirms_active(observation: &ExportObservation, graph: &super::super::GraphSnapshot) -> bool {
    observation.resource == ExportResourceState::Present
        && observation.intent.enabled
        && observation.intent.binding.is_some()
        && observation.active_revision.is_some()
        && observation.active_revision == graph.active_revision
}

impl ExportService {
    /// A plan is desired work, not evidence that an operation has started.
    /// Adapters must recheck its revision before dispatch and publish actual
    /// creating/present/removing observations independently of gate/audio state.
    pub fn reconcile(&self, graph: &super::super::GraphSnapshot) -> ExportReconcilePlan {
        let mut plan = ExportReconcilePlan {
            candidate_revision: graph.candidate_revision,
            available: self.available,
            actions: Vec::new(),
        };
        if !self.available {
            return plan;
        }
        for observation in self.observations.values() {
            let id = observation.intent.endpoint_id;
            let desired = graph
                .exports
                .iter()
                .find(|intent| intent.endpoint_id == id && intent.enabled);
            let retained = desired.is_some_and(|intent| same_resource(intent, &observation.intent));
            let active = confirms_active(observation, graph);
            let allow_default = self.allow_default(id);
            let rebuild = retained
                && matches!(
                    observation.resource,
                    ExportResourceState::Creating | ExportResourceState::Present
                )
                && allow_default != observation.allow_default;
            if (!retained || rebuild)
                && !active
                && matches!(
                    observation.resource,
                    ExportResourceState::Creating | ExportResourceState::Present
                )
            {
                plan.actions.push(if observation.gate_open {
                    ExportAction::SetGate {
                        endpoint_id: id,
                        open: false,
                    }
                } else {
                    ExportAction::Remove { endpoint_id: id }
                });
            } else if retained && observation.resource == ExportResourceState::Present {
                // Admission belongs to the binding confirmed by the engine.
                // An unapplied candidate may already contain a different one.
                if observation.gate_open && (!active || rebuild) {
                    plan.actions.push(ExportAction::SetGate {
                        endpoint_id: id,
                        open: false,
                    });
                } else if !observation.gate_open && active && !rebuild {
                    plan.actions.push(ExportAction::SetGate {
                        endpoint_id: id,
                        open: true,
                    });
                }
            }
        }
        for intent in graph.exports.iter().filter(|intent| intent.enabled) {
            if self
                .observations
                .get(&intent.endpoint_id)
                .is_none_or(|observation| observation.resource == ExportResourceState::Absent)
            {
                plan.actions.push(ExportAction::Create {
                    intent: intent.clone(),
                    allow_default: self.allow_default(intent.endpoint_id),
                });
            }
        }
        plan
    }

    pub fn statuses(&self, graph: &super::super::GraphSnapshot) -> Vec<ExportStatus> {
        let mut intents: BTreeMap<_, _> = self
            .observations
            .iter()
            .filter(|(_, observation)| observation.resource != ExportResourceState::Absent)
            .map(|(&id, observation)| (id, observation.intent.clone()))
            .collect();
        intents.extend(
            graph
                .exports
                .iter()
                .map(|intent| (intent.endpoint_id, intent.clone())),
        );
        intents
            .into_values()
            .map(|intent| {
                let observed = self.observations.get(&intent.endpoint_id);
                ExportStatus {
                    resource: if self.available {
                        observed.map_or(ExportResourceState::Absent, |value| value.resource)
                    } else {
                        ExportResourceState::Unavailable
                    },
                    allow_default: self.allow_default(intent.endpoint_id),
                    gate_open: self.available && observed.is_some_and(|value| value.gate_open),
                    active_revision: observed
                        .filter(|value| self.available && confirms_active(value, graph))
                        .and_then(|value| value.active_revision),
                    intent,
                }
            })
            .collect()
    }
}
