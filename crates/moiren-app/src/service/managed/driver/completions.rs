//! Cleanup-worker completions and startup automation hooks.
use super::super::*;

impl ManagedDriver {
    /// Drain owner-reaping and catalog-refresh completions from the cleanup
    /// worker.
    pub(super) fn poll_completions(&mut self, c: &mut ServiceContext, budget: RoundBudget) {
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
                                    self.apply_audio_settings = Some(c.audio_settings.clone());
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
    pub(super) fn cancel_startup_automation(&mut self) {
        self.automatic_startup = false;
        self.automatic_resume = false;
    }
    /// Startup resume of a restored project. The restored graph already
    /// carries its device bindings, so unlike automatic startup nothing is
    /// rebound to the catalog defaults. IO nodes whose devices are absent
    /// stay in the graph: the backend prepares them as silent null streams
    /// and reports the degradation, so the node returns by itself once the
    /// device is back.
    pub(super) fn resume_restored(&mut self, c: &mut ServiceContext) {
        self.apply_audio_settings = Some(c.audio_settings.clone());
        self.apply = true;
    }
}
