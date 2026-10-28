use super::*;

impl ManagedDriver {
    pub(super) fn retire(&mut self, c: &mut ServiceContext) {
        self.exports_runtime.revoke();
        let generation = c.core.generation();
        log::info!(
            target: "moiren::audio::service",
            "audio owner retirement requested: generation={} owner_present={} cleaning={}",
            generation.0,
            self.owner.is_some(),
            self.cleaning,
        );
        self.meters = None;
        c.tone_session = false;
        c.signals.clear_node_levels();
        c.active_latency_profile = None;
        c.active_audio_settings = None;
        c.active_transport_latency = None;
        #[cfg(windows)]
        {
            if let Some(observations) = &self.native_observations {
                *observations.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            c.native_observations = None;
            if let Some(observations) = &self.professional_observations {
                *observations.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            c.professional_observations = None;
        }
        self.edits.clear();
        self.clear_parameters(c);
        if let Some(owner) = self.owner.take() {
            let Owner {
                host,
                observer,
                generation,
                cleanup,
                completion,
                ..
            } = owner;
            self.cleaning += 1;
            let job: Job = Box::new(move || {
                log::info!(
                    target: "moiren::audio::service",
                    "audio owner cleanup started: generation={generation:?}",
                );
                drop(host);
                log::info!(
                    target: "moiren::audio::service",
                    "audio owner cleanup finished: generation={generation:?}",
                );
                // The observer survives the host join without retaining its
                // native owner. Read the final cleanup result, not a pre-stop
                // Running snapshot or the fact that Drop returned.
                let cleanup = SessionCleanupObservation {
                    generation,
                    state: observer.read().state,
                };
                completion.send_or_dispose(Completion::Reaped(cleanup), drop);
            });
            // Reserved before opening the host, so stop never needs additional
            // storage and host Drop/join always runs on the cleanup worker.
            if c.defer_reserved_cleanup(cleanup, job).is_err() {
                unreachable!("owner cleanup permit belongs to context");
            }
        }
        c.graph.stopped();
        c.graph.clear_pending();
    }
    pub(super) fn apply(&mut self, c: &mut ServiceContext) {
        // Any apply — automatic or explicit — consumes the pending auto-apply
        // deadline; a structural edit during this apply re-arms it.
        self.auto_apply_deadline = None;
        let preparation = c.graph.preparation();
        let virtual_transport_ready = self.exports_runtime.adapter.is_some();
        c.exports.set_available(virtual_transport_ready);
        if !virtual_transport_ready
            && let Err(error) = c.exports.preflight_audio(&preparation.exports)
        {
            c.graph.record_error(error);
            self.apply = false;
            return;
        }
        let requested = self
            .apply_settings
            .as_ref()
            .expect("Apply captures the requested audio configuration")
            .clone();
        let settings = requested.audio;
        let block = settings.processing.block_frames as usize;
        let project =
            match preparation.configured_engine_project(block, &settings, requested.profile) {
                Ok(project) => project,
                Err(error) => {
                    if self.owner.is_none() && self.cleaning == 0 {
                        c.core.failed(c.core.generation(), error.clone());
                    }
                    c.graph.record_error(error);
                    self.apply = false;
                    return;
                }
            };
        // A failed PCM worker can drain while its PDO stays healthy/Present.
        // Waiting for transport admission before replacing that host would
        // strand Apply: only a fresh preparation can create its PCM session.
        // Physical resource waits retain their host when export transport is
        // still ready (including projects with no enabled virtual exports).
        let stranded_virtual_transport = self.owner.as_ref().is_some_and(|owner| {
            owner.observer.read().state.execution == ExecutionState::WaitingForResources
                && self
                    .exports_runtime
                    .lifecycle_ready_for(&c.exports, &preparation.exports)
                && !self
                    .exports_runtime
                    .ready_for(&c.exports, &preparation.exports)
        });
        if self.owner.as_ref().is_some_and(|o| {
            o.document != self.document
                || o.profile != requested.profile
                || o.audio_settings != settings
                || o.transport_latency != requested.transport
                || o.exports != preparation.exports
                || stranded_virtual_transport
        }) {
            if let Ok(summary) = c
                .graph
                .managed_session_spec(c.core.desired(), c.core.run_limit())
            {
                let _ = c.core.restart_managed_graph(summary);
            }
            self.retire(c);
            return;
        }
        if self.owner.is_some() {
            if !self
                .exports_runtime
                .ready_for(&c.exports, &preparation.exports)
            {
                return;
            }
        } else if !self
            .exports_runtime
            .lifecycle_ready_for(&c.exports, &preparation.exports)
        {
            return;
        }
        if let Some(owner) = self.owner.as_mut() {
            if self.edits.len() == 16 {
                return;
            }
            let Some(apply_id) = owner.latest_apply_id.checked_add(1) else {
                c.graph
                    .record_error("The apply request identity space is exhausted");
                self.apply = false;
                return;
            };
            let values = project
                .parameter_values()
                .into_iter()
                .map(|p| (p.target, p.value))
                .collect();
            // Validation above must leave the confirmed graph's admission
            // intact. Revoke before this edit can reach the independent owner.
            self.exports_runtime.revoke();
            match owner.control.edit(move |current| {
                *current = project;
                Ok(())
            }) {
                Ok(receipt) => {
                    owner.latest_apply_id = apply_id;
                    owner.latest_apply_revision =
                        owner.latest_apply_revision.max(preparation.revision);
                    self.confirm_parameters = None;
                    self.confirmation_values.clear();
                    c.graph.preparing(owner.generation, preparation.revision);
                    self.edits.push(PendingEdit {
                        apply_id,
                        revision: preparation.revision,
                        receipt,
                        values,
                    });
                    self.apply = false;
                }
                Err(error) => {
                    c.graph.record_error(error);
                    self.apply = false;
                }
            }
        } else if self.cleaning == 0 && !self.exiting {
            let mut summary = match c
                .graph
                .managed_session_spec(c.core.desired(), c.core.run_limit())
            {
                Ok(summary) => summary,
                Err(error) => {
                    c.core.failed(c.core.generation(), error.clone());
                    c.graph.record_error(error);
                    self.apply = false;
                    return;
                }
            };
            if let Some(spec) = &mut summary {
                spec.max_block_frames = block;
            }
            if c.core.managed_graph_desired(summary).is_err() {
                self.apply = false;
                return;
            }
            let Ok(cleanup) = c.try_reserve_cleanup() else {
                return;
            };
            let Ok(completion) = self.queue().port().try_reserve() else {
                return;
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.open_host(requested.profile, &settings, &requested.transport, project)
            }))
            .unwrap_or_else(|_| {
                Err(host::BackendError::Failed(
                    "backend factory panicked".into(),
                ))
            });
            match result {
                Ok(host) => {
                    log::info!(
                        target: "moiren::audio::service",
                        "audio host prepared: generation={} backend={} mode={:?} processing_rate={} precision={:?} block_frames={}",
                        c.core.generation().0,
                        settings.backend.title(),
                        settings.output.mode,
                        settings.processing.sample_rate,
                        settings.processing.precision,
                        settings.processing.block_frames
                    );
                    let observer = host.observer();
                    let control = host.control();
                    let generation = c.core.generation();
                    c.graph.preparing(generation, preparation.revision);
                    self.meters = meters::MeterBridge::new(c).ok();
                    self.owner = Some(Owner {
                        host,
                        control,
                        observer,
                        generation,
                        document: self.document,
                        profile: requested.profile,
                        audio_settings: settings,
                        transport_latency: requested.transport,
                        exports: preparation.exports.clone(),
                        active_revision: (1, preparation.revision),
                        active_apply_id: 0,
                        latest_apply_revision: preparation.revision,
                        latest_apply_id: 0,
                        cleanup,
                        completion,
                        started: None,
                    });
                }
                Err(error) => {
                    log::error!(
                        target: "moiren::audio::service",
                        "audio host preparation failed: generation={} backend={} mode={:?} processing_rate={} precision={:?} block_frames={} error={error}",
                        c.core.generation().0,
                        settings.backend.title(),
                        settings.output.mode,
                        settings.processing.sample_rate,
                        settings.processing.precision,
                        settings.processing.block_frames
                    );
                    c.core.failed(c.core.generation(), error.to_string());
                    c.graph.record_error(error);
                }
            }
            self.apply = false;
        }
    }
    fn open_host(
        &self,
        profile: AudioLatencyProfile,
        settings: &AudioSettings,
        transport: &TransportLatencySettings,
        project: host::EngineProject,
    ) -> Result<AudioHost, host::BackendError> {
        log::info!(
            target: "moiren::audio::service",
            "opening audio host: backend={} mode={:?} processing_rate={} precision={:?} block_frames={} input_rate={:?} input_format={:?} input_buffer={:?} input_mode={:?} output_rate={:?} output_format={:?} output_buffer={:?} output_mode={:?}",
            settings.backend.title(),
            settings.output.mode,
            settings.processing.sample_rate,
            settings.processing.precision,
            settings.processing.block_frames,
            settings.input.sample_rate,
            settings.input.format,
            settings.input.buffer_frames,
            settings.input.mode,
            settings.output.sample_rate,
            settings.output.format,
            settings.output.buffer_frames,
            settings.output.mode
        );
        match settings.processing.precision {
            ProcessingPrecision::F32 => {
                let backend = (self.factory)(profile, settings, transport)?;
                EngineHost::open(self.host_config.clone(), project, backend).map(AudioHost::F32)
            }
            ProcessingPrecision::F64 => {
                let factory = self.f64_factory.as_ref().ok_or_else(|| {
                    host::BackendError::Unsupported(
                        "this driver has no f64 processing backend".into(),
                    )
                })?;
                let backend = factory(profile, settings, transport)?;
                EngineHost::open(self.host_config.clone(), project, backend).map(AudioHost::F64)
            }
        }
        .map_err(|error| host::BackendError::Failed(error.to_string()))
    }
}
