//! Cleanup-worker completions and startup automation hooks.
use super::super::*;

impl ManagedDriver {
    pub(in crate::service::managed) fn reject_failed_cleanup(
        &mut self,
        c: &mut ServiceContext,
        just_reaped: bool,
    ) -> bool {
        let Some(cleanup) = c.last_cleanup.as_ref() else {
            return false;
        };
        let state = &cleanup.state;
        if state.host == host::HostState::Closed
            && state.cleanup_error.is_none()
            && state.fault.is_none()
        {
            return false;
        }
        let measured = self
            .apply_settings
            .as_ref()
            .is_some_and(|settings| settings.measured_process.is_some());
        // A measured baseline needs a clean receipt. Declared replacement may
        // recover the modeled PCM disconnect, but no other queued work survives
        // a cleanup failure. Once Closed, a later explicit declared request can
        // retry without erasing the receipt or requiring a service restart.
        if state.host == host::HostState::Closed
            && !measured
            && (!just_reaped
                || (state.fault.is_none()
                    && matches!(
                        state.cleanup_error,
                        Some(host::BackendError::ResourceUnavailable(_))
                    )))
        {
            return false;
        }
        let error = format!(
            "audio owner cleanup failed: generation={} host={:?}, cleanup_error={:?}, fault={:?}",
            cleanup.generation.0, state.host, state.cleanup_error, state.fault,
        );
        // Keep the measured draft and original receipt for review; cancelling
        // queued work must not silently turn it into a declared replacement.
        self.apply = false;
        self.apply_settings = None;
        self.auto_apply_deadline = None;
        self.automatic_startup = false;
        c.core.failed(c.core.generation(), error.clone());
        c.graph.record_error(error);
        true
    }

    /// Drain owner-reaping and catalog-refresh completions from the cleanup
    /// worker.
    pub(in crate::service::managed) fn poll_completions(
        &mut self,
        c: &mut ServiceContext,
        budget: RoundBudget,
    ) {
        for _ in 0..budget.completions {
            let Some(completion) = self.queue().try_recv() else {
                break;
            };
            match completion {
                #[cfg(windows)]
                Completion::WindowEvaluated { id, result } => {
                    self.finish_windowed_process(c, id, result);
                }
                Completion::Exports(result) => {
                    for error in self.exports_runtime.complete(&mut c.exports, result) {
                        c.graph.record_error(error);
                    }
                }
                Completion::Reaped(cleanup) => {
                    self.cleaning -= 1;
                    let generation = cleanup.generation;
                    log::info!(
                        target: "moiren::audio::service",
                        "audio owner reaped: generation={generation:?} cleaning={}",
                        self.cleaning,
                    );
                    c.core.owner_reaped(generation);
                    #[cfg(windows)]
                    {
                        c.process_trace_status = None;
                    }
                    c.last_cleanup = Some(cleanup);
                    self.reject_failed_cleanup(c, true);
                }
                Completion::PluginScan(result) => {
                    self.plugin_scan.running = false;
                    match result {
                        Ok(library) => {
                            log::info!(
                                target: "moiren::audio::service",
                                "plugin scan completed: {} plugin(s), {} failure(s)",
                                library.len(),
                                library.failures.len(),
                            );
                            c.replace_plugins(library);
                        }
                        Err(error) => {
                            log::error!(
                                target: "moiren::audio::service",
                                "plugin scan failed: {error}"
                            );
                            c.graph.record_error(error.to_string());
                        }
                    }
                }
                Completion::Catalog(result) => {
                    self.catalog.finished();
                    match result {
                        Ok(mut catalog) => {
                            // Enumeration has no export lifecycle knowledge. Retain known owned
                            // MMDevice IDs until the lifecycle confirms absence, including removal.
                            catalog
                                .vad_endpoints
                                .extend(c.catalog.vad_endpoints.clone());
                            c.replace_catalog(catalog);
                            c.catalog_error = None;
                            if self.automatic_startup {
                                self.automatic_startup = false;
                                if let Some(spec) = startup::default_session_for_backend(
                                    &c.catalog,
                                    c.audio_settings.backend,
                                ) {
                                    let _ = c.graph.configure_session(&spec);
                                    self.apply_settings = Some(ApplySettings::capture(c));
                                    self.apply = true;
                                }
                                // Known divergence: managed automatic startup
                                // does not retry after an unusable catalog;
                                // the legacy driver retries every second.
                            }
                            if self.automatic_resume {
                                self.automatic_resume = false;
                                self.resume_restored(c);
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
    /// An explicit transport or project request outranks whatever startup
    /// automation is still pending.
    pub(in crate::service::managed) fn cancel_startup_automation(&mut self) {
        self.automatic_startup = false;
        self.automatic_resume = false;
        self.resume_notice = None;
    }
    /// Startup resume of a restored project. The restored graph already
    /// carries its device bindings, so unlike automatic startup nothing is
    /// rebound to the catalog defaults. IO nodes whose devices are absent
    /// stay in the graph: the backend prepares them as silent null streams
    /// and reports the degradation, so the node returns by itself once the
    /// device is back.
    pub(super) fn resume_restored(&mut self, c: &mut ServiceContext) {
        let snapshot = c.graph.snapshot();
        let mut stripped = Vec::new();
        for io in startup::strippable_offline_io(&snapshot, &c.catalog) {
            match c.graph.edit(GraphEdit::RemoveNode(io.node)) {
                Ok(_) => stripped.push(io),
                Err(error) => log::warn!(
                    target: "moiren::audio::service",
                    "resume kept offline node {:?} ({}): {error}",
                    io.node,
                    io.label
                ),
            }
        }
        if !stripped.is_empty() {
            let names = stripped
                .iter()
                .map(|io| {
                    let role = match io.kind {
                        moiren_core::graph::NodeKind::Source => "input",
                        _ => "output",
                    };
                    format!("'{}' ({role})", io.label)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let (nodes, devices) = if stripped.len() == 1 {
                ("node", "its device is")
            } else {
                ("nodes", "their devices are")
            };
            self.resume_notice = Some(super::GraphFault {
                subject: super::GraphSubject::Graph,
                cause: format!(
                    "Resumed without {} {nodes} because {devices} offline: {names}",
                    stripped.len()
                ),
                hint: Some(
                    "Reconnect the device, then undo (Ctrl+Z) to restore the removed node.".into(),
                ),
            });
        }
        self.apply_settings = Some(ApplySettings::capture(c));
        self.apply = true;
    }

    /// The stripped-device notice waits until the resumed session has fully
    /// activated: activation clears the fault group, so recording it earlier
    /// would erase it before users could see the degradation.
    pub(super) fn publish_resume_notice(&mut self, c: &mut ServiceContext) {
        if self.resume_notice.is_none() || self.owner.is_none() {
            return;
        }
        let graph = c.graph.snapshot();
        if graph.pending_revision.is_none()
            && graph.active_revision == Some(graph.candidate_revision)
            && let Some(notice) = self.resume_notice.take()
        {
            c.graph.record_fault(notice);
        }
    }
}
