//! Recording admission uses confirmed host state, never an unapplied candidate.
use super::*;
use moiren_core::graph::NodeKind;

impl ManagedDriver {
    pub(super) fn set_recording_format(
        &mut self,
        c: &mut ServiceContext,
        node: NodeId,
        format: RecordingFormat,
    ) {
        if self.recordings.is_busy(node) {
            c.graph.record_error_with_subject(
                GraphSubject::Node(node),
                "Recording is active or finalizing",
            );
            return;
        }
        let graph = c.graph.snapshot();
        if let Err(error) = c.recordings.set_format(&graph, node, format) {
            c.graph
                .record_error_with_subject(GraphSubject::Node(node), &error);
            return;
        }
        // Re-selecting an unchanged terminal format preserves the completed
        // file's status. A changed format is fresh Idle intent and new preflight.
        if c.recordings.snapshots(&graph).iter().any(|entry| {
            entry.node == node && entry.phase == RecordingPhase::Idle && entry.path.is_some()
        }) {
            let channels = graph
                .graph
                .get_node(node)
                .expect("validated recorder")
                .channels();
            if let Err(error) = self.recordings.preflight(
                &mut c.recordings,
                node,
                c.audio_settings.processing.sample_rate,
                channels,
            ) {
                c.graph
                    .record_error_with_subject(GraphSubject::Node(node), &error);
            }
        }
    }

    pub(super) fn start_recording(&mut self, c: &mut ServiceContext, node: NodeId) {
        if c.graph.snapshot().kind_of(node) != Some(NodeKind::Recorder) {
            c.graph.record_error_with_subject(
                GraphSubject::Node(node),
                "Recording requires a Recorder node",
            );
            return;
        }
        if self.recordings.is_busy(node) {
            c.graph.record_error_with_subject(
                GraphSubject::Node(node),
                "Recording is already active or finalizing",
            );
            return;
        }
        // Start supersedes idle preflight even when admission fails before a
        // file worker exists. Cancel its ticket/flag now, not on a later poll;
        // a blocked read-only probe must not keep rejected admission undrained.
        self.recordings.cancel_preflight(&mut c.recordings, node);
        if self.apply || !self.edits.is_empty() || self.auto_apply_deadline.is_some() {
            c.recordings.fail(
                node,
                "Wait for pending graph changes before starting recording",
            );
            return;
        }
        let result = self
            .recording_reader(c, node)
            .and_then(|(reader, rate, channels)| {
                self.recordings
                    .start(&mut c.recordings, node, reader, rate, channels)
            });
        if let Err(error) = result {
            c.recordings.fail(node, error);
        }
    }

    fn recording_reader(
        &self,
        c: &ServiceContext,
        node: NodeId,
    ) -> Result<(host::ManagedRecorderReader, u32, usize), String> {
        let owner = self
            .owner
            .as_ref()
            .ok_or("Start audio and apply the Recorder node before recording")?;
        if self.exiting
            || owner.document != self.document
            || c.core.phase() != SessionPhase::Running
        {
            return Err("Apply this project and start audio before recording".into());
        }
        let snapshot = owner.observer.read();
        if snapshot.state.execution != ExecutionState::Running {
            return Err("The audio owner is not running".into());
        }
        if snapshot.state.active_revision != Some(snapshot.state.desired_revision) {
            return Err("Wait for the audio graph to finish applying before recording".into());
        }
        let active = snapshot.active.ok_or("The audio graph is not confirmed")?;
        if !matches!(active.nodes.get(&node), Some(host::NodeConfig::Recorder(_))) {
            return Err("Apply the Recorder node before recording".into());
        }
        let channels = active
            .graph
            .get_node(node)
            .map_err(|error| error.to_string())?
            .channels();
        let rate = active.processing.engine.processing_sr;
        if !rate.is_finite() || rate < 1.0 || rate > f64::from(u32::MAX) || rate.fract() != 0.0 {
            return Err("The processing sample rate cannot be represented in a WAV header".into());
        }
        let reader = owner.observer.recorder_reader(node)?;
        if reader.owner_generation() != snapshot.state.owner_generation
            || reader.channel_count() != Some(channels)
        {
            return Err(
                "The Recorder endpoint changed or is already leased; retry after Apply".into(),
            );
        }
        Ok((reader, rate as u32, channels))
    }

    pub(super) fn poll_recordings(&mut self, c: &mut ServiceContext) {
        // Resource loss can leave a retained logical owner with no active audio.
        // Do not keep claiming its endpoint while waiting for a device to return.
        if c.core.phase() != SessionPhase::Running {
            self.recordings.stop_all(&mut c.recordings);
        }
        self.recordings
            .reconcile(&mut c.recordings, &c.graph.snapshot());
        if !self.exiting {
            self.recordings.refresh_preflights(
                &mut c.recordings,
                &c.graph.snapshot(),
                c.audio_settings.processing.sample_rate,
            );
        }
        for error in self.recordings.poll(&mut c.recordings) {
            // Removed/replaced nodes have no current node card, but finalization
            // failures must still reach the application's visible diagnostics.
            c.graph.record_error(error);
        }
    }
}
