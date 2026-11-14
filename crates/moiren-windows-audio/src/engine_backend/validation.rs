//! Clock selection and native admission are independent of virtual IO intent.
use super::*;

/// Admission checks that run before endpoints are resolved. Returns the
/// processing rate; the clock follows in [`select_clock`] once absence is
/// known.
pub(super) fn validate(
    request: &PrepareRequest<'_>,
    has_provider: bool,
) -> Result<u32, BackendError> {
    let project = request.project;
    project.validate().map_err(unsupported)?;
    if !has_provider {
        for config in project.nodes.values() {
            let id = match config {
                NodeConfig::Input(input) => match input.source {
                    InputSource::VirtualPlayback { endpoint_id } => Some(endpoint_id),
                    _ => None,
                },
                NodeConfig::Output(output) => match output.target {
                    OutputTarget::VirtualMicrophone { endpoint_id } => Some(endpoint_id),
                    _ => None,
                },
                _ => None,
            };
            if let Some(id) = id {
                return Err(BackendError::ResourceUnavailable(format!(
                    "virtual endpoint {id} requires an opt-in virtual boundary provider"
                )));
            }
        }
    }
    let mut virtual_ids = Vec::new();
    let mut io_count = 0;
    for config in project.nodes.values() {
        let (channels, device, virtual_id) = match config {
            NodeConfig::Input(input) => {
                if input.device.clock_role != ClockRole::Follower {
                    return Err(unsupported("inputs must follow the output master"));
                }
                (
                    input.channels,
                    input.device,
                    match input.source {
                        InputSource::VirtualPlayback { endpoint_id } => Some(endpoint_id),
                        _ => None,
                    },
                )
            }
            NodeConfig::Output(output) => {
                let virtual_id = match output.target {
                    OutputTarget::VirtualMicrophone { endpoint_id } => Some(endpoint_id),
                    OutputTarget::Physical(_) => None,
                    _ => {
                        return Err(unsupported(
                            "professional outputs need physical or virtual targets",
                        ));
                    }
                };
                (output.channels, output.device, virtual_id)
            }
            _ => continue,
        };
        io_count += 1;
        if let Some(id) = virtual_id {
            if virtual_ids.contains(&id) {
                return Err(unsupported("duplicate virtual endpoint ownership"));
            }
            virtual_ids.push(id);
            if !(1..=2).contains(&channels)
                || device.clock_role != ClockRole::Follower
                || device
                    .hardware_sample_rate
                    .is_some_and(|rate| rate != 48_000)
                || device.period_frames.is_some()
            {
                return Err(unsupported(
                    "virtual boundaries require 48kHz mono/stereo followers without a hardware period",
                ));
            }
            // Backend, encoding and mode describe physical devices. The virtual
            // carrier stays f32/48kHz even in f64 ASIO/Exclusive projects.
            continue;
        }
        if channels != 2 {
            return Err(unsupported("professional graph IO is stereo"));
        }
        if device
            .hardware_sample_rate
            .is_some_and(|rate| !SAMPLE_RATES.contains(&rate))
            || !crate::wasapi::native_period_request_supported(
                device,
                matches!(config, NodeConfig::Input(_)),
            )
        {
            return Err(unsupported(
                "unsupported requested native rate or buffer size",
            ));
        }
    }
    let sr = project.processing.engine.processing_sr;
    if !SAMPLE_RATES.iter().any(|&rate| f64::from(rate) == sr) {
        return Err(unsupported(
            "professional Engine rate must be 44100/48000/88200/96000 Hz",
        ));
    }
    if !(1..=4096).contains(&project.processing.engine.max_block_frames) {
        return Err(unsupported(
            "professional Engine block must be 1..4096 frames",
        ));
    }
    if io_count > 64 {
        return Err(unsupported(
            "professional backend allows at most 64 IO nodes",
        ));
    }
    Ok(sr as u32)
}

/// Picks the project clock once endpoint presence is known. Exactly one
/// master is required among all physical outputs (a configuration fact), but
/// an absent master degrades the whole project to the QPC virtual clock —
/// the present outputs keep rendering as followers while the device is gone.
pub(super) fn select_clock(
    request: &PrepareRequest<'_>,
    absent: &[NodeId],
) -> Result<ProjectClock, BackendError> {
    let mut physical_outputs = Vec::new();
    for (&node, config) in &request.project.nodes {
        if let NodeConfig::Output(output) = config
            && matches!(output.target, OutputTarget::Physical(_))
        {
            physical_outputs.push((node, output.device.clock_role));
        }
    }
    if physical_outputs.is_empty() {
        return Ok(ProjectClock::VirtualRealtime);
    }
    let mut masters = physical_outputs
        .iter()
        .filter(|(_, role)| *role == ClockRole::Master);
    let master = masters.next().ok_or_else(|| {
        unsupported("professional backend needs exactly one physical output master")
    })?;
    if masters.next().is_some() {
        return Err(unsupported(
            "professional backend needs exactly one physical output master",
        ));
    }
    if absent.contains(&master.0) {
        return Ok(ProjectClock::VirtualRealtime);
    }
    Ok(ProjectClock::Physical(master.0))
}
