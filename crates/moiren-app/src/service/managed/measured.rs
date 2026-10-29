use super::*;
#[cfg(windows)]
use crate::service::measured_process::MeasuredProcessProfile;

impl ManagedDriver {
    #[cfg(windows)]
    pub(super) fn validate_evaluated_process(
        &self,
        context: &ServiceContext,
        candidate: &super::super::EvaluatedProcessCandidate,
    ) -> Result<(), String> {
        super::super::measured_process::validate_ingress_elapsed_admission(
            context.transport_latency.process_max_ingress_elapsed_100ns,
        )?;
        if self.exiting
            || self.owner.is_some()
            || self.cleaning != 0
            || self.auto_apply_deadline.is_some()
            || !self.edits.is_empty()
            || self.confirm_parameters.is_some()
            || !self.parameters.is_empty()
            || !self.unsent.is_empty()
            || !self.confirmation_values.is_empty()
        {
            return Err("evaluated Process Apply requires a settled stopped owner".into());
        }
        candidate.validate_stopped(context, self.document)
    }

    #[cfg(windows)]
    pub(super) fn select_evaluated_process(
        &mut self,
        context: &mut ServiceContext,
        candidate: &super::super::EvaluatedProcessCandidate,
    ) -> Result<(), String> {
        if self.apply {
            return Err("another Apply is pending".into());
        }
        self.validate_evaluated_process(context, candidate)?;
        let id = self
            .measured_selection_id
            .checked_add(1)
            .ok_or("measured selection identity exhausted")?;
        let profile = candidate.profile(id)?;
        self.measured_selection_id = id;
        context.measured_process = Some(profile);
        Ok(())
    }
    #[cfg(windows)]
    fn owner_for_measurement(
        &self,
        context: &ServiceContext,
        request: MeasuredProcessRequest,
    ) -> Result<host::ProjectSnapshot, String> {
        let owner = self
            .owner
            .as_ref()
            .ok_or("measured Process selection requires a Running owner")?;
        if self.exiting
            || self.apply
            || self.auto_apply_deadline.is_some()
            || !self.edits.is_empty()
            || self.confirm_parameters.is_some()
            || context.core.phase() != SessionPhase::Running
            || context.core.generation() != request.session_generation
            || owner.generation != request.session_generation
            || owner.document != self.document
            || owner.audio_settings != context.audio_settings
            || owner.profile != context.latency_profile
            || owner.transport_latency != context.transport_latency
        {
            return Err(
                "measured Process selection origin is stale or settings/apply are pending".into(),
            );
        }
        let snapshot = owner.observer.read();
        if snapshot.state.active_revision != Some(owner.active_revision.0)
            || owner.active_revision.1 != request.graph_revision
        {
            return Err("measured Process active host revision changed".into());
        }
        Ok(snapshot)
    }
    #[cfg(windows)]
    pub(super) fn select_measured_process(
        &mut self,
        context: &mut super::ServiceContext,
        request: MeasuredProcessRequest,
    ) -> Result<(), String> {
        request.validate()?;
        super::super::measured_process::validate_ingress_elapsed_admission(
            context.transport_latency.process_max_ingress_elapsed_100ns,
        )?;
        let graph = context.graph.snapshot();
        self.owner_for_measurement(context, request)?;
        if graph.candidate_revision != request.graph_revision
            || graph.active_revision != Some(request.graph_revision)
            || graph.pending_revision.is_some()
        {
            return Err("measured Process graph revision is stale or pending".into());
        }
        let native = context
            .native_observations
            .as_ref()
            .filter(|value| value.generation == request.session_generation)
            .filter(|value| {
                value.observation.running
                    && value.observation.owner_generation == request.native_owner_generation
            })
            .ok_or("matching live native Process observations are unavailable")?;
        let observation = native
            .observation
            .process_delivery
            .iter()
            .find(|value| value.node == request.node)
            .ok_or("selected Process node has no native delivery observations")?;
        let selection_id = self
            .measured_selection_id
            .checked_add(1)
            .ok_or("measured selection identity exhausted")?;
        let profile = MeasuredProcessProfile::from_observation(selection_id, request, observation)?;
        let source = graph
            .node(request.node)
            .and_then(|node| node.source.as_ref())
            .ok_or("selected Process node has no source")?;
        profile.validate_source(source)?;
        // Compare against the actual active host too, not only the last UI
        // projection; a native recovery can change generation between polls.
        let owner = self.owner_for_measurement(context, request)?;
        if owner.state.execution != moiren_engine::host::ExecutionState::Running
            || owner.state.owner_generation != request.native_owner_generation
        {
            return Err("measured Process native owner changed before selection".into());
        }
        self.measured_selection_id = selection_id;
        context.measured_process = Some(profile);
        Ok(())
    }

    #[cfg(not(windows))]
    pub(super) fn select_measured_process(
        &mut self,
        _context: &mut super::ServiceContext,
        _request: MeasuredProcessRequest,
    ) -> Result<(), String> {
        Err("measured Process application is only supported on Windows".into())
    }
}
