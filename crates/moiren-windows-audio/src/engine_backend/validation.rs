//! Portable intent checks precede resource probes and native owner creation.
use super::selectors;
use moiren_engine::{
    boundary::{CaptureIntent, ClockRole},
    host::{BackendError, NodeConfig},
    node::{DeviceMode, InputSource, OutputTarget},
};

pub fn validate_native_project(
    project: &moiren_engine::host::EngineProject,
) -> Result<(), BackendError> {
    if project.processing.engine.processing_sr != 48_000.0
        || project
            .nodes
            .values()
            .filter(|node| matches!(node, NodeConfig::Output(_)))
            .count()
            == 0
    {
        return Err(BackendError::Unsupported(
            "Windows owner requires 48kHz and at least one stereo output".into(),
        ));
    }
    if project.nodes.values().filter(|config| matches!(config, NodeConfig::Output(output) if output.device.clock_role == ClockRole::Master)).count() != 1 { return Err(BackendError::Unsupported("native outputs require exactly one master clock".into())); }
    let mut endpoints = Vec::new();
    let mut processes = Vec::new();
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
        if channels != 2 || device.mode != DeviceMode::Shared {
            return Err(BackendError::Unsupported(
                "Windows owner supports Shared stereo graph IO".into(),
            ));
        }
        device
            .validate()
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        super::profile::device_period(device)
            .validate()
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
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
            },
            NodeConfig::Output(output) if !matches!(output.target, OutputTarget::Physical(_)) => {
                return Err(BackendError::Unsupported(
                    "output requires a physical Shared endpoint".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::{
        host::EngineProject,
        node::{EndpointSelector, InputConfig},
    };
    fn output(name: &str, role: ClockRole) -> NodeConfig {
        let mut output = crate::engine_backend::physical_output(EndpointSelector::PinnedEndpoint {
            endpoint_id: name.into(),
            stable_id: None,
        });
        output.device.clock_role = role;
        NodeConfig::Output(output)
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
}
