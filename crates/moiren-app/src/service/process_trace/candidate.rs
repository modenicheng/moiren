//! Opaque, non-persisted selection. A mutable replay report cannot authorize Apply.
use super::ProcessTraceSessionEvidence;
use crate::service::{MeasuredProcessRequest, measured_process::MeasuredProcessProfile};
use moiren_windows_audio::process_loopback::{
    AssessedProcessReplay, ProcessBridgeOptions, ProcessTraceOrigin, ReplayWindowPolicy,
    ReplayWindowStatus,
};
use std::sync::Arc;

#[derive(Debug)]
struct Candidate {
    evidence: Arc<ProcessTraceSessionEvidence>,
    request: MeasuredProcessRequest,
    options: ProcessBridgeOptions,
    assessment: AssessedProcessReplay,
}

/// Issued only by conservative final native admission plus this exact replay.
/// It remains historical evidence, not sustained or physical qualification.
#[derive(Debug, Clone)]
pub struct EvaluatedProcessCandidate(Arc<Candidate>);

impl PartialEq for EvaluatedProcessCandidate {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl EvaluatedProcessCandidate {
    pub fn request(&self) -> MeasuredProcessRequest {
        self.0.request
    }
    pub fn assessment(&self) -> &AssessedProcessReplay {
        &self.0.assessment
    }
    pub(crate) fn evidence(&self) -> &Arc<ProcessTraceSessionEvidence> {
        &self.0.evidence
    }
    pub(crate) fn validate_stopped(
        &self,
        context: &crate::service::ServiceContext,
        document: u64,
    ) -> Result<(), String> {
        let origin = self.evidence().origin();
        let graph = context.graph.snapshot();
        let cleanup = context
            .last_cleanup
            .as_ref()
            .ok_or("evaluated predecessor cleanup is missing")?;
        if context.core.phase() != crate::service::SessionPhase::Idle
            || context.core.owner_generation().is_some()
            || document != origin.document
            || graph.candidate_revision != origin.graph_revision
            || graph.pending_revision.is_some()
            || context.audio_settings != origin.audio_settings
            || context.transport_latency != origin.transport_latency
            || context.latency_profile != origin.latency_profile
            || cleanup.generation != origin.generation
            || cleanup.state.host != moiren_engine::host::HostState::Closed
            || cleanup.state.cleanup_error.is_some()
            || cleanup.state.fault.is_some()
            || cleanup
                .process_trace
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, self.evidence()))
        {
            return Err(
                "evaluated Process predecessor, document, graph or settings changed".into(),
            );
        }
        self.evidence().ordered_trace()?;
        let source = graph
            .node(origin.node)
            .and_then(|node| node.source.as_ref());
        if source
            != Some(&crate::service::InputSelection::Process {
                pid: origin.process.pid,
                creation_time_100ns: origin.process.creation_time_100ns,
            })
        {
            return Err("evaluated Process source was removed or replaced".into());
        }
        Ok(())
    }
    pub(crate) fn profile(&self, selection_id: u64) -> Result<MeasuredProcessProfile, String> {
        let evidence = self
            .0
            .evidence
            .measurement
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        let profile = MeasuredProcessProfile::from_evidence(
            selection_id,
            self.0.request,
            evidence,
            self.0.options.engine_sample_rate,
            self.0.options.max_demand_frames,
        )?;
        Ok(profile
            .with_evaluated_envelope(self.0.options)
            .with_evaluated_candidate(self.clone()))
    }
}

#[cfg(test)]
pub(crate) fn software_candidate(
    evidence: Arc<ProcessTraceSessionEvidence>,
) -> EvaluatedProcessCandidate {
    // Test-only service/UI control fixture. Its report remains CallerSupplied;
    // this bypass never exists in production and never authenticates native IO.
    let request = MeasuredProcessRequest {
        node: evidence.origin.node,
        session_generation: evidence.origin.generation,
        native_owner_generation: evidence.origin.native_owner_generation,
        graph_revision: evidence.origin.graph_revision,
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    };
    let measured = evidence.measurement.as_ref().unwrap();
    let options = measured
        .admit_for_consumer(
            &evidence.origin.process,
            evidence.origin.native_owner_generation,
            moiren_windows_audio::latency::transport::TransportInputPolicy::Target {
                capacity_frames: 2048,
                target_frames: 256,
                jitter_frames: 0,
            },
            48_000,
            96,
        )
        .unwrap();
    let assessment = evidence
        .ordered_trace()
        .unwrap()
        .replay_window(
            &evidence.origin.process,
            evidence.origin.native_owner_generation,
            options,
            super::tests::policy(),
        )
        .unwrap();
    ProcessTraceSessionEvidence::candidate_from_assessment(evidence, request, options, assessment)
        .unwrap()
}

impl ProcessTraceSessionEvidence {
    /// Separate from declared diagnostic evaluation. No imported/caller trace
    /// or successful-but-inconclusive report can mint an application token.
    pub fn evaluate_for_apply(
        self: Arc<Self>,
        capacity_frames: u32,
        target_frames: u32,
        jitter_frames: u32,
        policy: ReplayWindowPolicy,
    ) -> Result<EvaluatedProcessCandidate, String> {
        let trace = self.ordered_trace()?;
        let evidence = self.measurement.as_ref().map_err(Clone::clone)?.clone();
        let request = MeasuredProcessRequest {
            node: self.origin.node,
            session_generation: self.origin.generation,
            native_owner_generation: self.origin.native_owner_generation,
            graph_revision: self.origin.graph_revision,
            capacity_frames,
            target_frames,
            jitter_frames,
        };
        request.validate()?;
        let options = evidence
            .admit_for_consumer(
                &self.origin.process,
                self.origin.native_owner_generation,
                moiren_windows_audio::latency::transport::TransportInputPolicy::Target {
                    capacity_frames,
                    target_frames,
                    jitter_frames,
                },
                self.artifact.consumer_sample_rate,
                self.artifact.prepared_consumer_demand_frames,
            )
            .map_err(|error| error.to_string())?;
        let assessment = trace
            .replay_window(
                &self.origin.process,
                self.origin.native_owner_generation,
                options,
                policy,
            )
            .map_err(|error| error.to_string())?;
        if assessment.report.trace_origin != ProcessTraceOrigin::RetainedNativeOwner {
            return Err("evaluated Apply requires retained native Process evidence".into());
        }
        Self::candidate_from_assessment(self, request, options, assessment)
    }

    fn candidate_from_assessment(
        evidence: Arc<Self>,
        request: MeasuredProcessRequest,
        options: ProcessBridgeOptions,
        assessment: AssessedProcessReplay,
    ) -> Result<EvaluatedProcessCandidate, String> {
        if assessment.assessment.status != ReplayWindowStatus::NoViolationInWindow {
            return Err(format!(
                "evaluated Process window is {:?}",
                assessment.assessment.status
            ));
        }
        Ok(EvaluatedProcessCandidate(Arc::new(Candidate {
            evidence,
            request,
            options,
            assessment,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_apply_observed_violation_or_insufficient_exposure_cannot_issue_a_candidate() {
        let mut graph = moiren_core::graph::LogicalGraph::new();
        let node = graph
            .create_node(moiren_core::graph::NodeKind::Source, 2)
            .unwrap();
        let mut evidence = super::super::tests::evidence(node);
        let observation = crate::service::measured_process::tests::observation(node);
        evidence.measurement = observation
            .measured(&observation.process, 7)
            .map_err(|e| e.to_string());
        let evidence = Arc::new(evidence);
        let fixture = software_candidate(evidence.clone());
        for (policy, expected) in [
            (
                ReplayWindowPolicy {
                    min_output_frames: 10_000,
                    ..super::super::tests::policy()
                },
                ReplayWindowStatus::Inconclusive,
            ),
            (
                ReplayWindowPolicy {
                    max_initial_unserved_frames: 0,
                    ..super::super::tests::policy()
                },
                ReplayWindowStatus::ObservedViolation,
            ),
        ] {
            let assessment = evidence
                .ordered_trace()
                .unwrap()
                .replay_window(&evidence.origin.process, 7, fixture.0.options, policy)
                .unwrap();
            assert_eq!(assessment.assessment.status, expected);
            assert!(
                ProcessTraceSessionEvidence::candidate_from_assessment(
                    evidence.clone(),
                    fixture.request(),
                    fixture.0.options,
                    assessment,
                )
                .unwrap_err()
                .contains(&format!("{expected:?}"))
            );
        }
    }
}
