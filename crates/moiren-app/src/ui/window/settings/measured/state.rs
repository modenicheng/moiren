use super::{
    AppSnapshot, AudioSettings, Duration, InputSelection, Instant, MeasuredProcessRequest, NodeId,
    RequestId, SessionPhase,
};
use rust_i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::window::settings) struct MeasuredDraft {
    pub node: Option<NodeId>,
    pub capacity_frames: u32,
    pub target_frames: u32,
    pub jitter_frames: u32,
}

impl Default for MeasuredDraft {
    fn default() -> Self {
        Self {
            node: None,
            capacity_frames: 8192,
            target_frames: 2048,
            jitter_frames: 96,
        }
    }
}

impl MeasuredDraft {
    pub(super) fn selected_node(self, snapshot: &AppSnapshot) -> Option<NodeId> {
        self.node.or_else(|| {
            let mut sources = snapshot
                .graph
                .nodes
                .iter()
                .filter(|node| matches!(node.source, Some(InputSelection::Process { .. })));
            let node = sources.next()?;
            sources.next().is_none().then_some(node.id)
        })
    }

    #[cfg(windows)]
    pub(in crate::ui::window::settings) fn request(
        self,
        snapshot: &AppSnapshot,
    ) -> Result<MeasuredProcessRequest, String> {
        if snapshot.phase != SessionPhase::Running
            || snapshot.graph.active_revision != Some(snapshot.graph.candidate_revision)
            || snapshot.graph.pending_revision.is_some()
            || snapshot.active_audio_settings.as_ref() != Some(&snapshot.audio_settings)
            || snapshot.active_transport_latency != Some(snapshot.transport_latency)
            || snapshot.active_latency_profile != Some(snapshot.latency_profile)
        {
            return Err(t!(
                "Measured selection requires a matching running graph and saved settings."
            )
            .into());
        }
        let node = self
            .selected_node(snapshot)
            .ok_or_else(|| t!("Select a running Process source.").to_string())?;
        let source = snapshot
            .graph
            .node(node)
            .and_then(|node| node.source.as_ref())
            .ok_or_else(|| t!("Select a running Process source.").to_string())?;
        let native = snapshot
            .native_observations
            .as_ref()
            .filter(|n| n.generation == snapshot.generation && n.observation.running)
            .ok_or_else(|| t!("Matching live Process observations are unavailable.").to_string())?;
        let observed = native
            .observation
            .process_delivery
            .iter()
            .find(|p| p.node == node)
            .ok_or_else(|| t!("Matching live Process observations are unavailable.").to_string())?;
        let request = MeasuredProcessRequest {
            node,
            session_generation: snapshot.generation,
            native_owner_generation: native.observation.owner_generation,
            graph_revision: snapshot.graph.candidate_revision,
            capacity_frames: self.capacity_frames,
            target_frames: self.target_frames,
            jitter_frames: self.jitter_frames,
        };
        let profile = crate::service::measured_process::MeasuredProcessProfile::from_observation(
            1, request, observed,
        )?;
        profile.validate_source(source)?;
        Ok(request)
    }

    #[cfg(windows)]
    pub(super) fn live_request(
        self,
        snapshot: &AppSnapshot,
    ) -> Result<MeasuredProcessRequest, String> {
        let request = self.request(snapshot)?;
        if snapshot.measured_process != snapshot.active_measured_process {
            return Err("The original Process selection is not settled.".into());
        }
        let status = snapshot
            .process_trace_status
            .as_ref()
            .ok_or("Original live Process collection is unavailable.")?;
        if let Some(refusal) = &status.refusal {
            return Err(refusal.clone());
        }
        let origin = status
            .origin
            .as_ref()
            .ok_or("Original live Process collection identity is unavailable.")?;
        let source = snapshot
            .graph
            .node(request.node)
            .and_then(|node| node.source.as_ref());
        if origin.generation != request.session_generation
            || origin.native_owner_generation != request.native_owner_generation
            || origin.graph_revision != request.graph_revision
            || origin.node != request.node
            || origin.audio_settings != snapshot.audio_settings
            || origin.transport_latency != snapshot.transport_latency
            || origin.latency_profile != snapshot.latency_profile
            || source
                != Some(&InputSelection::Process {
                    pid: origin.process.pid,
                    creation_time_100ns: origin.process.creation_time_100ns,
                })
        {
            return Err("Original live Process collection, source or settings changed.".into());
        }
        if snapshot
            .transport_latency
            .process_max_ingress_elapsed_100ns
            .is_some()
        {
            return Err(
                "Observed-window admission cannot replay ingress elapsed enforcement.".into(),
            );
        }
        let native = &snapshot.native_observations.as_ref().unwrap().observation;
        let matching: Vec<_> = native
            .process_delivery
            .iter()
            .filter(|p| p.node == request.node)
            .collect();
        if matching.len() != 1 || !matching[0].process.same_process(&origin.process) {
            return Err("Original live Process observations are missing or ambiguous.".into());
        }
        for history in [&matching[0].producer, &matching[0].consumer] {
            let history = history
                .as_ref()
                .ok_or("Original live Process measurement history is unavailable.")?;
            if history.overwritten_events != 0 {
                return Err("Original live Process measurement history was overwritten; no per-wake fallback.".into());
            }
        }
        Ok(request)
    }

    #[cfg(not(windows))]
    pub(in crate::ui::window::settings) fn request(
        self,
        _: &AppSnapshot,
    ) -> Result<MeasuredProcessRequest, String> {
        Err(t!("Measured Process selection is only supported on Windows.").into())
    }
}

struct Pending {
    id: RequestId,
    request: MeasuredProcessRequest,
    previous_selection: Option<u64>,
    selection: Option<u64>,
    audio: AudioSettings,
    transport: crate::service::TransportLatencySettings,
    profile: crate::service::AudioLatencyProfile,
    previous_graph_error: Option<String>,
    began: Instant,
    retained: bool,
    window_policy: Option<crate::service::ProcessWindowPolicy>,
    #[cfg(windows)]
    previous_summary: Option<crate::service::MeasuredProcessSummary>,
    #[cfg(windows)]
    window_origin: Option<crate::service::ProcessTraceSessionOrigin>,
}

#[derive(Default)]
pub(in crate::ui::window::settings) struct MeasuredEditor {
    pub draft: MeasuredDraft,
    pub error: Option<String>,
    pub message: Option<String>,
    #[cfg(windows)]
    pub(super) window_open: bool,
    #[cfg(windows)]
    pub(super) window: super::window_state::WindowEditor,
    #[cfg(windows)]
    pub(super) live_open: bool,
    #[cfg(windows)]
    pub(super) live_policy: super::live::PolicyDraft,
    pending: Option<Pending>,
    confirmed: Option<(crate::service::SessionGeneration, u64)>,
}

impl MeasuredEditor {
    pub(super) fn expire(&mut self, id: RequestId) -> bool {
        if self.pending.as_ref().is_none_or(|pending| pending.id != id) {
            return false;
        }
        self.pending = None;
        self.message = None;
        self.error = Some(t!("Measured activation was not confirmed within 30 seconds. Check Diagnostics; audio may still change later.").into());
        true
    }

    pub(in crate::ui::window::settings) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn changed(&mut self) {
        #[cfg(windows)]
        self.window.changed();
        self.error = None;
        self.message = None;
        self.confirmed = None;
    }

    pub(in crate::ui::window::settings) fn begin(
        &mut self,
        id: RequestId,
        request: MeasuredProcessRequest,
        snapshot: &AppSnapshot,
    ) {
        self.changed();
        self.pending = Some(Pending {
            id,
            request,
            previous_selection: snapshot.measured_process.as_ref().map(|s| s.selection_id),
            selection: None,
            audio: snapshot.audio_settings.clone(),
            transport: snapshot.transport_latency,
            profile: snapshot.latency_profile,
            previous_graph_error: snapshot.graph.last_error.clone(),
            began: Instant::now(),
            retained: false,
            window_policy: None,
            #[cfg(windows)]
            previous_summary: snapshot.measured_process.clone(),
            #[cfg(windows)]
            window_origin: None,
        });
        self.message =
            Some(t!("Restarting audio; waiting for the measured budget to become active.").into());
    }

    #[cfg(windows)]
    pub(super) fn begin_live(
        &mut self,
        id: RequestId,
        request: MeasuredProcessRequest,
        policy: crate::service::ProcessWindowPolicy,
        snapshot: &AppSnapshot,
    ) {
        self.begin(id, request, snapshot);
        let pending = self.pending.as_mut().unwrap();
        pending.window_policy = Some(policy);
        pending.window_origin = snapshot
            .process_trace_status
            .as_ref()
            .and_then(|s| s.origin.clone());
        self.message = Some("Evaluating the original live Process window; waiting for exact policy admission and a replacement owner. Physical latency remains unqualified.".into());
    }

    #[cfg(windows)]
    pub(super) fn begin_retained(
        &mut self,
        id: RequestId,
        request: MeasuredProcessRequest,
        snapshot: &AppSnapshot,
    ) {
        self.begin(id, request, snapshot);
        self.pending.as_mut().unwrap().retained = true;
    }

    pub(in crate::ui::window::settings) fn observe(
        &mut self,
        snapshot: &AppSnapshot,
        dispatch_error: Option<&str>,
    ) {
        if self.confirmed.is_some_and(|(generation, selection)| {
            snapshot.phase != SessionPhase::Running
                || snapshot.generation != generation
                || snapshot
                    .active_measured_process
                    .as_ref()
                    .is_none_or(|s| s.selection_id != selection)
        }) {
            self.confirmed = None;
            self.message = None;
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        let outcome: Result<bool, String> = (|| {
            if pending.began.elapsed() >= Duration::from_secs(30) {
                return Err(t!("Measured activation was not confirmed within 30 seconds. Check Diagnostics; audio may still change later.").into());
            }
            if snapshot.consumed_request_id < pending.id {
                return Ok(false);
            }
            if let Some(error) = dispatch_error {
                return Err(error.to_owned());
            }
            if let Some(error) = &snapshot.error {
                return Err(error.clone());
            }
            if snapshot.audio_settings != pending.audio
                || snapshot.transport_latency != pending.transport
                || snapshot.latency_profile != pending.profile
            {
                return Err(t!("Settings changed while applying the measured budget.").into());
            }
            if snapshot.graph.candidate_revision != pending.request.graph_revision {
                return Err(t!("The graph changed while applying the measured budget.").into());
            }
            if let Some(error) = snapshot
                .graph
                .last_error
                .as_ref()
                .filter(|error| Some(*error) != pending.previous_graph_error.as_ref())
            {
                return Err(error.clone());
            }
            let selected = snapshot.measured_process.as_ref().filter(|s| {
                s.request == pending.request && Some(s.selection_id) != pending.previous_selection
            });
            // Receipts are cumulative: a later catalog refresh is not
            // cancellation. Keep waiting only on the exact captured evidence.
            if pending.window_policy.is_some() && selected.is_none() && pending.selection.is_none()
            {
                #[cfg(windows)]
                {
                    // Consumption acknowledges enqueue, not background
                    // evaluation. Only the same original settled owner may
                    // wait without a newly selected profile.
                    let origin = snapshot
                        .process_trace_status
                        .as_ref()
                        .filter(|s| s.refusal.is_none())
                        .and_then(|s| s.origin.as_ref());
                    let source = snapshot
                        .graph
                        .node(pending.request.node)
                        .and_then(|node| node.source.as_ref());
                    let expected_source =
                        pending
                            .window_origin
                            .as_ref()
                            .map(|o| InputSelection::Process {
                                pid: o.process.pid,
                                creation_time_100ns: o.process.creation_time_100ns,
                            });
                    if snapshot.phase == SessionPhase::Running
                        && snapshot.generation == pending.request.session_generation
                        && snapshot.measured_process == pending.previous_summary
                        && snapshot.active_measured_process == pending.previous_summary
                        && snapshot.graph.pending_revision.is_none()
                        && snapshot.graph.active_revision == Some(pending.request.graph_revision)
                        && origin == pending.window_origin.as_ref()
                        && origin.is_some()
                        && source == expected_source.as_ref()
                        && snapshot.native_observations.as_ref().is_some_and(|n| {
                            n.generation == pending.request.session_generation
                                && n.observation.running
                                && n.observation.owner_generation
                                    == pending.request.native_owner_generation
                        })
                    {
                        return Ok(false);
                    }
                }
                return Err("Original live Process collection, source or settings changed during evaluation.".into());
            }
            let selected = selected.ok_or_else(|| {
                snapshot.graph.last_error.clone().unwrap_or_else(|| {
                    t!("Measured budget was refused or replaced. Your draft is retained.").into()
                })
            })?;
            if selected
                .observed_window
                .as_ref()
                .map(|window| window.policy)
                != pending.window_policy
            {
                return Err("Measured Process admission mode or window policy differs from the captured request.".into());
            }
            if pending
                .selection
                .is_some_and(|id| id != selected.selection_id)
            {
                return Err(
                    t!("Measured budget was refused or replaced. Your draft is retained.").into(),
                );
            }
            pending.selection = Some(selected.selection_id);
            if matches!(
                snapshot.phase,
                SessionPhase::Failed | SessionPhase::Exiting | SessionPhase::Exited
            ) || (snapshot.phase == SessionPhase::Idle && !pending.retained)
            {
                return Err(snapshot.graph.last_error.clone().unwrap_or_else(|| {
                    t!("Audio stopped before the measured budget became active.").into()
                }));
            }
            if snapshot.phase != SessionPhase::Running
                || snapshot.generation == pending.request.session_generation
            {
                return Ok(false);
            }
            if snapshot
                .active_measured_process
                .as_ref()
                .is_some_and(|s| s != selected)
            {
                return Err(
                    t!("Measured budget was refused or replaced. Your draft is retained.").into(),
                );
            }
            #[cfg(windows)]
            let native_running = snapshot
                .native_observations
                .as_ref()
                .is_some_and(|n| n.generation == snapshot.generation && n.observation.running);
            #[cfg(not(windows))]
            let native_running = false;
            Ok(snapshot.active_measured_process.as_ref() == Some(selected)
                && snapshot.active_audio_settings.as_ref() == Some(&pending.audio)
                && snapshot.active_transport_latency == Some(pending.transport)
                && snapshot.active_latency_profile == Some(pending.profile)
                && snapshot.graph.pending_revision.is_none()
                && snapshot.graph.active_revision == Some(pending.request.graph_revision)
                && native_running)
        })();
        match outcome {
            Ok(false) => {}
            Ok(true) => {
                self.confirmed = snapshot
                    .active_measured_process
                    .as_ref()
                    .map(|s| (snapshot.generation, s.selection_id));
                self.pending = None;
                self.error = None;
                self.message = Some(
                    t!("Measured budget is active; physical latency remains unqualified.").into(),
                );
            }
            Err(error) => {
                self.pending = None;
                self.message = None;
                self.error = Some(error);
            }
        }
    }
}
