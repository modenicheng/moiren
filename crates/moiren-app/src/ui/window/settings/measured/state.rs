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
        });
        self.message =
            Some(t!("Restarting audio; waiting for the measured budget to become active.").into());
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
            let selected = snapshot
                .measured_process
                .as_ref()
                .filter(|s| {
                    s.request == pending.request
                        && Some(s.selection_id) != pending.previous_selection
                })
                .ok_or_else(|| {
                    snapshot.graph.last_error.clone().unwrap_or_else(|| {
                        t!("Measured budget was refused or replaced. Your draft is retained.")
                            .into()
                    })
                })?;
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
                SessionPhase::Idle
                    | SessionPhase::Failed
                    | SessionPhase::Exiting
                    | SessionPhase::Exited
            ) {
                return Err(snapshot.graph.last_error.clone().unwrap_or_else(|| {
                    t!("Audio stopped before the measured budget became active.").into()
                }));
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
