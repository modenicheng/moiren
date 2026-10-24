//! Request dispatch and bounded projection of host work.
use super::*;

impl ServiceDriver for ManagedDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        match request {
            AppRequest::SetExportAuthorizations(values) => {
                let previous = c.exports.authorizations.clone();
                if let Err(error) = c.exports.set_authorizations(&values) {
                    c.graph.record_error(error);
                } else if c.exports.authorizations != previous {
                    self.exports_runtime.revoke();
                    // Eligibility is fixed before native exposure. Drain the
                    // owner before the lifecycle worker replaces its resource;
                    // resuming requires a fresh Apply and Engine confirmation.
                    if self.owner.as_ref().is_some_and(|owner| {
                        owner.exports.iter().any(|intent| {
                            intent.enabled
                                && previous.get(&intent.endpoint_id).copied().unwrap_or(false)
                                    != c.exports.allow_default(intent.endpoint_id)
                        })
                    }) {
                        self.apply = false;
                        self.apply_audio_settings = None;
                        let _ = c.core.stop();
                        self.retire(c);
                    }
                }
            }
            AppRequest::LoadProject(document) => match c.load_project(document) {
                Ok(()) => {
                    self.exports_runtime.revoke();
                    self.automatic_startup = false;
                    self.document = self
                        .document
                        .checked_add(1)
                        .expect("document identity exhausted");
                    self.clear_parameters(c);
                    self.edits.clear();
                    c.graph.clear_pending();
                    self.apply = false;
                    self.apply_audio_settings = None;
                }
                Err(error) => c.graph.record_error(error),
            },
            AppRequest::SetLatencyProfile(profile) => c.latency_profile = profile,
            AppRequest::SetAudioSettings(settings) => {
                c.set_audio_settings(settings);
            }
            AppRequest::RefreshCatalog => self.catalog.request(),
            AppRequest::Start(spec) => {
                self.automatic_startup = false;
                let mut settings = c.audio_settings.clone();
                let Ok(block_frames) = u32::try_from(spec.max_block_frames) else {
                    c.graph
                        .record_error("processing block size is out of range");
                    return;
                };
                settings.processing.block_frames = block_frames;
                if let Err(error) = settings.validate() {
                    c.graph.record_error(error);
                    return;
                }
                if let Err(error) = c.graph.configure_session(&spec) {
                    c.graph.record_error(error);
                } else if c.core.managed_desired(spec).is_ok() {
                    c.audio_settings = settings;
                    self.apply_audio_settings = Some(c.audio_settings.clone());
                    self.apply = true;
                }
            }
            AppRequest::ApplyGraph => {
                self.automatic_startup = false;
                self.apply_audio_settings = Some(c.audio_settings.clone());
                self.apply = true;
            }
            AppRequest::Stop => {
                self.automatic_startup = false;
                self.apply = false;
                self.apply_audio_settings = None;
                let _ = c.core.stop();
                self.retire(c);
            }
            AppRequest::ReplaceGraph { graph, nodes } => {
                if c.graph.replace(graph, nodes).is_ok() {
                    self.exports_runtime.revoke();
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
                let values = graph::edit_parameter_values(&edit);
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
                        let values = graph::edit_parameter_values(&edit);
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
        self.poll_completions(c, budget);
        self.poll_catalog(pool);
        if self.apply {
            self.apply(c);
        }
        if let Some(owner) = self.owner.as_ref() {
            let snapshot = owner.observer.read();
            self.poll_observations(c, &snapshot, owner.generation);
            self.poll_execution(c, &snapshot);
            self.poll_edits(c);
            self.poll_parameters(c);
            self.pump_parameters(c, budget);
            self.finish_apply_confirmation(c, &snapshot);
            self.poll_meters(c);
            self.poll_run_limit(c);
        }
        self.poll_exports(c, pool);
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        self.apply = false;
        self.apply_audio_settings = None;
        self.retire(c);
    }
    fn is_drained(&self) -> bool {
        self.owner.is_none()
            && self.cleaning == 0
            && !self.catalog.is_running()
            && self.exports_runtime.is_drained()
            && self
                .completions
                .as_ref()
                .is_none_or(|q| q.outstanding() == 0)
    }
}

impl ManagedDriver {
    /// Drain owner-reaping and catalog-refresh completions from the cleanup
    /// worker.
    fn poll_completions(&mut self, c: &mut ServiceContext, budget: RoundBudget) {
        for _ in 0..budget.completions {
            let Some(completion) = self.queue().try_recv() else {
                break;
            };
            match completion {
                Completion::Exports(result) => {
                    for error in self.exports_runtime.complete(&mut c.exports, result) {
                        c.graph.record_error(error);
                    }
                }
                Completion::Reaped(generation) => {
                    self.cleaning -= 1;
                    log::info!(
                        target: "moiren::audio::service",
                        "audio owner reaped: generation={generation:?} cleaning={}",
                        self.cleaning,
                    );
                    c.core.owner_reaped(generation);
                }
                Completion::Catalog(result) => {
                    self.catalog.finished();
                    match result {
                        Ok(catalog) => {
                            c.catalog = Arc::new(catalog);
                            c.catalog_error = None;
                            if self.automatic_startup {
                                self.automatic_startup = false;
                                if let Some(spec) = startup::default_session_for_backend(
                                    &c.catalog,
                                    c.audio_settings.backend,
                                ) {
                                    let _ = c.graph.configure_session(&spec);
                                    self.apply_audio_settings = Some(c.audio_settings.clone());
                                    self.apply = true;
                                }
                                // Known divergence: managed automatic startup
                                // does not retry after an unusable catalog;
                                // the legacy driver retries every second.
                            }
                        }
                        Err(error) => {
                            log::error!(
                                target: "moiren::audio::catalog",
                                "audio device catalog refresh failed: {error}"
                            );
                            c.catalog_error = Some(error.to_string());
                        }
                    }
                }
            }
        }
    }
    fn poll_exports(&mut self, c: &mut ServiceContext, pool: &mut JobPool) {
        let mut graph = c.graph.snapshot();
        let active = self
            .owner
            .as_ref()
            .map_or_else(Vec::new, |o| o.exports.clone());
        let basis = self.owner.as_ref().and_then(|owner| {
            let host = owner.observer.read();
            (host.state.execution == ExecutionState::Running
                && host.state.active_revision == Some(owner.active_revision.0)
                && graph.active_revision == Some(owner.active_revision.1)
                && graph.pending_revision.is_none()
                && !self.apply
                && owner.document == self.document
                && !self.exiting)
                .then_some([
                    owner.generation.0,
                    host.state.owner_generation,
                    owner.active_revision.0,
                    owner.active_revision.1,
                    owner.active_apply_id,
                ])
        });
        self.exports_runtime
            .project(&mut c.exports, &graph, &active, basis);
        if self.exiting {
            graph.exports.clear();
        } else {
            // Unapplied candidate changes cannot replace a resource still used
            // by the current Engine owner. Retirement also waits for its reap.
            for intent in &active {
                graph
                    .exports
                    .retain(|i| i.endpoint_id != intent.endpoint_id);
                graph.exports.push(intent.clone());
            }
        }
        // Reconciliation retains every export leased by the current owner.
        // Once old owners are reaped, unrelated endpoints may retire while a
        // replacement owner runs. The provider also checks each PCM drain.
        let drained = self.cleaning == 0;
        let work = if self.exiting {
            self.exports_runtime
                .shutdown_work(drained && self.owner.is_none())
        } else {
            self.exports_runtime.work(&c.exports, &graph, drained)
        };
        if let Some(work) = work
            && let Ok(completion) = self.queue().port().try_reserve()
        {
            self.exports_runtime
                .submit(pool, completion, work, Completion::Exports);
        }
    }
    /// Kick a catalog refresh when the refresh interval elapsed.
    fn poll_catalog(&mut self, pool: &mut JobPool) {
        let completions = self
            .completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()));
        self.catalog.poll(
            self.exiting,
            self.catalog_backend.as_ref(),
            completions,
            pool,
            Completion::Catalog,
        );
    }
    /// Publish native and professional observations, filtered by the running
    /// owner generation.
    #[cfg(windows)]
    fn poll_observations(
        &mut self,
        c: &mut ServiceContext,
        snapshot: &host::ProjectSnapshot,
        generation: SessionGeneration,
    ) {
        // Write-side stale guard shared by the native and professional
        // publishers: an observation counts only while its own owner
        // generation still runs.
        let fresh = |running: bool, owner_generation: u64| {
            running
                && owner_generation == snapshot.state.owner_generation
                && snapshot.state.execution == ExecutionState::Running
        };
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
            .filter(|observation| fresh(observation.running, observation.owner_generation))
            .map(|observation| NativeSessionObservations {
                generation,
                observation,
            });
        c.professional_observations = self
            .professional_observations
            .as_ref()
            .and_then(|handle| {
                handle
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .and_then(|value| value.snapshot())
            })
            .filter(|observation| fresh(observation.running, observation.owner_generation))
            .map(|observation| ProfessionalSessionObservations {
                generation,
                observation,
            });
    }
    #[cfg(not(windows))]
    fn poll_observations(
        &mut self,
        _c: &mut ServiceContext,
        _snapshot: &host::ProjectSnapshot,
        _generation: SessionGeneration,
    ) {
    }
    /// Project execution state, faults, the active latency profile, and
    /// settled-revision confirmations into the service context.
    fn poll_execution(&mut self, c: &mut ServiceContext, snapshot: &host::ProjectSnapshot) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
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
        if let Some(error) = error.as_deref()
            && c.core.error() != Some(error)
        {
            log::error!(
                target: "moiren::audio::service",
                "audio owner reported a backend fault: generation={} state={:?} fault={error}",
                owner.generation.0,
                snapshot.state.execution
            );
        }
        c.core.managed_execution(owner.generation, running, error);
        if running {
            // Professional streams use their explicit buffer configuration.
            // The saved latency preset belongs only to the legacy default path.
            c.active_latency_profile =
                (owner.audio_settings == AudioSettings::default()).then_some(owner.profile);
            c.active_audio_settings = Some(owner.audio_settings.clone());
            if owner.started.is_none() {
                log::info!(
                    target: "moiren::audio::service",
                    "audio owner started: generation={} backend={} processing_rate={} precision={:?} block_frames={}",
                    owner.generation.0,
                    owner.audio_settings.backend.title(),
                    owner.audio_settings.processing.sample_rate,
                    owner.audio_settings.processing.precision,
                    owner.audio_settings.processing.block_frames
                );
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
            c.active_audio_settings = None;
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
    }
    /// Retire finished host edits and establish the parameter-ACK barrier for
    /// newly activated applies.
    fn poll_edits(&mut self, c: &mut ServiceContext) {
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
    }
    /// Retire finished parameter submissions; a failed or closed confirmation
    /// tears down the current ACK barrier.
    fn poll_parameters(&mut self, c: &mut ServiceContext) {
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
    }
    /// Submit pending parameter values to the running owner within the round
    /// budget, then publish queue-depth counters.
    fn pump_parameters(&mut self, c: &mut ServiceContext, budget: RoundBudget) {
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
    fn finish_apply_confirmation(
        &mut self,
        c: &mut ServiceContext,
        snapshot: &host::ProjectSnapshot,
    ) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
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
            if snapshot.state.execution == ExecutionState::Running
                && snapshot.state.active_revision == Some(host_revision)
            {
                c.graph.confirmed(owner.generation, revision);
            }
        }
    }
    /// Pump the meter bridge from the owner's observer.
    fn poll_meters(&mut self, c: &mut ServiceContext) {
        let Some(owner) = self.owner.as_ref() else {
            return;
        };
        if let Some(meters) = &mut self.meters {
            meters.poll(c, &owner.observer);
        }
    }
    /// Stop and retire the owner once the requested run limit elapsed.
    fn poll_run_limit(&mut self, c: &mut ServiceContext) {
        let Some(owner) = self.owner.as_ref() else {
            return;
        };
        if let Some(RunLimit::Seconds(seconds)) = c.core.run_limit()
            && owner
                .started
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(u64::from(seconds)))
        {
            let _ = c.core.stop();
            self.retire(c);
            self.apply = false;
        }
    }
}
