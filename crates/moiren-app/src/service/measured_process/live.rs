//! Private observed-window proof; no mutable report enters application requests.
use super::*;
use crate::service::{ProcessTraceSessionOrigin, ServiceContext};
use moiren_windows_audio::engine_backend::NativeProcessWindowCandidate;
use moiren_windows_audio::process_loopback::{
    LiveProcessWindowReport, MeasuredProcessDelivery, ProcessTraceOrigin, ReplayWindowStatus,
};
use std::sync::Arc;

pub(crate) type WindowEvaluation =
    Box<dyn FnOnce() -> Result<LiveProcessEvaluation, String> + Send>;

/// Constructed by the private collection adapter from an opaque native result.
pub(crate) struct LiveProcessEvaluation {
    pub(super) measurement: MeasuredProcessDelivery,
    pub(super) report: LiveProcessWindowReport,
    #[cfg(test)]
    pub(crate) software_fixture: bool,
}

impl LiveProcessEvaluation {
    pub(crate) fn native(candidate: NativeProcessWindowCandidate) -> Self {
        Self {
            measurement: candidate.measurement().clone(),
            report: candidate.report().clone(),
            #[cfg(test)]
            software_fixture: false,
        }
    }

    pub(crate) fn profile(
        self,
        selection_id: u64,
        request: MeasuredProcessRequest,
        policy: ProcessWindowPolicy,
        origin: ProcessTraceSessionOrigin,
    ) -> Result<MeasuredProcessProfile, String> {
        let report = self.report;
        if report.status != ReplayWindowStatus::NoViolationInWindow {
            return Err(format!(
                "Process observed window {:?}: {}",
                report.status,
                report
                    .refusal
                    .as_deref()
                    .unwrap_or("exposure or supply/demand violation")
            ));
        }
        let fixture = {
            #[cfg(test)]
            {
                self.software_fixture
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let replay = report
            .replay
            .as_ref()
            .ok_or("Process window replay is missing")?;
        let prefix = report
            .prefix
            .as_ref()
            .ok_or("Process window seal is missing")?;
        if (!fixture
            && (report.trace_origin != ProcessTraceOrigin::RetainedNativeOwner
                || replay.report.trace_origin != ProcessTraceOrigin::RetainedNativeOwner))
            || report.refusal.is_some()
            || report.collection_refusal.is_some()
            || report.policy != policy.native()
            || replay.assessment.policy != policy.native()
            || replay.assessment.status != ReplayWindowStatus::NoViolationInWindow
            || report.owner_generation != origin.native_owner_generation
            || !report.process.same_process(&origin.process)
            || self.measurement.owner_generation() != origin.native_owner_generation
            || !self.measurement.process().same_process(&origin.process)
            || replay.report.owner_generation != origin.native_owner_generation
            || !replay.report.process.same_process(&origin.process)
            || replay.report.options != report.options
        {
            return Err("Process window needs matching retained native evidence".into());
        }
        let mut profile = MeasuredProcessProfile::from_evidence(
            selection_id,
            request,
            self.measurement,
            report.options.engine_sample_rate,
            report.options.max_demand_frames,
        )?;
        let options = profile.options(
            report.options.engine_sample_rate,
            report.options.max_demand_frames,
        )?;
        if options != report.options {
            return Err("Process window options differ from measured admission".into());
        }
        profile.summary.observed_window = Some(MeasuredProcessWindow {
            policy,
            cutoff_100ns: prefix.cutoff_100ns,
            replayed_packet_events: prefix.replayed_packet_events,
            replayed_read_events: prefix.replayed_read_events,
        });
        profile.evaluated_envelope = Some(options);
        profile.live_window = Some(Arc::new(LiveProcessSelection { origin, report }));
        Ok(profile)
    }

    #[cfg(test)]
    pub(crate) fn software(
        measurement: MeasuredProcessDelivery,
        report: LiveProcessWindowReport,
        already_issued: bool,
    ) -> Self {
        // Lifecycle-only fixture: preserve CallerSupplied labels. This bypass
        // is unavailable to application callers and all production builds.
        Self {
            measurement,
            report,
            software_fixture: already_issued,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LiveProcessSelection {
    pub(crate) origin: ProcessTraceSessionOrigin,
    pub(crate) report: LiveProcessWindowReport,
}

impl MeasuredProcessProfile {
    pub(crate) fn options(
        &self,
        consumer_rate: u32,
        demand: u32,
    ) -> Result<moiren_windows_audio::process_loopback::ProcessBridgeOptions, String> {
        // Before a window is bound, evidence carries the original rate/demand.
        self.evidence
            .admit_for_consumer(
                self.evidence.process(),
                self.summary.request.native_owner_generation,
                Self::policy(self.summary.request),
                consumer_rate,
                demand,
            )
            .map_err(|error| error.to_string())
    }
}

impl LiveProcessSelection {
    pub(crate) fn validate_context(
        &self,
        context: &ServiceContext,
        document: u64,
    ) -> Result<(), String> {
        let origin = &self.origin;
        let graph = context.graph.snapshot();
        if document != origin.document
            || context.audio_settings != origin.audio_settings
            || context.transport_latency != origin.transport_latency
            || context.latency_profile != origin.latency_profile
            || graph.candidate_revision != origin.graph_revision
            || graph.pending_revision.is_some()
            || graph
                .node(origin.node)
                .and_then(|node| node.source.as_ref())
                != Some(&InputSelection::Process {
                    pid: origin.process.pid,
                    creation_time_100ns: origin.process.creation_time_100ns,
                })
        {
            return Err(
                "observed Process window document, graph, source or settings changed".into(),
            );
        }
        Ok(())
    }
}
