//! One original source archive per managed host, bounded independently of RT.
use super::*;
use moiren_windows_audio::{
    engine_backend::NativeProcessTraceCollector,
    process_loopback::{ObservedProcessTrace, ProcessTraceArchive},
};

// Private seam lets host lifecycle tests use actual software bridge histories
// without publishing a fake native registry or authenticating editable DTOs.
pub(super) trait Collection: Send {
    fn poll(&mut self);
    fn refusal(&self) -> Option<String> {
        None
    }
    fn artifact(&self) -> ProcessTraceArchive;
    fn ordered_trace(&self) -> Result<ObservedProcessTrace, String>;
}

impl Collection for NativeProcessTraceCollector {
    fn refusal(&self) -> Option<String> {
        self.collection_refusal()
            .map(|error| format!("Process trace collection refused: {error:?}"))
    }
    fn poll(&mut self) {
        self.poll();
    }
    fn artifact(&self) -> ProcessTraceArchive {
        self.artifact()
    }
    fn ordered_trace(&self) -> Result<ObservedProcessTrace, String> {
        self.ordered_trace().map_err(|e| e.to_string())
    }
}

pub(super) struct SessionTrace {
    pub(super) origin: ProcessTraceSessionOrigin,
    collection: Box<dyn Collection>,
    refusal: Option<String>,
}

impl SessionTrace {
    pub(super) fn new(origin: ProcessTraceSessionOrigin, collection: Box<dyn Collection>) -> Self {
        let refusal = collection.refusal();
        Self {
            origin,
            collection,
            refusal,
        }
    }

    fn observe(&mut self, owner: &Owner, document: u64, state: &host::HostSnapshot) {
        self.collection.poll();
        if let Some(refusal) = self.collection.refusal() {
            self.refusal.get_or_insert(refusal);
        }
        let origin = &self.origin;
        if owner.generation != origin.generation
            || owner.document != origin.document
            || document != origin.document
            || owner.active_revision != (origin.host_revision, origin.graph_revision)
            || owner.audio_settings != origin.audio_settings
            || owner.transport_latency != origin.transport_latency
            || owner.profile != origin.latency_profile
            || state.owner_generation != origin.native_owner_generation
            || state.active_revision != Some(origin.host_revision)
            || state.execution != ExecutionState::Running
        {
            self.refusal
                .get_or_insert_with(|| "Process trace origin changed during collection".into());
        }
    }

    pub(super) fn finish(
        mut self,
        cleanup: &host::HostSnapshot,
    ) -> Arc<ProcessTraceSessionEvidence> {
        // Called after host Drop has joined preparation, native and cleanup
        // workers. A registry's running=false alone never reaches this path.
        self.collection.poll();
        if cleanup.owner_generation != self.origin.native_owner_generation
            || cleanup.desired_revision != self.origin.host_revision
        {
            self.refusal.get_or_insert_with(|| {
                "Process trace native owner or host revision changed before cleanup".into()
            });
        }
        if cleanup.host != host::HostState::Closed
            || cleanup.cleanup_error.is_some()
            || cleanup.fault.is_some()
        {
            self.refusal
                .get_or_insert_with(|| "Process trace predecessor cleanup failed".into());
        }
        let trace = match self.collection.ordered_trace() {
            Ok(trace) if self.refusal.is_none() => Some(trace),
            Ok(_) => None,
            Err(error) => {
                self.refusal.get_or_insert(error);
                None
            }
        };
        Arc::new(ProcessTraceSessionEvidence {
            origin: self.origin,
            artifact: self.collection.artifact(),
            refusal: self.refusal,
            trace,
        })
    }
}

impl ManagedDriver {
    pub(super) fn process_trace_origin(
        &self,
        c: &ServiceContext,
        host: &host::ProjectSnapshot,
    ) -> Result<Option<ProcessTraceSessionOrigin>, String> {
        let Some(owner) = self.owner.as_ref() else {
            return Ok(None);
        };
        let graph = c.graph.snapshot();
        if self.exiting
            || self.apply
            || self.auto_apply_deadline.is_some()
            || !self.edits.is_empty()
            || self.confirm_parameters.is_some()
            || !self.parameters.is_empty()
            || !self.unsent.is_empty()
            || !self.confirmation_values.is_empty()
            || c.core.phase() != SessionPhase::Running
            || c.core.generation() != owner.generation
            || owner.document != self.document
            || owner.audio_settings != c.audio_settings
            || owner.transport_latency != c.transport_latency
            || owner.profile != c.latency_profile
            || c.active_audio_settings.as_ref() != Some(&owner.audio_settings)
            || c.active_transport_latency != Some(owner.transport_latency)
            || c.active_latency_profile != Some(owner.profile)
            || host.state.execution != ExecutionState::Running
            || host.state.active_revision != Some(owner.active_revision.0)
            || host.state.desired_revision != owner.active_revision.0
            || graph.active_revision != Some(owner.active_revision.1)
            || graph.pending_revision.is_some()
            || graph.candidate_revision != owner.active_revision.1
        {
            return Ok(None);
        }
        let mut sources = graph.nodes.iter().filter_map(|node| match node.source {
            Some(InputSelection::Process {
                pid,
                creation_time_100ns,
            }) => Some((node.id, pid, creation_time_100ns)),
            _ => None,
        });
        let Some((node, pid, creation_time_100ns)) = sources.next() else {
            return Ok(None);
        };
        if sources.next().is_some() {
            return Err("Process trace collection requires exactly one Process source".into());
        }
        if pid == 0 || creation_time_100ns == 0 {
            return Ok(None);
        }
        // Candidate labels alone cannot describe the executing host source.
        let selector = format!("moiren:process:{pid}:{creation_time_100ns}");
        let matches_active = host.active.as_ref().and_then(|project| project.nodes.get(&node))
            .is_some_and(|config| matches!(config, host::NodeConfig::Input(config)
                if matches!(&config.source, moiren_engine::node::InputSource::Application { executable, .. } if *executable == selector)));
        if !matches_active {
            return Ok(None);
        }
        Ok(Some(ProcessTraceSessionOrigin {
            generation: owner.generation,
            document: owner.document,
            node,
            process: moiren_windows_audio::process_loopback::ProcessIdentity {
                pid,
                creation_time_100ns,
                executable_name: String::new(),
            },
            native_owner_generation: host.state.owner_generation,
            host_revision: owner.active_revision.0,
            graph_revision: owner.active_revision.1,
            audio_settings: owner.audio_settings.clone(),
            transport_latency: owner.transport_latency,
            latency_profile: owner.profile,
        }))
    }

    pub(super) fn poll_process_trace(
        &mut self,
        c: &mut ServiceContext,
        host: &host::ProjectSnapshot,
    ) {
        let Some(owner) = self.owner.as_ref() else {
            return;
        };
        if let Some(trace) = &mut self.process_trace {
            trace.observe(owner, self.document, &host.state);
            if owner.audio_settings != c.audio_settings
                || owner.transport_latency != c.transport_latency
                || owner.profile != c.latency_profile
            {
                trace.refusal.get_or_insert_with(|| {
                    "Process trace requested settings changed during collection".into()
                });
            }
            c.process_trace_status = Some(ProcessTraceCollectionStatus {
                origin: Some(trace.origin.clone()),
                refusal: trace.refusal.clone(),
            });
            return;
        }
        let mut origin = match self.process_trace_origin(c, host) {
            Ok(Some(origin)) => origin,
            Ok(None) => return,
            Err(error) => {
                c.process_trace_status = Some(ProcessTraceCollectionStatus {
                    origin: None,
                    refusal: Some(error),
                });
                return;
            }
        };
        // Read provenance from the backend-owned registry, not the UI DTO.
        let retained = self.native_observations.as_ref().and_then(|registry| {
            registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .and_then(|registry| registry.retain_session())
        });
        let Some(retained) = retained else { return };
        if retained.running_generation() != Some(host.state.owner_generation) {
            return;
        }
        match retained.process_trace_collector(
            origin.node,
            &origin.process,
            host.state.owner_generation,
        ) {
            Ok(collection) => {
                origin.process = collection.process().clone();
                let trace = SessionTrace::new(origin.clone(), Box::new(collection));
                c.process_trace_status = Some(ProcessTraceCollectionStatus {
                    origin: Some(origin.clone()),
                    refusal: trace.refusal.clone(),
                });
                self.process_trace = Some(trace);
            }
            Err(error) => {
                c.process_trace_status = Some(ProcessTraceCollectionStatus {
                    origin: None,
                    refusal: Some(error.to_string()),
                })
            }
        }
    }

    pub(super) fn retiring_process_trace(
        &mut self,
        c: &mut ServiceContext,
    ) -> Option<SessionTrace> {
        // Take the exact original collector before live registries are cleared.
        if let Some(owner) = self.owner.as_ref() {
            let host = owner.observer.read();
            self.poll_process_trace(c, &host);
        }
        self.process_trace.take()
    }
}
