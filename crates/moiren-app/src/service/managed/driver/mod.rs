//! Request dispatch and bounded projection of host work. The poll phases
//! live in sibling modules by concern: completions (cleanup worker),
//! exports, catalog (catalog/plugin-scan/VAD reconciliation), observations
//! (professional + execution projection), and ack (the Apply-ACK barrier).
use super::*;

mod ack;
mod catalog;
mod completions;
mod exports;
mod observations;

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
                    // the session stays intended, so the next poll re-opens
                    // the host under the new authorizations.
                    if self.owner.as_ref().is_some_and(|owner| {
                        owner.exports.iter().any(|intent| {
                            intent.enabled
                                && previous.get(&intent.endpoint_id).copied().unwrap_or(false)
                                    != c.exports.allow_default(intent.endpoint_id)
                        })
                    }) {
                        self.apply_audio_settings
                            .get_or_insert_with(|| c.audio_settings.clone());
                        if let Ok(summary) = c.graph.managed_session_spec(c.core.desired()) {
                            let _ = c.core.restart_managed_graph(summary);
                        }
                        self.retire(c);
                        self.apply = true;
                    }
                }
            }
            AppRequest::LoadProject(document) => match c.load_project(document) {
                Ok(()) => {
                    self.exports_runtime.revoke();
                    self.cancel_startup_automation();
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
            AppRequest::ScanPlugins { force } => self.plugin_scan.request(force),
            AppRequest::Start(spec) => {
                self.cancel_startup_automation();
                self.auto_apply_deadline = None;
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
                self.cancel_startup_automation();
                self.auto_apply_deadline = None;
                self.apply_audio_settings = Some(c.audio_settings.clone());
                self.apply = true;
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
                let edited_strip = match &edit {
                    GraphEdit::SetMixer { node, .. } => Some(*node),
                    _ => None,
                };
                if let Ok(structural) = c.graph.edit(edit) {
                    for (target, value) in values {
                        self.submit_parameter(c, target, value);
                    }
                    if let Some(node) = edited_strip {
                        self.sync_effective_strip_parameters(c, node);
                    }
                    if structural {
                        self.auto_apply_deadline = Some(Instant::now() + super::AUTO_APPLY_DELAY);
                    }
                }
            }
            // A restored candidate is a structural change riding the same
            // trailing debounce as structural edits, so bursts of undo steps
            // coalesce into one atomic plan swap.
            AppRequest::UndoGraph | AppRequest::RedoGraph => {
                let back = matches!(request, AppRequest::UndoGraph);
                let stepped = if back { c.graph.undo() } else { c.graph.redo() };
                if stepped {
                    self.auto_apply_deadline = Some(Instant::now() + super::AUTO_APPLY_DELAY);
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
            AppRequest::StartRecording {
                node,
                directory,
                format,
                mp3,
                flac,
            } => self.start_recording(c, node, directory, format, mp3, flac),
            AppRequest::PauseRecording { node } => self.pause_recording(c, node),
            AppRequest::ResumeRecording { node } => self.resume_recording(c, node),
            AppRequest::StopRecording { node } => self.stop_recording(c, node),
        }
    }
    fn poll(&mut self, c: &mut ServiceContext, pool: &mut JobPool, budget: RoundBudget) {
        self.poll_completions(c, budget);
        self.poll_catalog(pool);
        self.poll_plugin_scan(pool);
        if let Some(deadline) = self.auto_apply_deadline
            && deadline <= Instant::now()
        {
            self.auto_apply_deadline = None;
            self.apply_audio_settings
                .get_or_insert_with(|| c.audio_settings.clone());
            self.apply = true;
        }
        if self.apply {
            self.apply(c);
        }
        if let Some(owner) = self.owner.as_ref() {
            let snapshot = owner.observer.read();
            self.poll_observations(c, &snapshot, owner.generation);
            self.poll_execution(c, &snapshot);
            self.poll_recording_rebinds(c, &snapshot);
            self.poll_edits(c);
            self.poll_parameters(c);
            self.pump_parameters(c, budget);
            self.finish_apply_confirmation(c, &snapshot);
            self.poll_meters(c);
        }
        self.poll_exports(c, pool);
        // Recording workers finalize on their own threads; this folds their
        // statuses into the domain and queues terminal sessions for cleanup.
        self.poll_recordings(c);
        // One catalog reconciliation per poll, after every input it reads has
        // settled: the second of the former two calls ran after `poll_exports`
        // and saw the same resource/name evidence, so a single trailing call
        // lands on the identical end-of-poll catalog state.
        catalog::update_vad_catalog(c);
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        self.apply = false;
        self.apply_audio_settings = None;
        self.auto_apply_deadline = None;
        self.retire(c);
    }
    fn is_drained(&self) -> bool {
        self.auto_apply_deadline.is_none()
            && self.owner.is_none()
            && self.recording_sessions.is_empty()
            && self.cleaning == 0
            && !self.catalog.is_running()
            && !self.plugin_scan.running
            && self.exports_runtime.is_drained()
            && self
                .completions
                .as_ref()
                .is_none_or(|q| q.outstanding() == 0)
    }
}

impl ManagedDriver {
    /// Pump the meter surfaces from the owner's observer: the spectrum
    /// relay, then the level subscription registry.
    fn poll_meters(&mut self, c: &mut ServiceContext) {
        let Some(owner) = self.owner.as_ref() else {
            return;
        };
        if let Some(spectra) = &mut self.spectra {
            spectra.poll(c, &owner.observer);
        }
        if let Some(tone) = c.levels.poll(&owner.observer, &mut c.signals.topics) {
            c.tone_session = tone;
        }
    }
}
