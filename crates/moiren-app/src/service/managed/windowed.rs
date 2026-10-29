//! Bounded background replay, followed by exact live-owner admission.
use super::*;
use crate::service::measured_process::{MeasuredProcessProfile, live::LiveProcessEvaluation};
use moiren_windows_audio::latency::transport::TransportInputPolicy;

pub(super) struct PendingWindowEvaluation {
    id: u64,
    request: MeasuredProcessRequest,
    policy: ProcessWindowPolicy,
    origin: ProcessTraceSessionOrigin,
    instance: Arc<()>,
}

fn matching_origin(a: &ProcessTraceSessionOrigin, b: &ProcessTraceSessionOrigin) -> bool {
    if !a.process.same_process(&b.process) {
        return false;
    }
    let mut a = a.clone();
    a.process = b.process.clone();
    a == *b
}

impl ManagedDriver {
    pub(super) fn begin_windowed_process(
        &mut self,
        c: &mut ServiceContext,
        pool: &mut JobPool,
        request: MeasuredProcessRequest,
        policy: ProcessWindowPolicy,
    ) -> Result<(), String> {
        request.validate()?;
        policy.validate()?;
        super::super::measured_process::validate_ingress_elapsed_admission(
            c.transport_latency.process_max_ingress_elapsed_100ns,
        )?;
        let host = self
            .owner
            .as_ref()
            .ok_or("Process window needs a Running owner")?
            .observer
            .read();
        let origin = self
            .process_trace_origin(c, &host)?
            .ok_or("Process window origin is stale or unsettled")?;
        if origin.generation != request.session_generation
            || origin.native_owner_generation != request.native_owner_generation
            || origin.graph_revision != request.graph_revision
            || origin.node != request.node
        {
            return Err("Process window request does not match the Running origin".into());
        }
        let id = self
            .window_evaluation_id
            .checked_add(1)
            .ok_or("Process window identity exhausted")?;
        let credit = self
            .queue()
            .port()
            .try_reserve()
            .map_err(|error| format!("Process window completion unavailable: {error:?}"))?;
        let trace = self
            .process_trace
            .as_mut()
            .ok_or("original native Process collection is unavailable")?;
        if !matching_origin(&origin, &trace.origin) {
            return Err("Process window original collection changed".into());
        }
        let evaluation = trace.prepare_window(
            TransportInputPolicy::Target {
                capacity_frames: request.capacity_frames,
                target_frames: request.target_frames,
                jitter_frames: request.jitter_frames,
            },
            policy.native(),
        )?;
        let pending = PendingWindowEvaluation {
            id,
            request,
            policy,
            origin: trace.origin.clone(),
            instance: trace.instance.clone(),
        };
        let job: Job = Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(evaluation))
                .unwrap_or_else(|_| Err("Process window evaluation panicked".into()));
            credit.send_or_dispose(
                Completion::WindowEvaluated {
                    id,
                    result: result.map(Box::new),
                },
                drop,
            );
        });
        pool.try_submit_owned(JobPriority::Prepare, job)
            .map_err(|(error, _)| format!("Process window worker unavailable: {error:?}"))?;
        self.window_evaluation_id = id;
        self.window_evaluation = Some(pending);
        Ok(())
    }

    pub(super) fn finish_windowed_process(
        &mut self,
        c: &mut ServiceContext,
        id: u64,
        result: Result<Box<LiveProcessEvaluation>, String>,
    ) {
        if self
            .window_evaluation
            .as_ref()
            .is_none_or(|pending| pending.id != id)
        {
            return;
        }
        let pending = self.window_evaluation.take().unwrap();
        let result: Result<(), String> = (|| {
            let host = self
                .owner
                .as_ref()
                .ok_or("Process window owner stopped")?
                .observer
                .read();
            let origin = self
                .process_trace_origin(c, &host)?
                .ok_or("Process window owner changed during evaluation")?;
            let trace = self
                .process_trace
                .as_ref()
                .ok_or("Process window original collection retired")?;
            if !matching_origin(&origin, &pending.origin)
                || !Arc::ptr_eq(&trace.instance, &pending.instance)
            {
                return Err("Process window original owner or collection changed".into());
            }
            let selection = self
                .measured_selection_id
                .checked_add(1)
                .ok_or("measured selection identity exhausted")?;
            let profile =
                (*result?).profile(selection, pending.request, pending.policy, pending.origin)?;
            self.measured_selection_id = selection;
            c.measured_process = Some(profile);
            self.automatic_startup = false;
            self.auto_apply_deadline = None;
            self.apply_settings = Some(ApplySettings::capture(c));
            self.apply = true;
            Ok(())
        })();
        if let Err(error) = result {
            c.graph.record_error(error);
        }
    }

    pub(super) fn validate_windowed_apply(
        &self,
        c: &ServiceContext,
        requested: &ApplySettings,
        profile: &MeasuredProcessProfile,
    ) -> Result<(), String> {
        let Some(window) = &profile.live_window else {
            return Ok(());
        };
        window.validate_context(c, self.document)?;
        let origin = &window.origin;
        if profile
            .summary()
            .observed_window
            .as_ref()
            .is_none_or(|summary| summary.policy.native() != window.report.policy)
        {
            return Err("Process window policy binding changed".into());
        }
        if requested.audio != origin.audio_settings
            || requested.transport != origin.transport_latency
            || requested.profile != origin.latency_profile
        {
            return Err("captured Process window settings differ from the evaluated owner".into());
        }
        if let Some(owner) = &self.owner {
            // A confirmed replacement already uses this proof. A later
            // structural re-open must obtain fresh evidence from that owner.
            if owner.measured_process.as_ref().map(|p| p.summary()) == Some(profile.summary()) {
                return Ok(());
            }
            let host = owner.observer.read();
            let selector = format!(
                "moiren:process:{}:{}",
                origin.process.pid, origin.process.creation_time_100ns
            );
            let source_matches = host.active.as_ref().and_then(|p| p.nodes.get(&origin.node))
                .is_some_and(|config| matches!(config, host::NodeConfig::Input(config)
                    if matches!(&config.source, moiren_engine::node::InputSource::Application { executable, .. } if *executable == selector)));
            if owner.generation != origin.generation
                || owner.document != origin.document
                || owner.active_revision != (origin.host_revision, origin.graph_revision)
                || owner.audio_settings != origin.audio_settings
                || owner.transport_latency != origin.transport_latency
                || owner.profile != origin.latency_profile
                || host.state.owner_generation != origin.native_owner_generation
                || host.state.execution != ExecutionState::Running
                || host.state.active_revision != Some(origin.host_revision)
                || host.state.desired_revision != origin.host_revision
                || !source_matches
                || !self.edits.is_empty()
                || self.confirm_parameters.is_some()
                || !self.parameters.is_empty()
                || !self.unsent.is_empty()
                || !self.confirmation_values.is_empty()
            {
                return Err("Process window predecessor changed before retirement".into());
            }
        } else if self.cleaning == 0 {
            let cleanup = c
                .last_cleanup
                .as_ref()
                .ok_or("Process window predecessor cleanup is missing")?;
            if cleanup.generation != origin.generation
                || cleanup.state.owner_generation != origin.native_owner_generation
                || cleanup.state.desired_revision != origin.host_revision
                || cleanup.state.host != host::HostState::Closed
                || cleanup.state.cleanup_error.is_some()
                || cleanup.state.fault.is_some()
            {
                return Err("Process window predecessor cleanup changed or failed".into());
            }
        }
        Ok(())
    }
}
