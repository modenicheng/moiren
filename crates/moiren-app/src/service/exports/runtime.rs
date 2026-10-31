//! Bounded lifecycle work shared by the managed driver and fake-adapter tests.
//! Only the adapter's atomic admission invalidation runs on the service pump.
use super::reconcile::same_resource;
use super::*;
use crate::service::{CompletionPermit, GraphSnapshot, JobPool, JobPriority};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) type DriverStamp = [u64; 5];

// Native adapters and portable fake adapters consume these work records.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) struct Dispatch {
    pub action: ExportAction,
    pub expected: Option<DriverStamp>,
}
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) struct Work {
    pub actions: Vec<Dispatch>,
    pub revision: Option<u64>,
    pub ticket: u64,
    pub drained: bool,
    pub shutdown: bool,
}
pub(crate) struct NativeObservation {
    pub intent: ExportIntent,
    pub snapshot: Option<DriverStamp>,
    /// Set only by the native adapter for a fresh, closed Removed snapshot.
    #[cfg(feature = "vad-integration-probe")]
    pub native_removed: Option<DriverStamp>,
    pub resource: ExportResourceState,
    pub gate_open: bool,
    pub allow_default: bool,
    pub can_open: bool,
    pub error: Option<String>,
    #[cfg(windows)]
    pub diagnostics: Option<ExportDiagnostics>,
    #[cfg(windows)]
    pub diagnostic_provenance: moiren_windows_audio::vad::VadExportDiagnosticProvenance,
    pub windows_name: Option<WindowsEndpointNameObservation>,
}
pub(crate) struct Batch {
    pub observations: Vec<NativeObservation>,
    pub errors: Vec<String>,
}
pub(crate) trait Adapter: Send + Sync {
    /// No IO, waiting, or locks shared with control/PCM workers.
    fn authorize(&self, revision: Option<u64>) -> u64;
    fn execute(&self, work: Work) -> Batch;
}

pub(crate) struct Runtime {
    pub adapter: Option<Arc<dyn Adapter>>,
    in_flight: bool,
    native_empty: bool,
    shutdown_in_flight: bool,
    shutdown_complete: bool,
    ticket: u64,
    basis: Option<[u64; 5]>,
    stamps: BTreeMap<VirtualEndpointId, DriverStamp>,
    healthy: BTreeSet<VirtualEndpointId>,
    ready: BTreeSet<VirtualEndpointId>,
    sequence: u64,
    next_poll: Instant,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            adapter: None,
            in_flight: false,
            native_empty: true,
            shutdown_in_flight: false,
            shutdown_complete: false,
            ticket: 0,
            basis: None,
            stamps: BTreeMap::new(),
            healthy: BTreeSet::new(),
            ready: BTreeSet::new(),
            sequence: 0,
            next_poll: Instant::now(),
        }
    }
}
impl Runtime {
    /// Confirms that the native endpoint resource can participate in an owner
    /// preparation. Transport admission is checked separately before opening
    /// its gate because the prepared PCM worker may still be unavailable.
    pub fn lifecycle_ready_for(&self, service: &ExportService, intents: &[ExportIntent]) -> bool {
        let mut enabled = intents.iter().filter(|intent| intent.enabled);
        if !service.available && intents.iter().any(|intent| intent.enabled) {
            return false;
        }
        enabled.all(|intent| {
            service
                .observations
                .get(&intent.endpoint_id)
                .is_some_and(|observation| {
                    observation.resource == ExportResourceState::Present
                        && self.healthy.contains(&intent.endpoint_id)
                        && same_resource(intent, &observation.intent)
                        && observation.allow_default == service.allow_default(intent.endpoint_id)
                })
        })
    }

    /// Only bound exports lease a PCM session from the Engine owner. An
    /// enabled, unbound endpoint remains present with its admission gate closed.
    pub fn ready_for(&self, service: &ExportService, intents: &[ExportIntent]) -> bool {
        intents
            .iter()
            .filter(|intent| intent.enabled && intent.binding.is_some())
            .all(|intent| {
                service
                    .observations
                    .get(&intent.endpoint_id)
                    .is_some_and(|o| {
                        o.resource == ExportResourceState::Present
                            && self.ready.contains(&intent.endpoint_id)
                            && same_resource(intent, &o.intent)
                            && o.allow_default == service.allow_default(intent.endpoint_id)
                    })
            })
    }
    pub fn revoke(&mut self) {
        if let Some(adapter) = &self.adapter {
            self.ticket = adapter.authorize(None);
        }
        self.basis = None;
        self.next_poll = Instant::now();
    }
    pub fn is_drained(&self) -> bool {
        !self.in_flight && (self.native_empty || self.shutdown_complete)
    }

    /// After Engine/PCM retirement, request revocation once and release the
    /// owning session on service teardown. Waiting for final PDO destruction
    /// here would deadlock uncertain CREATEs and client-held Removing devices.
    /// This never turns an unknown/removing observation into confirmed absence.
    pub fn shutdown_work(&self, drained: bool) -> Option<Work> {
        (drained && self.adapter.is_some() && !self.in_flight && !self.shutdown_complete).then_some(
            Work {
                actions: Vec::new(),
                revision: None,
                ticket: self.ticket,
                drained: true,
                shutdown: true,
            },
        )
    }

    pub fn complete(
        &mut self,
        service: &mut ExportService,
        result: Result<Batch, String>,
    ) -> Vec<String> {
        self.in_flight = false;
        // Every completion replaces diagnostic/probe evidence, including missing
        // endpoints and failed polls. Dispatch keeps its own stamp policy.
        #[cfg(windows)]
        service.diagnostics.clear();
        #[cfg(feature = "vad-integration-probe")]
        service.native_stamps.clear();
        #[cfg(feature = "vad-integration-probe")]
        service.observation_generations.clear();
        if std::mem::take(&mut self.shutdown_in_flight) {
            // Even a failed retirement attempt must permit closing the owning
            // file. Its cleanup revokes every endpoint, including unknown keys.
            self.shutdown_complete = true;
        }
        let batch = match result {
            Ok(batch) => batch,
            Err(error) => {
                self.healthy.clear();
                self.ready.clear();
                service.windows_names.clear();
                self.revoke();
                return vec![error];
            }
        };
        service.windows_names.clear();
        self.native_empty = batch
            .observations
            .iter()
            .all(|o| o.resource == ExportResourceState::Absent);
        self.stamps.clear();
        self.healthy.clear();
        self.ready.clear();
        let mut errors = batch.errors;
        for native in batch.observations {
            let id = native.intent.endpoint_id;
            #[cfg(feature = "vad-integration-probe")]
            let accepted_result = native.error.is_none();
            #[cfg(feature = "vad-integration-probe")]
            if service.available
                && native.error.is_none()
                && native.intent.validate().is_ok()
                && let Some(stamp) = native.snapshot
                && stamp[..3].iter().all(|value| *value != 0)
            {
                // Historical evidence stays separate from current observations.
                // A new control session cannot inherit the previous tombstone.
                if service
                    .native_retirements
                    .values()
                    .any(|old| old.native_stamp[1] != stamp[1])
                {
                    service.native_retirements.clear();
                }
                if native.resource == ExportResourceState::Absent
                    && !native.gate_open
                    && !native.can_open
                    && native.native_removed == Some(stamp)
                    && service
                        .native_retirements
                        .get(&id)
                        .is_none_or(|old| old.native_stamp != stamp)
                {
                    self.sequence = self
                        .sequence
                        .checked_add(1)
                        .expect("export observation sequence exhausted");
                    if service.native_retirements.len() >= MAX_EXPORTS
                        && !service.native_retirements.contains_key(&id)
                        && let Some(oldest) = service
                            .native_retirements
                            .iter()
                            .min_by_key(|(_, evidence)| evidence.generation)
                            .map(|(id, _)| *id)
                    {
                        service.native_retirements.remove(&oldest);
                    }
                    service.native_retirements.insert(
                        id,
                        ExportRetirementEvidence {
                            generation: self.sequence,
                            native_stamp: stamp,
                        },
                    );
                }
            }
            if native.resource == ExportResourceState::Absent {
                // Project only the probe's accepted sequence. Keep the live
                // provider observation absent so reconciliation still plans
                // a fresh CREATE and no historical stamp becomes current.
                self.sequence = self
                    .sequence
                    .checked_add(1)
                    .expect("export observation sequence exhausted");
                let generation = self.sequence;
                let accepted = native.error.is_none()
                    && !native.gate_open
                    && !native.can_open
                    && service
                        .observe(ExportObservation {
                            intent: native.intent.clone(),
                            generation,
                            resource: ExportResourceState::Absent,
                            allow_default: native.allow_default,
                            gate_open: false,
                            active_revision: None,
                        })
                        .is_ok();
                if accepted {
                    #[cfg(feature = "vad-integration-probe")]
                    service.observation_generations.insert(id, generation);
                }
                service.observations.remove(&id);
                continue;
            }
            if let Some(stamp) = native.snapshot {
                self.stamps.insert(native.intent.endpoint_id, stamp);
            }
            #[cfg(windows)]
            let current_name = native.diagnostic_provenance
                == moiren_windows_audio::vad::VadExportDiagnosticProvenance::Fresh;
            #[cfg(not(windows))]
            let current_name = true;
            let windows_name = if current_name
                && native.resource == ExportResourceState::Present
                && native.error.is_none()
            {
                native
                    .windows_name
                    .as_ref()
                    .filter(|name| {
                        native.snapshot.is_some_and(|stamp| {
                            name.native_identity == [stamp[0], stamp[1], stamp[2]]
                        }) && name.error.is_none()
                    })
                    .cloned()
            } else {
                None
            };
            #[cfg(feature = "vad-integration-probe")]
            let native_stamp = native.snapshot.filter(|_| {
                native.error.is_none() && native.resource != ExportResourceState::Unavailable
            });
            if native.can_open && native.error.is_none() {
                self.ready.insert(native.intent.endpoint_id);
            }
            if native.error.is_none() {
                self.healthy.insert(native.intent.endpoint_id);
            }
            #[cfg(windows)]
            let diagnostics = native.diagnostics.map(|mut diagnostics| {
                // Evidence belongs to this lifecycle result, never to identity
                // or generation values supplied in a detached diagnostic DTO.
                let fresh = native.diagnostic_provenance
                    == moiren_windows_audio::vad::VadExportDiagnosticProvenance::Fresh;
                diagnostics.native_stamp = native.snapshot.filter(|_| fresh);
                if !fresh {
                    // QUERY failure retains lifecycle/control state, but the
                    // cached measurement must not acquire this poll's sequence.
                    diagnostics.telemetry = None;
                }
                diagnostics.error = native.error.clone();
                diagnostics.windows_name = native
                    .windows_name
                    .filter(|_| fresh && native.resource == ExportResourceState::Present)
                    .filter(|name| {
                        native.snapshot.is_some_and(|stamp| {
                            name.native_identity == [stamp[0], stamp[1], stamp[2]]
                        })
                    });
                diagnostics
            });
            if let Some(error) = native.error {
                errors.push(error);
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .expect("export observation sequence exhausted");
            let generation = self.sequence;
            if let Err(error) = service.observe(ExportObservation {
                intent: native.intent,
                generation,
                resource: native.resource,
                allow_default: native.allow_default,
                gate_open: native.gate_open,
                active_revision: None,
            }) {
                errors.push(error);
            } else {
                if let Some(name) = windows_name {
                    service.windows_names.insert(id, name);
                }
                #[cfg(feature = "vad-integration-probe")]
                if native.resource != ExportResourceState::Unavailable && accepted_result {
                    service.observation_generations.insert(id, generation);
                }
                #[cfg(windows)]
                if let Some(mut diagnostics) =
                    diagnostics.filter(|_| native.resource != ExportResourceState::Unavailable)
                {
                    diagnostics.generation = self.sequence;
                    service.diagnostics.insert(id, diagnostics);
                }
                #[cfg(feature = "vad-integration-probe")]
                if let Some(stamp) = native_stamp {
                    service.native_stamps.insert(id, stamp);
                }
            }
        }
        errors
    }

    pub fn project(
        &mut self,
        service: &mut ExportService,
        graph: &GraphSnapshot,
        active: &[ExportIntent],
        basis: Option<[u64; 5]>,
    ) {
        service.set_available(self.adapter.is_some());
        if self.basis != basis {
            self.ticket = self
                .adapter
                .as_ref()
                .map_or(0, |a| a.authorize(basis.map(|b| b[3])));
            self.basis = basis;
        }
        for observation in service.observations.values_mut() {
            observation.active_revision = None;
            if observation.resource == ExportResourceState::Present
                && self.ready.contains(&observation.intent.endpoint_id)
                && let Some(intent) = active.iter().find(|i| {
                    i.enabled && i.binding.is_some() && same_resource(i, &observation.intent)
                })
                && let Some(basis) = basis
                && graph.active_revision == Some(basis[3])
                && observation.allow_default
                    == service
                        .authorizations
                        .get(&intent.endpoint_id)
                        .copied()
                        .unwrap_or(false)
            {
                observation.intent.binding = intent.binding;
                observation.active_revision = Some(basis[3]);
            }
        }
    }

    pub fn work(
        &mut self,
        service: &ExportService,
        graph: &GraphSnapshot,
        drained: bool,
    ) -> Option<Work> {
        if self.adapter.is_none()
            || self.in_flight
            || self.shutdown_complete
            || Instant::now() < self.next_poll
        {
            return None;
        }
        let plan = service.reconcile(graph);
        let actions = plan
            .actions
            .into_iter()
            .filter(|action| match action {
                ExportAction::SetGate {
                    endpoint_id,
                    open: true,
                } => {
                    self.basis.is_some()
                        && graph
                            .exports
                            .iter()
                            .find(|intent| intent.endpoint_id == *endpoint_id)
                            .is_some_and(|intent| {
                                self.ready_for(service, std::slice::from_ref(intent))
                            })
                }
                ExportAction::Remove { .. } => drained,
                _ => true,
            })
            .map(|action| {
                let id = match &action {
                    ExportAction::Create { intent, .. } => intent.endpoint_id,
                    ExportAction::Remove { endpoint_id }
                    | ExportAction::SetGate { endpoint_id, .. }
                    | ExportAction::SetDefaultEligibility { endpoint_id, .. } => *endpoint_id,
                };
                Dispatch {
                    expected: self.stamps.get(&id).copied(),
                    action,
                }
            })
            .collect();
        Some(Work {
            actions,
            revision: self.basis.map(|b| b[3]),
            ticket: self.ticket,
            drained,
            shutdown: false,
        })
    }

    pub fn submit<T: Send + 'static>(
        &mut self,
        pool: &mut JobPool,
        completion: CompletionPermit<T>,
        work: Work,
        wrap: impl FnOnce(Result<Batch, String>) -> T + Send + 'static,
    ) {
        let adapter = self.adapter.as_ref().expect("available adapter").clone();
        let shutdown = work.shutdown;
        let job = Box::new(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| adapter.execute(work)))
                    .map_err(|_| {
                        "export lifecycle adapter panicked; native state remains unknown".into()
                    });
            completion.send_or_dispose(wrap(result), drop);
        });
        if pool.try_submit(JobPriority::Prepare, job).is_ok() {
            self.in_flight = true;
            self.shutdown_in_flight = shutdown;
            self.native_empty = false;
            self.next_poll = Instant::now() + Duration::from_millis(20);
        }
    }
}

#[cfg(windows)]
pub(crate) struct WindowsAdapter(pub Arc<moiren_windows_audio::vad::WindowsVadProvider>);

#[cfg(windows)]
fn native_can_open(
    transport_prepared: bool,
    snapshot: Option<moiren_windows_audio::vad::EndpointSnapshot>,
) -> bool {
    use moiren_windows_audio::vad::PdoState;
    // Admission enables a prepared owner, not an external client's PCM stream.
    transport_prepared
        && snapshot.is_some_and(|s| {
            s.state() == PdoState::Present && s.policy_prepared() && s.audio_ready()
        })
}

#[cfg(windows)]
impl Adapter for WindowsAdapter {
    fn authorize(&self, revision: Option<u64>) -> u64 {
        self.0.set_active_revision(revision)
    }
    fn execute(&self, work: Work) -> Batch {
        use moiren_engine::host::BackendError as EngineBackendError;
        use moiren_windows_audio::vad::{Channels, Intent, PdoState};
        let dispatched = |result| match result {
            Ok(()) | Err(EngineBackendError::Cancelled) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
        let mut errors = Vec::new();
        if work.shutdown {
            self.0.set_active_revision(None);
            // No Engine/PCM owner remains. Close known gates before requesting
            // removal; do not wait for an external client to release its PDO.
            for view in self.0.export_observations() {
                if let Some(snapshot) = view.snapshot
                    && snapshot.gate_open()
                    && let Err(error) = self.0.set_export_gate(
                        view.endpoint_id,
                        snapshot_stamp(snapshot),
                        false,
                        0,
                        0,
                    )
                {
                    errors.push(error.to_string());
                }
            }
            // Gate close advances the epoch. Re-query rather than retire with
            // the old stamp. Unknown CREATEs are cleaned by last-file cleanup.
            for view in self.0.export_observations() {
                if let Some(snapshot) = view.snapshot
                    && matches!(snapshot.state(), PdoState::Creating | PdoState::Present)
                    && !snapshot.gate_open()
                    && let Err(error) = self
                        .0
                        .retire_export(view.endpoint_id, snapshot_stamp(snapshot))
                {
                    errors.push(error.to_string());
                }
            }
        }
        for dispatch in work.actions {
            let result = match dispatch.action {
                ExportAction::Create {
                    intent,
                    allow_default,
                } => {
                    let direction = match intent.direction {
                        ExportDirection::Playback => Intent::Playback,
                        ExportDirection::Microphone => Intent::Microphone,
                    };
                    Channels::try_from(intent.channels as u32)
                        .map_err(|e| e.to_string())
                        .and_then(|channels| {
                            self.0
                                .create_export(
                                    intent.endpoint_id,
                                    direction,
                                    channels,
                                    &intent.name,
                                    allow_default,
                                )
                                .map_err(|e| e.to_string())
                        })
                }
                ExportAction::SetGate { endpoint_id, open } => dispatch
                    .expected
                    .ok_or_else(|| "export snapshot is unknown; gate dispatch refused".to_string())
                    .and_then(|expected| {
                        dispatched(self.0.set_export_gate(
                            endpoint_id,
                            expected,
                            open,
                            work.revision.unwrap_or(0),
                            work.ticket,
                        ))
                    }),
                ExportAction::Remove { endpoint_id } if work.drained => dispatch
                    .expected
                    .ok_or_else(|| {
                        "export snapshot is unknown; retirement remains pending".to_string()
                    })
                    .and_then(|expected| dispatched(self.0.retire_export(endpoint_id, expected))),
                _ => Err("export retirement requires a drained owner".into()),
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        let provider_views = self.0.export_observations();
        let name_targets = provider_views
            .iter()
            .map(|view| moiren_windows_audio::vad::VadEndpointNameTarget {
                endpoint_id: view.endpoint_id,
                intent: view.intent,
                snapshot: view.snapshot,
                removing: view.removing,
                error: view.error.clone().or_else(|| {
                    (view.diagnostic_provenance
                        == moiren_windows_audio::vad::VadExportDiagnosticProvenance::CachedAfterQueryFailure)
                        .then(|| "QUERY failed; retained snapshot is not current name authority".into())
                }),
            })
            .collect::<Vec<_>>();
        let windows_names = self.0.export_name_observations(&name_targets);
        let observations = provider_views
            .into_iter()
            .map(|view| {
                let mut native = NativeObservation::from(view);
                native.windows_name = windows_names
                    .iter()
                    .find(|name| name.endpoint_id == native.intent.endpoint_id)
                    .map(|observation| WindowsEndpointNameObservation {
                        native_identity: observation.native_identity,
                        observed_qpc_100ns: observation.observed_qpc_100ns,
                        mmdevice_id: observation.mmdevice_id.clone(),
                        name: observation.name.clone(),
                        error: observation.error.clone(),
                    });
                native
            })
            .collect();
        Batch {
            observations,
            errors,
        }
    }
}

#[cfg(windows)]
impl From<moiren_windows_audio::vad::VadExportObservation> for NativeObservation {
    fn from(view: moiren_windows_audio::vad::VadExportObservation) -> Self {
        use moiren_windows_audio::vad::{Intent, PdoState};
        let resource = match view.snapshot.map(|s| s.state()) {
            Some(PdoState::Removed) => ExportResourceState::Absent,
            Some(PdoState::Removing) => ExportResourceState::Removing,
            _ if view.removing => ExportResourceState::Removing,
            Some(PdoState::Present) => ExportResourceState::Present,
            _ => ExportResourceState::Creating,
        };
        Self {
            intent: ExportIntent {
                endpoint_id: view.endpoint_id,
                name: view.spec_name,
                direction: if view.intent == Intent::Playback {
                    ExportDirection::Playback
                } else {
                    ExportDirection::Microphone
                },
                channels: view.channels as usize,
                enabled: true,
                binding: None,
            },
            snapshot: view.snapshot.map(snapshot_stamp),
            #[cfg(feature = "vad-integration-probe")]
            native_removed: view
                .snapshot
                .filter(|snapshot| {
                    snapshot.state() == PdoState::Removed
                        && !snapshot.gate_open()
                        && view.error.is_none()
                        && view.diagnostic_provenance
                            == moiren_windows_audio::vad::VadExportDiagnosticProvenance::Fresh
                })
                .map(snapshot_stamp),
            resource,
            gate_open: view.snapshot.is_some_and(|s| s.gate_open()),
            allow_default: view.allow_default,
            can_open: native_can_open(view.transport_prepared, view.snapshot),
            diagnostics: Some(ExportDiagnostics {
                // Runtime binds this evidence only after observe accepts it.
                generation: 0,
                native_stamp: view.snapshot.map(snapshot_stamp),
                transport_prepared: view.transport_prepared,
                telemetry: view.telemetry,
                error: view.error.clone(),
                windows_name: None,
            }),
            error: view.error,
            diagnostic_provenance: view.diagnostic_provenance,
            windows_name: None,
        }
    }
}

#[cfg(windows)]
fn snapshot_stamp(snapshot: moiren_windows_audio::vad::EndpointSnapshot) -> DriverStamp {
    [
        snapshot.pdo_token().get(),
        snapshot.session_epoch(),
        snapshot.endpoint_generation(),
        snapshot.stream_generation(),
        snapshot.gate_epoch(),
    ]
}

#[cfg(test)]
pub(crate) mod tests;
