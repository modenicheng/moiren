use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::host::{EngineProject, NodeConfig, ProjectError};

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
