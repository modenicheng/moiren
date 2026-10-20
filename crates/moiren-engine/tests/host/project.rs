use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::host::{EngineProject, NodeConfig, ProjectError};

#[test]
fn rack_strip_channel_selection_must_fit_every_output() {
    use moiren_engine::{
        processor::{ChannelMode, StripConfig},
        rack::{RackRoute, RackSettings},
    };
    let mut project = EngineProject::default();
    let rack = project.graph.create_rack(&[2, 1], &[2, 1]).unwrap();
    project.nodes.insert(
        rack,
        NodeConfig::Rack(RackSettings {
            routes: vec![
                RackRoute {
                    input_port: 0,
                    output_port: 0,
                    slots: vec![],
                },
                RackRoute {
                    input_port: 1,
                    output_port: 1,
                    slots: vec![],
                },
            ],
            ..Default::default()
        }),
    );
    project.strips.insert(
        rack,
        StripConfig {
            mode: ChannelMode::Solo(1),
            ..Default::default()
        },
    );
    assert_eq!(
        project.validate(),
        Err(ProjectError::InvalidProcessor(rack))
    );
    project.strips.get_mut(&rack).unwrap().mode = ChannelMode::Solo(0);
    project.validate().unwrap();
}

#[test]
fn project_rejects_missing_configuration_and_kind_mismatch() {
    let mut project = EngineProject::default();
    let node = project.graph.create_node(NodeKind::Gain, 1).unwrap();
    assert_eq!(
        project.validate(),
        Err(ProjectError::MissingConfiguration(node))
    );
    project.nodes.insert(node, NodeConfig::Bus);
    assert_eq!(
        project.validate(),
        Err(ProjectError::ConfigurationKind(node))
    );
    project
        .nodes
        .insert(node, NodeConfig::Gain { initial: 1.0 });
    assert!(project.validate().is_ok());
}

#[test]
fn project_transaction_rolls_back_invalid_candidate() {
    let mut project = EngineProject::default();
    let original = project.clone();
    let result = project.edit(|candidate| {
        candidate.graph.create_node(NodeKind::Gain, 1)?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(project, original);
    assert_eq!(project.graph, LogicalGraph::new());
}

#[test]
fn project_unit_edit_keeps_error_only_callback_type_inference() {
    let mut project = EngineProject::default();
    let result = project.edit(|_| Err(ProjectError::InvalidProcessing));
    assert_eq!(result.unwrap_err(), ProjectError::InvalidProcessing);
    assert_eq!(project, EngineProject::default());
}

#[test]
fn project_parameter_reads_resolve_explicit_values_over_configuration_defaults() {
    use moiren_core::protocol::ParamValue;
    use moiren_engine::{
        host::{DesiredParameter, ParameterTarget, SendParameter},
        processor::Gain,
    };
    let (mut project, _, gain, _) = super::support::project();
    let edge = project.graph.edges()[0].id();
    let node_target = ParameterTarget::Node {
        node: gain,
        parameter: Gain::LEVEL,
    };
    let edge_target = ParameterTarget::Edge {
        edge,
        parameter: SendParameter::Gain,
    };
    project.parameters.extend([
        DesiredParameter {
            target: node_target,
            value: ParamValue::Float(0.25),
        },
        DesiredParameter {
            target: edge_target,
            value: ParamValue::Float(2.0),
        },
    ]);
    project.validate().unwrap();
    let values = project.parameter_values();
    assert_eq!(values.len(), 7);
    assert_eq!(
        values
            .iter()
            .find(|value| value.target == node_target)
            .unwrap()
            .value,
        ParamValue::Float(0.25)
    );
    assert_eq!(
        values
            .iter()
            .find(|value| value.target == edge_target)
            .unwrap()
            .value,
        ParamValue::Float(2.0)
    );
    assert_eq!(project.nodes[&gain], NodeConfig::Gain { initial: 1.0 });
    assert_eq!(project.graph.get_edge(edge).unwrap().params().gain, 1.0);
}

#[test]
fn virtual_identity_survives_project_edits_and_graph_node_replacement() {
    use moiren_engine::node::{
        InputConfig, InputSource, IoConfigError, OutputConfig, OutputTarget, VirtualEndpointId,
    };

    let playback_id = VirtualEndpointId::from_bytes([0x81; 16]);
    let microphone_id = VirtualEndpointId::from_bytes([0x42; 16]);
    for channels in [1, 2] {
        let mut project = EngineProject::default();
        let source = project
            .graph
            .create_node(NodeKind::Source, channels)
            .unwrap();
        let sink = project.graph.create_node(NodeKind::Sink, channels).unwrap();
        let input = NodeConfig::Input(InputConfig {
            source: InputSource::VirtualPlayback {
                endpoint_id: playback_id,
            },
            channels,
            device: Default::default(),
        });
        let output = NodeConfig::Output(OutputConfig {
            target: OutputTarget::VirtualMicrophone {
                endpoint_id: microphone_id,
            },
            channels,
            device: Default::default(),
        });
        project.nodes.insert(source, input.clone());
        project.nodes.insert(sink, output.clone());
        let source_port = project.graph.get_node(source).unwrap().outputs()[0].id();
        let sink_port = project.graph.get_node(sink).unwrap().inputs()[0].id();
        project
            .graph
            .connect(source_port, sink_port, Default::default())
            .unwrap();
        project.validate().unwrap();

        let before = project.clone();
        let error = project
            .edit(|candidate| {
                let NodeConfig::Input(input) = candidate.nodes.get_mut(&source).unwrap() else {
                    unreachable!()
                };
                input.channels = 3;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(
            error,
            ProjectError::Io {
                node: source,
                error: IoConfigError::InvalidVirtualChannels,
            }
        );
        assert_eq!(project, before);
        let error = project
            .edit(|candidate| {
                let NodeConfig::Output(output) = candidate.nodes.get_mut(&sink).unwrap() else {
                    unreachable!()
                };
                output.channels = 3 - channels;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error, ProjectError::IoChannels(sink));
        assert_eq!(project, before);

        let replacement = project
            .edit_with_result(|candidate| {
                candidate.graph.remove_node(source)?;
                let replacement = candidate.graph.create_node(NodeKind::Source, channels)?;
                candidate.nodes.insert(replacement, input.clone());
                let port = candidate.graph.get_node(replacement)?.outputs()[0].id();
                candidate
                    .graph
                    .connect(port, sink_port, Default::default())?;
                Ok(replacement)
            })
            .unwrap();
        assert_ne!(source, replacement);
        assert!(!project.nodes.contains_key(&source));
        assert_eq!(project.nodes[&replacement], input);
        assert_eq!(project.nodes[&sink], output);
    }
}
