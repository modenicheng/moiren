//! Provider sessions enter a control-side guard before any fallible binding.
use super::*;
use crate::virtual_owner::{
    VirtualBoundaryRequest, VirtualMicrophoneSink, VirtualPlaybackSource, VirtualSessions,
};
use moiren_engine::host::OwnerStatus;

pub(super) struct VirtualAuxiliary(pub VirtualSessions);

impl AuxiliaryOwner for VirtualAuxiliary {
    fn activate(&self) -> Result<(), BackendError> {
        self.0.activate()
    }
    fn cancel(&self) {
        self.0.cancel();
    }
    fn running(&self) -> bool {
        self.0.status() == OwnerStatus::Running
    }
    fn finished(&self) -> bool {
        matches!(self.0.status(), OwnerStatus::Failed(_))
    }
    fn status(&self) -> OwnerStatus {
        self.0.status()
    }
    fn join(&mut self) -> Result<(), BackendError> {
        self.0.stop_and_join()
    }
}

pub(super) fn prepare<S: ProcessingSample>(
    request: &PrepareRequest<'_>,
    provider: Option<&dyn VirtualBoundaryProvider>,
    rate: u32,
    bindings: &mut NodeBindings<S>,
    cancel: &CancellationToken,
) -> Result<(VirtualSessions, Vec<NodeId>), BackendError> {
    let mut sessions = VirtualSessions::default();
    let mut sinks = Vec::new();
    let config = request.project.processing;
    let physical_master = request.project.nodes.values().any(|node| {
        matches!(node, NodeConfig::Output(output) if matches!(output.target, OutputTarget::Physical(_)))
    });
    for (&node, config_node) in &request.project.nodes {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let (endpoint_id, channels, playback) = match config_node {
            NodeConfig::Input(input) => match input.source {
                InputSource::VirtualPlayback { endpoint_id } => (endpoint_id, input.channels, true),
                _ => continue,
            },
            NodeConfig::Output(output) => match output.target {
                OutputTarget::VirtualMicrophone { endpoint_id } => {
                    (endpoint_id, output.channels, false)
                }
                _ => continue,
            },
            _ => continue,
        };
        let provider = provider.expect("virtual provider validated before resource preparation");
        let boundary = VirtualBoundaryRequest {
            endpoint_id,
            channels,
            engine_sample_rate: rate,
            max_block_frames: config.engine.max_block_frames,
            audio_byte_budget: config.audio_byte_budget,
            owner_generation: request.owner_generation,
        };
        if playback {
            let prepared = provider.prepare_playback(boundary, cancel)?;
            sessions.push_with_reset_epoch(prepared.session, prepared.reset_epoch.clone());
            let source = VirtualPlaybackSource::new(
                prepared.carrier,
                channels,
                boundary.max_block_frames,
                rate,
                boundary.audio_byte_budget,
            )?
            .with_reset_fence(
                prepared.reset_epoch,
                prepared.producer_quiesced_epoch,
                prepared.consumer_quiesced_epoch,
            );
            let source = if physical_master {
                source.with_clock_drift()?
            } else {
                source
            };
            bindings.bind_source(node, source).map_err(unsupported)?;
        } else {
            let prepared = provider.prepare_microphone(boundary, cancel)?;
            sessions.push_with_reset_epoch(prepared.session, prepared.reset_epoch.clone());
            let sink = VirtualMicrophoneSink::new(
                prepared.carrier,
                channels,
                boundary.max_block_frames,
                rate,
                boundary.audio_byte_budget,
            )?
            .with_reset_fence(
                prepared.reset_epoch,
                prepared.producer_quiesced_epoch,
                prepared.consumer_quiesced_epoch,
            );
            let sink = if physical_master {
                sink.with_clock_drift()?
            } else {
                sink
            };
            bindings.bind_sink(node, sink).map_err(unsupported)?;
            sinks.push(node);
        }
    }
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }
    Ok((sessions, sinks))
}
