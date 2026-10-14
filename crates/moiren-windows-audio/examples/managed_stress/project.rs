use moiren_core::graph::{NodeId, NodeKind, SendParams};
use moiren_engine::{
    host::{EngineProject, NodeConfig},
    node::{DefaultDeviceRole, EndpointSelector},
    processor::CompressorSettings,
};
use moiren_windows_audio::engine_backend::{physical_output, tone_input};

pub struct Project {
    pub project: EngineProject,
    pub edited: NodeId,
    pub parameter: NodeId,
}

pub fn build(processors: usize) -> anyhow::Result<Project> {
    let mut project = EngineProject::default();
    let source = project.graph.create_node(NodeKind::Source, 2)?;
    project
        .nodes
        .insert(source, NodeConfig::Input(tone_input(440.0)?));
    let mut previous = source;
    let mut gain_nodes = Vec::new();
    for index in 0..processors {
        let compressor = index % 4 == 3;
        let kind = if compressor {
            NodeKind::Compressor
        } else {
            NodeKind::Gain
        };
        let node = project.graph.create_node(kind, 2)?;
        let configuration = if compressor {
            NodeConfig::Compressor(CompressorSettings::default())
        } else {
            gain_nodes.push(node);
            NodeConfig::Gain { initial: 1.0 }
        };
        project.nodes.insert(node, configuration);
        connect(&mut project, previous, node)?;
        previous = node;
    }
    let muted = project.graph.create_node(NodeKind::Gain, 2)?;
    project
        .nodes
        .insert(muted, NodeConfig::Gain { initial: 0.0 });
    connect(&mut project, previous, muted)?;
    let output = project.graph.create_node(NodeKind::Sink, 2)?;
    project.nodes.insert(
        output,
        NodeConfig::Output(physical_output(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Multimedia,
        ))),
    );
    connect(&mut project, muted, output)?;
    project.validate()?;
    Ok(Project {
        project,
        edited: gain_nodes[0],
        parameter: gain_nodes[1],
    })
}

fn connect(project: &mut EngineProject, source: NodeId, destination: NodeId) -> anyhow::Result<()> {
    project.graph.connect(
        project.graph.get_node(source)?.outputs()[0].id(),
        project.graph.get_node(destination)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_is_valid_and_output_always_has_a_separate_zero_gain_guard() {
        for processors in [2, 32, 120] {
            let built = build(processors).unwrap();
            assert_eq!(built.project.nodes.len(), processors + 3);
            assert_ne!(built.edited, built.parameter);
            assert_eq!(
                built
                    .project
                    .nodes
                    .values()
                    .filter(
                        |config| matches!(config, NodeConfig::Gain { initial } if *initial == 0.0)
                    )
                    .count(),
                1
            );
        }
    }
}
