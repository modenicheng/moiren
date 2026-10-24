//! Portable intent checks precede resource probes and native owner creation.
use super::selectors;
use moiren_engine::{
    boundary::{CaptureIntent, ClockRole},
    host::{BackendError, NodeConfig},
    node::{DeviceMode, InputSource, OutputTarget, VirtualEndpointId},
};

pub(super) fn virtual_endpoint_unavailable(
    direction: &str,
    endpoint_id: VirtualEndpointId,
) -> BackendError {
    BackendError::ResourceUnavailable(format!(
        "virtual {direction} endpoint {endpoint_id}: virtual audio transport is not integrated"
    ))
}

pub fn validate_native_project(
    project: &moiren_engine::host::EngineProject,
) -> Result<(), BackendError> {
    validate_with_virtual(project, false).map(|_| ())
}

pub(super) fn validate_with_virtual(
    project: &moiren_engine::host::EngineProject,
    prepared_provider: bool,
) -> Result<crate::virtual_owner::ProjectClock, BackendError> {
    // Reject virtual intent before physical clock/format validation or resource
    // probing. Driver control availability alone does not provide PCM transport.
    for config in project.nodes.values().filter(|_| !prepared_provider) {
        match config {
            NodeConfig::Input(input) => {
                if let InputSource::VirtualPlayback { endpoint_id } = input.source {
                    return Err(virtual_endpoint_unavailable("playback", endpoint_id));
                }
            }
            NodeConfig::Output(output) => {
                if let OutputTarget::VirtualMicrophone { endpoint_id } = output.target {
                    return Err(virtual_endpoint_unavailable("microphone", endpoint_id));
                }
            }
            _ => {}
        }
    }
    let output_count = project
        .nodes
        .values()
        .filter(|node| matches!(node, NodeConfig::Output(_)))
        .count();
    let has_virtual_boundary = project.nodes.values().any(|node| {
        matches!(node, NodeConfig::Input(input) if matches!(input.source, InputSource::VirtualPlayback { .. }))
            || matches!(node, NodeConfig::Output(output) if matches!(output.target, OutputTarget::VirtualMicrophone { .. }))
    });
    let supported_engine_rate = moiren_engine::audio_config::SAMPLE_RATES
        .iter()
        .any(|&rate| f64::from(rate) == project.processing.engine.processing_sr);
    if output_count == 0
        || (!has_virtual_boundary && project.processing.engine.processing_sr != 48_000.0)
        || (has_virtual_boundary && !supported_engine_rate)
    {
        return Err(BackendError::Unsupported(
            "Windows owner requires at least one supported output and a supported Engine rate; physical-only graphs require 48kHz".into(),
        ));
    }
    let physical: Vec<_> = project
        .nodes
        .iter()
        .filter_map(|(&node, config)| {
            if let NodeConfig::Output(output) = config
                && matches!(output.target, OutputTarget::Physical(_))
            {
                Some((node, output.device.clock_role))
            } else {
                None
            }
        })
        .collect();
    if !physical.is_empty() && project.processing.engine.processing_sr != 48_000.0 {
        return Err(BackendError::Unsupported(
            "Windows Shared render requires a 48kHz Engine; use the professional backend for other Engine rates".into(),
        ));
    }
    let clock = if physical.is_empty() {
        crate::virtual_owner::ProjectClock::VirtualRealtime
    } else {
        let masters: Vec<_> = physical
            .iter()
            .filter(|(_, role)| *role == ClockRole::Master)
            .collect();
        if masters.len() != 1 {
            return Err(BackendError::Unsupported(
                "native outputs require exactly one explicit physical master clock".into(),
            ));
        }
        crate::virtual_owner::ProjectClock::Physical(masters[0].0)
    };
    let mut endpoints = Vec::new();
    let mut processes = Vec::new();
    let mut virtual_ids = Vec::new();
    for config in project.nodes.values() {
        let endpoint = match config {
            NodeConfig::Input(input) => match &input.source {
                InputSource::Physical(endpoint) => Some((true, endpoint)),
                InputSource::Application { executable, .. } => {
                    let identity = selectors::process(executable)?;
                    if processes.contains(&identity) {
                        return Err(BackendError::Unsupported(
                            "duplicate process input ownership".into(),
                        ));
                    }
                    processes.push(identity);
                    None
                }
                _ => None,
            },
            NodeConfig::Output(output) => match &output.target {
                OutputTarget::Physical(endpoint) => Some((false, endpoint)),
                _ => None,
            },
            _ => None,
        };
        if let Some(endpoint) = endpoint {
            if endpoints.contains(&endpoint) {
                return Err(BackendError::Unsupported(
                    "duplicate native endpoint ownership".into(),
                ));
            }
            endpoints.push(endpoint);
        }
        let (channels, device) = match config {
            NodeConfig::Input(input) => (input.channels, input.device),
            NodeConfig::Output(output) => (output.channels, output.device),
            _ => continue,
        };
        let virtual_id = match config {
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
        if let Some(id) = virtual_id {
            if !prepared_provider {
                return Err(virtual_endpoint_unavailable("unprepared", id));
            }
            if virtual_ids.contains(&id) {
                return Err(BackendError::Unsupported(
                    "duplicate virtual endpoint ownership".into(),
                ));
            }
            virtual_ids.push(id);
            if !(1..=2).contains(&channels)
                || device.clock_role != ClockRole::Follower
                || device
                    .hardware_sample_rate
                    .is_some_and(|rate| rate != 48_000)
                || device.period_frames.is_some()
            {
                return Err(BackendError::Unsupported("virtual boundaries require 48kHz mono/stereo followers without a hardware period".into()));
            }
            // The global physical backend/mode is orthogonal to virtual resource
            // intent; it never creates an AudioBackend::Vad or a second clock.
            continue;
        }
        if channels != 2 || device.mode != DeviceMode::Shared {
            return Err(BackendError::Unsupported(
                "Windows owner supports Shared stereo graph IO".into(),
            ));
        }
        super::profile::device_period(device)?;
        if matches!(config, NodeConfig::Input(_)) && device.clock_role != ClockRole::Follower {
            return Err(BackendError::Unsupported(
                "render is the master clock and capture inputs must follow it".into(),
            ));
        }
        match config {
            NodeConfig::Input(input) => match &input.source {
                InputSource::Software { name } => {
                    selectors::tone(name)?;
                    if device.hardware_sample_rate.is_some() || device.period_frames.is_some() {
                        return Err(BackendError::Unsupported(
                            "software tone has no hardware rate or device period".into(),
                        ));
                    }
                }
                InputSource::Application { executable, intent } => {
                    selectors::process(executable)?;
                    if *intent != CaptureIntent::PassiveTap
                        || device.period_frames.is_some()
                        || device
                            .hardware_sample_rate
                            .is_some_and(|rate| rate != 48_000)
                    {
                        return Err(BackendError::Unsupported(
                            "process capture uses a fixed 48kHz passive Shared stream".into(),
                        ));
                    }
                }
                InputSource::Physical(_) => {}
                InputSource::VirtualPlayback { .. } => unreachable!("virtual IO validated above"),
            },
            NodeConfig::Output(output) if !matches!(output.target, OutputTarget::Physical(_)) => {
                return Err(BackendError::Unsupported(
                    "output requires a physical Shared endpoint".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(clock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::{
        host::EngineProject,
        node::{DeviceOptions, EndpointSelector, InputConfig, OutputConfig},
    };

    #[test]
    fn virtual_resources_remain_unavailable_after_persistent_identity_round_trip() {
        let endpoint_id = VirtualEndpointId::from_bytes([
            0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76,
            0x54, 0x32,
        ]);
        let json = serde_json::to_string(&endpoint_id).unwrap();
        assert_eq!(json, "\"000123456789abcdeffedcba98765432\"");
        let restored: VirtualEndpointId = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.to_bytes(), endpoint_id.to_bytes());
        assert!(serde_json::from_str::<VirtualEndpointId>("42").is_err());
        assert!(serde_json::from_str::<VirtualEndpointId>("[0, 1]").is_err());

        for channels in [1, 2] {
            for (kind, direction, config) in [
                (
                    NodeKind::Source,
                    "playback",
                    NodeConfig::Input(InputConfig {
                        source: InputSource::VirtualPlayback {
                            endpoint_id: restored,
                        },
                        channels,
                        device: Default::default(),
                    }),
                ),
                (
                    NodeKind::Sink,
                    "microphone",
                    NodeConfig::Output(OutputConfig {
                        target: OutputTarget::VirtualMicrophone {
                            endpoint_id: restored,
                        },
                        channels,
                        device: Default::default(),
                    }),
                ),
            ] {
                let mut project = EngineProject::default();
                let node = project.graph.create_node(kind, channels).unwrap();
                project.nodes.insert(node, config);
                project.validate().unwrap();
                let error = validate_native_project(&project).unwrap_err();
                assert!(
                    matches!(error, BackendError::ResourceUnavailable(ref message)
                        if message.contains(direction)
                            && message.contains(&restored.to_string())
                            && message.contains("not integrated")),
                    "{error:?}"
                );
                // The unsupported resource stays explicit even alongside a
                // physical master. This function never opens that endpoint.
                let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
                project
                    .nodes
                    .insert(sink, output("unopened", ClockRole::Master));
                assert_eq!(validate_native_project(&project), Err(error));
            }
        }
    }

    fn output(name: &str, role: ClockRole) -> NodeConfig {
        let mut output = crate::engine_backend::physical_output(EndpointSelector::PinnedEndpoint {
            endpoint_id: name.into(),
            stable_id: None,
        });
        output.device.clock_role = role;
        NodeConfig::Output(output)
    }

    fn virtual_project(
        input_id: VirtualEndpointId,
        output_id: VirtualEndpointId,
    ) -> (
        EngineProject,
        moiren_core::graph::NodeId,
        moiren_core::graph::NodeId,
    ) {
        virtual_project_with_channels(input_id, output_id, 2)
    }

    fn virtual_project_with_channels(
        input_id: VirtualEndpointId,
        output_id: VirtualEndpointId,
        channels: usize,
    ) -> (
        EngineProject,
        moiren_core::graph::NodeId,
        moiren_core::graph::NodeId,
    ) {
        let mut project = EngineProject::default();
        let source = project
            .graph
            .create_node(NodeKind::Source, channels)
            .unwrap();
        let sink = project.graph.create_node(NodeKind::Sink, channels).unwrap();
        project
            .graph
            .connect(
                project.graph.get_node(source).unwrap().outputs()[0].id(),
                project.graph.get_node(sink).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
        project.nodes.insert(
            source,
            NodeConfig::Input(InputConfig {
                source: InputSource::VirtualPlayback {
                    endpoint_id: input_id,
                },
                channels,
                device: Default::default(),
            }),
        );
        project.nodes.insert(
            sink,
            NodeConfig::Output(OutputConfig {
                target: OutputTarget::VirtualMicrophone {
                    endpoint_id: output_id,
                },
                channels,
                device: Default::default(),
            }),
        );
        (project, source, sink)
    }

    #[test]
    fn prepared_virtual_graph_selects_one_realtime_owner_clock() {
        let (project, _, _) = virtual_project(
            VirtualEndpointId::from_bytes([1; 16]),
            VirtualEndpointId::from_bytes([2; 16]),
        );
        project.validate().unwrap();
        assert_eq!(
            validate_with_virtual(&project, true),
            Ok(crate::virtual_owner::ProjectClock::VirtualRealtime)
        );
    }

    #[test]
    fn mixed_graph_keeps_the_explicit_physical_master_and_rejects_master_loss() {
        let (mut project, source, virtual_sink) = virtual_project(
            VirtualEndpointId::from_bytes([3; 16]),
            VirtualEndpointId::from_bytes([4; 16]),
        );
        let physical_sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        project
            .graph
            .connect(
                project.graph.get_node(source).unwrap().outputs()[0].id(),
                project.graph.get_node(physical_sink).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
        project
            .nodes
            .insert(physical_sink, output("physical-master", ClockRole::Master));
        assert_eq!(
            validate_with_virtual(&project, true),
            Ok(crate::virtual_owner::ProjectClock::Physical(physical_sink))
        );

        project.nodes.insert(
            physical_sink,
            output("physical-follower", ClockRole::Follower),
        );
        assert!(matches!(
            validate_with_virtual(&project, true),
            Err(BackendError::Unsupported(message))
                if message.contains("exactly one explicit physical master")
        ));
        assert!(project.nodes.contains_key(&virtual_sink));
    }

    #[test]
    fn virtual_endpoint_ids_cannot_be_shared_between_graph_boundaries() {
        let id = VirtualEndpointId::from_bytes([5; 16]);
        let (project, _, _) = virtual_project(id, id);
        assert!(matches!(
            validate_with_virtual(&project, true),
            Err(BackendError::Unsupported(message))
                if message.contains("duplicate virtual endpoint ownership")
        ));
    }

    #[test]
    fn virtual_mono_is_admitted_but_physical_mono_is_not() {
        let (virtual_project, _, _) = virtual_project_with_channels(
            VirtualEndpointId::from_bytes([6; 16]),
            VirtualEndpointId::from_bytes([7; 16]),
            1,
        );
        virtual_project.validate().unwrap();
        assert_eq!(
            validate_with_virtual(&virtual_project, true),
            Ok(crate::virtual_owner::ProjectClock::VirtualRealtime)
        );

        let mut physical = EngineProject::default();
        let physical_sink = physical.graph.create_node(NodeKind::Sink, 1).unwrap();
        physical.nodes.insert(
            physical_sink,
            NodeConfig::Output(OutputConfig {
                target: OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
                    endpoint_id: "mono-physical".into(),
                    stable_id: None,
                }),
                channels: 1,
                device: DeviceOptions {
                    clock_role: ClockRole::Master,
                    ..Default::default()
                },
            }),
        );
        assert!(matches!(
            validate_with_virtual(&physical, true),
            Err(BackendError::Unsupported(message))
                if message.contains("Windows owner supports")
        ));
    }
    #[test]
    fn multiple_outputs_require_one_master_and_distinct_endpoint_ownership() {
        let mut project = EngineProject {
            graph: LogicalGraph::new(),
            ..Default::default()
        };
        let master = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let follower = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        project.nodes.insert(master, output("a", ClockRole::Master));
        project
            .nodes
            .insert(follower, output("b", ClockRole::Follower));
        validate_native_project(&project).unwrap();
        for config in [
            output("b", ClockRole::Master),
            output("a", ClockRole::Follower),
        ] {
            project.nodes.insert(follower, config);
            assert!(matches!(
                validate_native_project(&project),
                Err(BackendError::Unsupported(_))
            ));
        }
        project
            .nodes
            .insert(master, output("a", ClockRole::Follower));
        project
            .nodes
            .insert(follower, output("b", ClockRole::Follower));
        assert!(validate_native_project(&project).is_err());
    }
    #[test]
    fn duplicate_capture_or_process_bindings_are_rejected_before_native_preparation() {
        let mut project = EngineProject::default();
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let a = project.graph.create_node(NodeKind::Source, 2).unwrap();
        let b = project.graph.create_node(NodeKind::Source, 2).unwrap();
        project
            .nodes
            .insert(sink, output("render", ClockRole::Master));
        for input in [
            crate::engine_backend::physical_input(EndpointSelector::PinnedEndpoint {
                endpoint_id: "capture".into(),
                stable_id: None,
            }),
            InputConfig {
                source: InputSource::Application {
                    executable: "moiren:process:12:34".into(),
                    intent: CaptureIntent::PassiveTap,
                },
                channels: 2,
                device: Default::default(),
            },
        ] {
            project.nodes.insert(a, NodeConfig::Input(input.clone()));
            project.nodes.insert(b, NodeConfig::Input(input));
            assert!(validate_native_project(&project).is_err());
        }
    }

    #[test]
    fn physical_mix_rate_requirements_allow_conversion_without_changing_graph_rate() {
        let mut project = EngineProject::default();
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let mut config = output("render", ClockRole::Master);
            if let NodeConfig::Output(output) = &mut config {
                output.device.hardware_sample_rate = Some(rate);
            }
            project.nodes.insert(sink, config);
            let mut input =
                crate::engine_backend::physical_input(EndpointSelector::PinnedEndpoint {
                    endpoint_id: "capture".into(),
                    stable_id: None,
                });
            input.device.hardware_sample_rate = Some(rate);
            project.nodes.insert(source, NodeConfig::Input(input));
            validate_native_project(&project).unwrap();
        }
    }

    #[test]
    fn virtual_boundaries_admit_supported_engine_rates_for_user_space_src() {
        let (mut project, _, _) = virtual_project(
            VirtualEndpointId::from_bytes([0x31; 16]),
            VirtualEndpointId::from_bytes([0x32; 16]),
        );
        for rate in moiren_engine::audio_config::SAMPLE_RATES {
            project.processing.engine.processing_sr = f64::from(rate);
            assert_eq!(
                validate_with_virtual(&project, true),
                Ok(crate::virtual_owner::ProjectClock::VirtualRealtime),
                "virtual graph should admit Engine rate {rate} Hz"
            );
        }
    }

    #[test]
    fn mixed_shared_render_rejects_unsupported_engine_rate_before_preparation() {
        let (mut project, _, _) = virtual_project(
            VirtualEndpointId::from_bytes([0x33; 16]),
            VirtualEndpointId::from_bytes([0x34; 16]),
        );
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        project
            .nodes
            .insert(sink, output("shared-master", ClockRole::Master));
        for rate in moiren_engine::audio_config::SAMPLE_RATES {
            project.processing.engine.processing_sr = f64::from(rate);
            let result = validate_with_virtual(&project, true);
            if rate == 48_000 {
                assert_eq!(
                    result,
                    Ok(crate::virtual_owner::ProjectClock::Physical(sink))
                );
            } else {
                assert!(
                    matches!(result, Err(BackendError::Unsupported(message))
                        if message.contains("professional backend")),
                    "Shared render must not process {rate} Hz at the wrong speed"
                );
            }
        }
    }

    #[test]
    fn render_frame_period_without_rate_is_rejected_before_native_preparation() {
        let mut project = EngineProject::default();
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let mut config = output("render", ClockRole::Master);
        if let NodeConfig::Output(output) = &mut config {
            output.device.period_frames = Some(441);
        }
        project.nodes.insert(sink, config);
        let result = validate_native_project(&project);
        assert!(
            matches!(result, Err(BackendError::Unsupported(ref message))
                if message.contains("period_frames") && message.contains("hardware_sample_rate")),
            "frame intent without a rate must fail clearly, got {result:?}"
        );
    }

    #[test]
    fn capture_frame_period_without_rate_is_rejected_before_native_preparation() {
        let mut project = EngineProject::default();
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
        project
            .nodes
            .insert(sink, output("render", ClockRole::Master));
        let mut input = crate::engine_backend::physical_input(EndpointSelector::PinnedEndpoint {
            endpoint_id: "capture".into(),
            stable_id: None,
        });
        input.device.period_frames = Some(441);
        project.nodes.insert(source, NodeConfig::Input(input));
        let result = validate_native_project(&project);
        assert!(
            matches!(result, Err(BackendError::Unsupported(ref message))
                if message.contains("period_frames") && message.contains("hardware_sample_rate")),
            "frame intent without a rate must fail clearly, got {result:?}"
        );
    }

    #[test]
    fn physical_frame_period_with_explicit_rate_is_admitted() {
        let mut project = EngineProject::default();
        let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
        let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
        for (rate, frames) in [(44_100, 441), (48_000, 480), (96_000, 960)] {
            let mut config = output("render", ClockRole::Master);
            if let NodeConfig::Output(output) = &mut config {
                output.device.hardware_sample_rate = Some(rate);
                output.device.period_frames = Some(frames);
            }
            project.nodes.insert(sink, config);
            let mut input =
                crate::engine_backend::physical_input(EndpointSelector::PinnedEndpoint {
                    endpoint_id: "capture".into(),
                    stable_id: None,
                });
            input.device.hardware_sample_rate = Some(rate);
            input.device.period_frames = Some(frames);
            project.nodes.insert(source, NodeConfig::Input(input));
            validate_native_project(&project).unwrap();
        }
    }
}
