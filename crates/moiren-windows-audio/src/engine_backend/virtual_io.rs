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

pub(super) struct PreparedVirtualIo {
    pub sessions: VirtualSessions,
    pub sinks: Vec<NodeId>,
    pub observations: Vec<VirtualObservation>,
    pub probes: Vec<VirtualProbe>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare<S: ProcessingSample>(
    request: &PrepareRequest<'_>,
    provider: Option<&dyn VirtualBoundaryProvider>,
    rate: u32,
    max_demand_frames: u32,
    buffer: crate::virtual_owner::VirtualBufferOptions,
    bindings: &mut NodeBindings<S>,
    cancel: &CancellationToken,
) -> Result<PreparedVirtualIo, BackendError> {
    let mut sessions = VirtualSessions::default();
    let mut sinks = Vec::new();
    let mut observations = Vec::new();
    let mut probes = Vec::new();
    let config = request.project.processing;
    for (&node, config_node) in &request.project.nodes {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let (endpoint_id, channels, playback, device) = match config_node {
            NodeConfig::Input(input) => match input.source {
                InputSource::VirtualPlayback { endpoint_id } => {
                    (endpoint_id, input.channels, true, input.device)
                }
                _ => continue,
            },
            NodeConfig::Output(output) => match output.target {
                OutputTarget::VirtualMicrophone { endpoint_id } => {
                    (endpoint_id, output.channels, false, output.device)
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
            max_demand_frames,
            buffer,
            audio_byte_budget: config.audio_byte_budget,
            owner_generation: request.owner_generation,
        };
        let probe = if playback {
            let (source, probe) = bind_playback_source(provider, boundary, cancel, &mut sessions)?;
            bindings.bind_source(node, source).map_err(unsupported)?;
            probe
        } else {
            let (sink, probe) = bind_microphone_sink(provider, boundary, cancel, &mut sessions)?;
            bindings.bind_sink(node, sink).map_err(unsupported)?;
            sinks.push(node);
            probe
        };
        probes.push((
            node,
            if playback {
                NativeStreamKind::VirtualPlayback
            } else {
                NativeStreamKind::VirtualMicrophone
            },
            probe,
        ));
        observations.push(virtual_observation(node, playback, device, channels));
    }
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }
    Ok(PreparedVirtualIo {
        sessions,
        sinks,
        observations,
        probes,
    })
}

pub(super) type VirtualObservation = (ProfessionalStreamObservation, Arc<Counters>);
pub(super) type VirtualProbe = (
    NodeId,
    NativeStreamKind,
    Arc<crate::virtual_owner::BoundaryProbe>,
);

fn virtual_observation(
    node: NodeId,
    input: bool,
    requested: DeviceOptions,
    channels: usize,
) -> VirtualObservation {
    (
        ProfessionalStreamObservation {
            node,
            input,
            master: false,
            requested,
            // Virtual boundaries bridge kernel PCM at the native rate with no
            // hardware device buffer; SRC to the engine rate stays internal.
            actual: NativeStreamInfo {
                sample_rate: crate::NATIVE_SAMPLE_RATE,
                format: SampleFormat::Float32,
                buffer_frames: 0,
                channels,
            },
            dropped_frames: 0,
            silent_frames: 0,
            native_callbacks: 0,
            native_wait_wakes: 0,
            native_written_frames: 0,
            native_wait_timeouts: 0,
            native_dropped_frames: 0,
            native_silent_frames: 0,
            driver_latency: None,
        },
        Arc::default(),
    )
}
