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
