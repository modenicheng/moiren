use super::*;

impl ManagedDriver {
    pub(super) fn retire(&mut self, c: &mut ServiceContext) {
        self.meters = None;
        c.signals.clear_node_levels();
        c.active_latency_profile = None;
        #[cfg(windows)]
        {
            if let Some(observations) = &self.observations {
                *observations.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            c.native_observations = None;
        }
        self.edits.clear();
        self.clear_parameters(c);
        if let Some(owner) = self.owner.take() {
            let Owner {
                host,
                generation,
                cleanup,
                completion,
                ..
            } = owner;
            self.cleaning += 1;
            let job: Job = Box::new(move || {
                drop(host);
                completion.send_or_dispose(Completion::Reaped(generation), drop);
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
        let preparation = c.graph.preparation();
        let block = c.core.desired().map_or(256, |spec| spec.max_block_frames);
        let project = match preparation.engine_project(block) {
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
        if self
            .owner
            .as_ref()
            .is_some_and(|o| o.document != self.document || o.profile != c.latency_profile)
        {
            if let Ok(spec) = c.graph.session_spec(c.core.desired()) {
                let _ = c.core.start(spec);
            }
            self.retire(c);
            return;
        }
        if let Some(owner) = self.owner.as_mut() {
            if self.edits.len() == 16 {
                return;
            }
            let Some(apply_id) = owner.latest_apply_id.checked_add(1) else {
                c.graph.record_error("应用请求身份空间已耗尽");
                self.apply = false;
                return;
            };
            let values = project
                .parameter_values()
                .into_iter()
                .map(|p| (p.target, p.value))
                .collect();
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
            let spec = match c.graph.session_spec(c.core.desired()) {
                Ok(spec) => spec,
                Err(error) => {
                    c.core.failed(c.core.generation(), error.clone());
                    c.graph.record_error(error);
                    self.apply = false;
                    return;
                }
            };
            if c.core.desired().is_none() && c.core.start(spec).is_err() {
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
                (self.factory)(c.latency_profile)
            }))
            .unwrap_or_else(|_| {
                Err(host::BackendError::Failed(
                    "backend factory panicked".into(),
                ))
            })
            .and_then(|backend| {
                EngineHost::open(self.host_config.clone(), project, Backend(backend))
                    .map_err(|error| host::BackendError::Failed(error.to_string()))
            });
            match result {
                Ok(host) => {
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
                        profile: c.latency_profile,
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
                    c.core.failed(c.core.generation(), error.to_string());
                    c.graph.record_error(error);
                }
            }
            self.apply = false;
        }
    }
}
