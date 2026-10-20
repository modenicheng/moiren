//! Clock selection and native admission are independent of virtual IO intent.
use super::*;

pub(super) fn validate(
    request: &PrepareRequest<'_>,
    has_provider: bool,
) -> Result<(u32, ProjectClock), BackendError> {
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
    let mut physical_outputs = Vec::new();
    let mut outputs = 0;
    let mut io_count = 0;
    for (&node, config) in &project.nodes {
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
                outputs += 1;
                let virtual_id = match output.target {
                    OutputTarget::VirtualMicrophone { endpoint_id } => Some(endpoint_id),
                    OutputTarget::Physical(_) => {
                        physical_outputs.push((node, output.device.clock_role));
                        None
                    }
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
            || device
                .period_frames
                .is_some_and(|frames| !(64..=4096).contains(&frames) || !frames.is_power_of_two())
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
    if outputs == 0 || io_count > 64 {
        return Err(unsupported(
            "professional backend needs an output and at most 64 IO nodes",
        ));
    }
    let clock = if physical_outputs.is_empty() {
        ProjectClock::VirtualRealtime
    } else {
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
        ProjectClock::Physical(master.0)
    };
    Ok((sr as u32, clock))
}
