//! Request dispatch and bounded projection of host work.
use super::*;

impl ServiceDriver for ManagedDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        match request {
            AppRequest::SetLatencyProfile(profile) => c.latency_profile = profile,
            AppRequest::RefreshCatalog => self.refresh = true,
            AppRequest::Start(spec) => {
                self.automatic_startup = false;
                if let Err(error) = c.graph.configure_session(&spec) {
                    c.graph.record_error(error);
                } else if c.core.managed_desired(spec).is_ok() {
                    self.apply = true;
                }
            }
            AppRequest::ApplyGraph => {
                self.automatic_startup = false;
                self.apply = true;
            }
            AppRequest::Stop => {
                self.automatic_startup = false;
                self.apply = false;
                let _ = c.core.stop();
                self.retire(c);
            }
            AppRequest::ReplaceGraph { graph, nodes } => {
                if c.graph.replace(graph, nodes).is_ok() {
                    self.document = self
                        .document
                        .checked_add(1)
                        .expect("document identity exhausted");
                    self.clear_parameters(c);
                    self.edits.clear();
                    c.graph.clear_pending();
                }
            }
            AppRequest::EditGraph(edit) => {
                let values = Self::parameter_edit(&edit);
                if c.graph.edit(edit).is_ok() {
                    for (target, value) in values {
                        self.submit_parameter(c, target, value);
                    }
                }
            }
            AppRequest::SetGainPan { gain, pan } => {
                if model::valid_gain_pan(gain, pan)
                    && let Some((gain_node, pan_node)) = c.graph.preparation().gain_pan_nodes()
                {
                    for edit in [
                        GraphEdit::SetGain {
                            node: gain_node,
                            gain,
                        },
                        GraphEdit::SetPan {
                            node: pan_node,
                            pan,
                        },
                    ] {
                        let values = Self::parameter_edit(&edit);
                        if c.graph.edit(edit).is_ok() {
                            for (target, value) in values {
                                self.submit_parameter(c, target, value);
                            }
                        }
                    }
                    c.core.update_gain_pan(gain, pan);
                }
            }
        }
    }
    fn poll(&mut self, c: &mut ServiceContext, pool: &mut JobPool, budget: RoundBudget) {
        for _ in 0..budget.completions {
            let Some(completion) = self.queue().try_recv() else {
                break;
            };
            match completion {
                Completion::Reaped(generation) => {
                    self.cleaning -= 1;
                    c.core.owner_reaped(generation);
                }
                Completion::Catalog(result) => {
                    self.catalog_running = false;
                    match result {
                        Ok(catalog) => {
                            c.catalog = Arc::new(catalog);
                            c.catalog_error = None;
                            if self.automatic_startup
                                && let Some(spec) = startup::default_session(&c.catalog)
                            {
                                self.automatic_startup = false;
                                let _ = c.graph.configure_session(&spec);
                                self.apply = true;
                            }
                        }
                        Err(error) => c.catalog_error = Some(error.to_string()),
                    }
                }
            }
        }
        if self.refresh
            && !self.catalog_running
            && !self.exiting
            && let Some(backend) = self.catalog_backend.clone()
            && let Ok(permit) = self.queue().port().try_reserve()
            && pool
                .try_submit(
                    JobPriority::Prepare,
                    Box::new(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            backend.catalog()
                        }))
                        .unwrap_or_else(|_| Err(BackendError::new("catalog", "backend panicked")));
                        permit.send_or_dispose(Completion::Catalog(result), drop);
                    }),
                )
                .is_ok()
        {
            self.catalog_running = true;
            self.refresh = false;
        }
        if self.apply {
            self.apply(c);
        }
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
        let snapshot = owner.observer.read();
        #[cfg(windows)]
        {
            c.native_observations = self
                .observations
                .as_ref()
                .and_then(|handle| {
                    handle
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .as_ref()
                        .and_then(|value| value.snapshot())
                })
                .filter(|observation| {
                    observation.running
                        && observation.owner_generation == snapshot.state.owner_generation
                        && snapshot.state.execution == ExecutionState::Running
                })
                .map(|observation| NativeSessionObservations {
                    generation: owner.generation,
                    observation,
                });
        }
        let running = snapshot.state.execution == ExecutionState::Running;
        let error = if snapshot.state.execution == ExecutionState::WaitingForResources {
            match &snapshot.state.update {
                UpdateState::Blocked { reason, .. } => Some(format!("{reason:?}")),
                _ => snapshot
                    .state
                    .fault
                    .as_ref()
                    .map(|error| format!("{error:?}")),
            }
        } else {
            None
        };
        c.core.managed_execution(owner.generation, running, error);
        if running {
            c.active_latency_profile = Some(owner.profile);
            if owner.started.is_none() {
                owner.started = Some(Instant::now());
            }
            // An old confirmation still describes valid playback, but must not
            // finish a new Apply of that same revision. Its own ACK barrier below
            // clears pending; periodic projection only restores settled state.
            if snapshot.state.active_revision == Some(owner.active_revision.0)
                && (owner.active_apply_id == owner.latest_apply_id
                    || c.graph.snapshot().pending_revision != Some(owner.active_revision.1))
            {
                c.graph.confirmed(owner.generation, owner.active_revision.1);
            }
        } else {
            c.active_latency_profile = None;
            c.graph.stopped();
        }
        if let UpdateState::Blocked { reason, .. } = &snapshot.state.update {
            c.graph
                .preparation_failed(owner.generation, format!("{reason:?}"));
        }
        if snapshot.state.execution == ExecutionState::WaitingForResources {
            c.graph
                .preparation_failed(owner.generation, format!("{:?}", snapshot.state.fault));
        }
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
                    c.graph.preparation_failed(owner.generation, error);
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
                && (values.is_empty() || c.graph.snapshot().candidate_revision == revision)
            {
                self.confirm_parameters = Some(ApplyConfirmation {
                    id: apply_id,
                    revision,
                    host_revision,
                });
                self.confirmation_values = values.into();
            } else if c.graph.snapshot().pending_revision == Some(revision) {
                c.graph.clear_pending();
            }
        }
        let mut parameter_failed = false;
        self.parameters
            .retain(|pending| match pending.receipt.outcome() {
                ParameterOutcome::Applied(reply) => {
                    c.control.last_result = Some(reply);
                    false
                }
                ParameterOutcome::Rejected(error) => {
                    c.graph.record_error(format!("实时参数被拒绝: {error:?}"));
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
        if !self.parameters.iter().any(|pending| {
            pending.confirmation.is_some() && pending.confirmation == self.confirm_parameters
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
            if running && snapshot.state.active_revision == Some(host_revision) {
                c.graph.confirmed(owner.generation, revision);
            }
        }
        if let Some(meters) = &mut self.meters {
            meters.poll(c, &owner.observer);
        }
        if let Some(spec) = c.core.desired()
            && let RunLimit::Seconds(seconds) = spec.limit
            && owner
                .started
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(u64::from(seconds)))
        {
            let _ = c.core.stop();
            self.retire(c);
            self.apply = false;
        }
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        self.apply = false;
        self.retire(c);
    }
    fn is_drained(&self) -> bool {
        self.owner.is_none()
            && self.cleaning == 0
            && !self.catalog_running
            && self
                .completions
                .as_ref()
                .is_none_or(|q| q.outstanding() == 0)
    }
}
