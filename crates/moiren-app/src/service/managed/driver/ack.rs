//! The Apply-ACK barrier: how an Apply becomes the active revision.
//!
//! Audit 06, observation 9 named this the hardest-to-maintain ~150 lines of
//! the service layer. Every Apply must collect its own matching parameter
//! ACKs before it may confirm a revision; a same-structure edit cannot be
//! identified by sampling the observer before enqueue, and older queued
//! commands may advance that basis before acceptance. Moved here verbatim —
//! do not change any judgment order or dimension while refactoring.
use super::super::*;

impl ManagedDriver {
    /// Retire finished host edits and establish the parameter-ACK barrier for
    /// newly activated applies.
    pub(super) fn poll_edits(&mut self, c: &mut ServiceContext) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
        let mut confirmed = Vec::new();
        self.edits.retain(|edit| match edit.receipt.outcome() {
            EditOutcome::Activated { graph_revision, .. } => {
                confirmed.push((
                    edit.apply_id,
                    edit.revision,
                    graph_revision,
                    edit.values.clone(),
                ));
                false
            }
            EditOutcome::Rejected(error) => {
                if edit.apply_id == owner.latest_apply_id {
                    let fault = project_fault(&error);
                    c.graph
                        .preparation_rejected(owner.generation, fault, error.to_string());
                }
                false
            }
            EditOutcome::Closed | EditOutcome::Conflict { .. } | EditOutcome::Superseded { .. } => {
                false
            }
            _ => true,
        });
        for (apply_id, revision, host_revision, values) in confirmed {
            // A parameter-only edit may activate before an earlier topology
            // preparation. Its later completion must neither replace this
            // Apply's ACK barrier nor regress an already confirmed revision.
            if apply_id != owner.latest_apply_id
                || revision < owner.latest_apply_revision
                || revision < owner.active_revision.1
            {
                continue;
            }
            // Every Apply requires its own matching parameter ACKs. Sampling
            // the observer before enqueue cannot identify a same-structure edit:
            // older queued commands may advance that basis before acceptance.
            if owner.document == self.document
                && (values.is_empty() || c.graph.candidate_revision() == revision)
            {
                self.confirm_parameters = Some(ApplyConfirmation {
                    id: apply_id,
                    revision,
                    host_revision,
                });
                self.confirmation_values = values.into();
            } else if c.graph.pending_revision() == Some(revision) {
                c.graph.clear_pending();
            }
        }
    }
    /// Retire finished parameter submissions; a failed or closed confirmation
    /// tears down the current ACK barrier.
    pub(super) fn poll_parameters(&mut self, c: &mut ServiceContext) {
        let mut parameter_failed = false;
        self.parameters
            .retain(|pending| match pending.receipt.outcome() {
                ParameterOutcome::Applied(reply) => {
                    c.control.last_result = Some(reply);
                    false
                }
                ParameterOutcome::Rejected(error) => {
                    // A target removed by a newer candidate is an expected
                    // consequence of editing, not a failure to report.
                    if error != host::ParameterFailure::TargetRemoved {
                        c.graph
                            .record_error(format!("Live parameters rejected: {error:?}"));
                    }
                    parameter_failed |= pending.confirmation.is_some()
                        && pending.confirmation == self.confirm_parameters;
                    false
                }
                ParameterOutcome::Closed => {
                    parameter_failed |= pending.confirmation.is_some()
                        && pending.confirmation == self.confirm_parameters;
                    false
                }
                ParameterOutcome::Superseded => {
                    parameter_failed |= pending.confirmation.is_some()
                        && pending.confirmation == self.confirm_parameters;
                    false
                }
                _ => true,
            });
        if parameter_failed {
            self.confirm_parameters = None;
            self.confirmation_values.clear();
            c.graph.clear_pending();
        }
    }
    /// Submit pending parameter values to the running owner within the round
    /// budget, then publish queue-depth counters.
    pub(super) fn pump_parameters(&mut self, c: &mut ServiceContext, budget: RoundBudget) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
        for _ in 0..budget.applied.min(32) {
            if self.parameters.len() == 64 {
                break;
            }
            let live = !self.unsent.is_empty();
            let Some(&(target, value)) = self
                .unsent
                .first()
                .or_else(|| self.confirmation_values.front())
            else {
                break;
            };
            match owner
                .control
                .parameter(target, value, ParameterTiming::NextBlock, 0)
            {
                Ok(receipt) => {
                    if live {
                        self.unsent.remove(0);
                    } else {
                        self.confirmation_values.pop_front();
                    }
                    self.parameters.push(PendingParameter {
                        target,
                        receipt,
                        confirmation: if live { None } else { self.confirm_parameters },
                    });
                }
                Err(host::HostError::Capacity) => break,
                Err(error) => {
                    c.graph.record_error(error);
                    if live {
                        self.unsent.remove(0);
                    } else {
                        self.confirmation_values.pop_front();
                    }
                    self.confirm_parameters = None;
                }
            }
        }
        c.control.pending_node_parameters = self.unsent.len();
        c.control.accepted_pending = self.parameters.len();
    }
    /// Resolve the parameter-ACK barrier once every confirmation value has been
    /// accepted; this is the point an Apply becomes the active revision.
    pub(super) fn finish_apply_confirmation(
        &mut self,
        c: &mut ServiceContext,
        snapshot: &host::ProjectSnapshot,
    ) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
        if !self.parameters.iter().any(|pending| {
            pending.confirmation.is_some()
                && pending.confirmation == self.confirm_parameters
                // A value whose target exists only in a pending topology is
                // parked until that plan activates; its own Activated edit
                // reconciles it, so it cannot block this Apply's barrier.
                && snapshot
                    .active
                    .as_ref()
                    .is_none_or(|active| active.target_exists(pending.target))
        }) && self.confirmation_values.is_empty()
            && let Some(ApplyConfirmation {
                id,
                revision,
                host_revision,
            }) = self.confirm_parameters.take()
            && id == owner.latest_apply_id
            && revision >= owner.latest_apply_revision
            && revision >= owner.active_revision.1
        {
            owner.active_revision = (host_revision, revision);
            owner.active_apply_id = id;
            if snapshot.state.execution == ExecutionState::Running
                && snapshot.state.active_revision == Some(host_revision)
            {
                c.graph.confirmed(owner.generation, revision);
            }
        }
    }
}

/// Map an engine project rejection to a located, user-language fault.
fn project_fault(error: &moiren_engine::host::ProjectError) -> super::GraphFault {
    use moiren_engine::host::ProjectError as E;
    let (subject, cause, hint) = match error {
        E::MissingConfiguration(node) => (
            super::GraphSubject::Node(*node),
            "This channel is missing a usable processor configuration".to_string(),
            Some("Delete this channel and re-add one of the same type"),
        ),
        E::ConfigurationKind(node) => (
            super::GraphSubject::Node(*node),
            "The channel's configuration does not match its kind".to_string(),
            Some("Check the channel's parameter settings, or delete and re-add it"),
        ),
        E::UnknownConfiguration(node) => (
            super::GraphSubject::Node(*node),
            "The configuration points at a channel that no longer exists".to_string(),
            None,
        ),
        E::IoChannels(node) => (
            super::GraphSubject::Node(*node),
            "The channel's channel count disagrees with its wiring".to_string(),
            Some("Adjust the channel count, or reconnect the route"),
        ),
        E::Io { node, .. } => (
            super::GraphSubject::Node(*node),
            "The input/output device configuration is invalid".to_string(),
            Some("Choose another device, or check the sample rate/precision settings"),
        ),
        E::InvalidProcessor(node) => (
            super::GraphSubject::Node(*node),
            "Channel parameters are outside their valid range".to_string(),
            None,
        ),
        E::InvalidParameter => (
            super::GraphSubject::Graph,
            "Parameters are outside their valid range".to_string(),
            None,
        ),
        E::Graph(_) | E::InvalidProcessing | E::Capacity => {
            (super::GraphSubject::Graph, error.to_string(), None)
        }
    };
    super::GraphFault {
        subject,
        cause,
        hint: hint.map(str::to_string),
    }
}
