//! Provider sessions enter a control-side guard before any fallible binding.
use super::*;
use crate::virtual_owner::{
    VirtualBoundaryRequest, VirtualSessions, bind_microphone_sink, bind_playback_source,
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
            let (source, _) = bind_playback_source(provider, boundary, cancel, &mut sessions)?;
            bindings.bind_source(node, source).map_err(unsupported)?;
        } else {
            let (sink, _) = bind_microphone_sink(provider, boundary, cancel, &mut sessions)?;
            bindings.bind_sink(node, sink).map_err(unsupported)?;
            sinks.push(node);
        }
    }
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }
    Ok((sessions, sinks))
}
