//! Portable route intent and exact IO/Bus topology; never opens native resources.
use anyhow::{Context, Result, ensure};
#[cfg(any(windows, test))]
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};
use moiren_engine::node::VirtualEndpointId;
#[cfg(any(windows, test))]
use moiren_engine::{
    boundary::ClockRole,
    host::{EngineProject, NodeConfig},
    node::{DeviceOptions, InputConfig, InputSource, OutputConfig, OutputTarget},
};
use moiren_windows_audio::vad::{Channels, Intent, StableEndpointId, create_request};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RouteKind {
    #[default]
    Physical,
    Microphone,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct MicrophoneTarget {
    pub endpoint_id: VirtualEndpointId,
    pub name: String,
}

pub(super) fn microphone_target(id: &str, name: &str, playback: &str) -> Result<MicrophoneTarget> {
    let endpoint_id = VirtualEndpointId::from_str(id).context("VAD microphone endpoint ID")?;
    let playback = VirtualEndpointId::from_str(playback).context("VAD playback endpoint ID")?;
    ensure!(
        endpoint_id != playback,
        "VAD playback and microphone endpoint IDs must differ"
    );
    // Share CREATE's exact UTF-16/control-character rules without native IO.
    create_request(
        StableEndpointId::from_virtual_endpoint_id(endpoint_id),
        Intent::Microphone,
        Channels::Stereo,
        name,
        false,
    )
    .context("VAD microphone endpoint name")?;
    Ok(MicrophoneTarget {
        endpoint_id,
        name: name.to_owned(),
    })
}

#[cfg(any(windows, test))]
pub(super) fn minimal_project(
    playback: VirtualEndpointId,
    target: OutputTarget,
) -> Result<EngineProject> {
    let clock_role = match &target {
        OutputTarget::Physical(_) => ClockRole::Master,
        OutputTarget::VirtualMicrophone { endpoint_id } => {
            ensure!(
                *endpoint_id != playback,
                "VAD playback and microphone endpoint IDs must differ"
            );
            ClockRole::Follower
        }
        OutputTarget::Software { .. } => {
            anyhow::bail!("VAD route requires physical or microphone output")
        }
    };
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2)?;
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    let source_port = graph.get_node(source)?.outputs()[0].id();
    let bus_port = graph.get_node(bus)?.outputs()[0].id();
    let sink_port = graph.get_node(sink)?.inputs()[0].id();
    connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
    graph.connect(bus_port, sink_port, SendParams::default())?;
    let mut project = EngineProject {
        graph,
        ..EngineProject::default()
    };
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::VirtualPlayback {
                endpoint_id: playback,
            },
            channels: 2,
            device: DeviceOptions::default(),
        }),
    );
    project.nodes.insert(bus, NodeConfig::Bus);
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target,
            channels: 2,
            device: DeviceOptions {
                clock_role,
                ..DeviceOptions::default()
            },
        }),
    );
    project
        .validate()
        .context("validate VAD Source -> Bus -> Sink route")?;
    Ok(project)
}

#[cfg(any(windows, test))]
pub(super) fn apply_physical_render_request(
    project: &mut EngineProject,
    request: super::render_request::PhysicalRenderRequest,
) -> Result<Option<moiren_core::graph::NodeId>> {
    let mut physical = project
        .nodes
        .iter()
        .filter_map(|(node, config)| match config {
            NodeConfig::Output(output) if matches!(output.target, OutputTarget::Physical(_)) => {
                Some(*node)
            }
            _ => None,
        });
    let output = physical.next();
    ensure!(
        physical.next().is_none(),
        "VAD route has ambiguous physical outputs"
    );
    let Some(node) = output else {
        ensure!(
            request.mode.is_none() && request.period_frames.is_none(),
            "physical render request cannot apply to virtual microphone output"
        );
        return Ok(None);
    };
    let NodeConfig::Output(output) = project.nodes.get_mut(&node).expect("selected output") else {
        unreachable!("selected physical output");
    };
    ensure!(
        output.device.clock_role == ClockRole::Master,
        "VAD physical output must remain the Master clock"
    );
    output.device = request.apply(output.device);
    Ok(Some(node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::NodeKind;
    use moiren_engine::{
        boundary::ClockRole,
        host::NodeConfig,
        node::{DefaultDeviceRole, EndpointSelector},
    };
    use moiren_windows_audio::vad::{Channels, Intent, StableEndpointId, create_request};

    const PLAYBACK: &str = "0123456789abcdef0123456789abcdef";
    const MICROPHONE: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn physical_render_project_applies_only_to_physical_master_and_keeps_defaults() {
        use super::super::render_request::PhysicalRenderRequest;
        use moiren_engine::{audio_config::SampleFormat, node::DeviceMode};
        let target = OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
            endpoint_id: "selected render".into(),
            stable_id: Some("selected stable".into()),
        });
        for mode in [DeviceMode::Shared, DeviceMode::Exclusive] {
            let mut project = minimal_project(PLAYBACK.parse().unwrap(), target.clone()).unwrap();
            let original = project.nodes.clone();
            let expected = apply_physical_render_request(
                &mut project,
                PhysicalRenderRequest {
                    mode: Some(mode),
                    period_frames: Some(128),
                },
            )
            .unwrap()
            .unwrap();
            for (node, config) in &project.nodes {
                if *node == expected {
                    let NodeConfig::Output(output) = config else {
                        panic!("physical sink missing")
                    };
                    assert_eq!(output.target, target);
                    assert_eq!(output.device.mode, mode);
                    assert_eq!(output.device.period_frames, Some(128));
                    assert_eq!(output.device.clock_role, ClockRole::Master);
                    assert_eq!(output.device.sample_format, SampleFormat::Native);
                    assert!(output.device.hardware_sample_rate.is_none());
                } else {
                    assert_eq!(config, &original[node]);
                }
            }
            let mut omitted = minimal_project(PLAYBACK.parse().unwrap(), target.clone()).unwrap();
            let before = omitted.nodes.clone();
            assert!(
                apply_physical_render_request(&mut omitted, PhysicalRenderRequest::default())
                    .unwrap()
                    .is_some()
            );
            assert_eq!(omitted.nodes, before);
        }
    }

    #[test]
    fn physical_render_project_leaves_virtual_route_unchanged_and_refuses_explicit_intent() {
        use super::super::render_request::PhysicalRenderRequest;
        let mut project = minimal_project(
            PLAYBACK.parse().unwrap(),
            OutputTarget::VirtualMicrophone {
                endpoint_id: MICROPHONE.parse().unwrap(),
            },
        )
        .unwrap();
        let original = project.nodes.clone();
        assert!(
            apply_physical_render_request(&mut project, PhysicalRenderRequest::default())
                .unwrap()
                .is_none()
        );
        assert_eq!(project.nodes, original);
        assert!(
            apply_physical_render_request(
                &mut project,
                PhysicalRenderRequest {
                    period_frames: Some(128),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(project.nodes, original);
    }

    #[test]
    fn route_kind_defaults_physical_and_uses_exact_snake_case_json() {
        assert_eq!(RouteKind::default(), RouteKind::Physical);
        for (route, text) in [
            (RouteKind::Physical, "\"physical\""),
            (RouteKind::Microphone, "\"microphone\""),
        ] {
            let copied = route;
            assert_eq!(route, copied);
            assert_eq!(serde_json::to_string(&route).unwrap(), text);
            assert_eq!(serde_json::from_str::<RouteKind>(text).unwrap(), route);
        }
        for text in ["\"Physical\"", "\"software\"", "null", "0"] {
            assert!(serde_json::from_str::<RouteKind>(text).is_err());
        }
    }

    #[test]
    fn microphone_target_parses_both_ids_and_preserves_name_and_serialized_identity() {
        let name = "  Exact microphone  ";
        let target = microphone_target(&MICROPHONE.to_uppercase(), name, PLAYBACK).unwrap();
        assert_eq!(target.endpoint_id, MICROPHONE.parse().unwrap());
        assert_eq!(target.name, name);
        let cloned = target.clone();
        assert_eq!(cloned.endpoint_id, target.endpoint_id);
        assert_eq!(cloned.name, target.name);
        let value = serde_json::to_value(&target).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"endpoint_id": MICROPHONE, "name": name})
        );
    }

    #[test]
    fn microphone_target_rejects_non_exact_hex_in_either_id() {
        for invalid in [
            "",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "g123456789abcdef0123456789abcdef",
            " 0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef ",
            "01234567-89ab-cdef-0123-456789abcdef",
        ] {
            assert!(microphone_target(invalid, "Microphone", PLAYBACK).is_err());
            assert!(microphone_target(MICROPHONE, "Microphone", invalid).is_err());
        }
    }

    #[test]
    fn microphone_target_rejects_equal_ids_after_case_insensitive_parse() {
        assert!(microphone_target(PLAYBACK, "Microphone", PLAYBACK).is_err());
        assert!(microphone_target(&PLAYBACK.to_uppercase(), "Microphone", PLAYBACK).is_err());
    }

    #[test]
    fn zero_id_retains_native_parser_semantics_but_still_must_be_distinct() {
        let zero = "00000000000000000000000000000000";
        assert_eq!(
            microphone_target(zero, "Microphone", PLAYBACK)
                .unwrap()
                .endpoint_id,
            VirtualEndpointId::from_bytes([0; 16])
        );
        assert!(microphone_target(MICROPHONE, "Microphone", zero).is_ok());
        assert!(microphone_target(zero, "Microphone", zero).is_err());
    }

    #[test]
    fn microphone_name_matches_provider_utf16_and_control_character_limits() {
        // U+1F3B5 occupies two UTF-16 units, not one Unicode scalar or four bytes.
        let supplementary = "\u{1f3b5}";
        let valid_names = [
            "x".to_owned(),
            " ".to_owned(),
            "x".repeat(63),
            format!("{}x", supplementary.repeat(31)),
            "\u{007f}".to_owned(),
        ];
        let invalid_names = [
            String::new(),
            "x".repeat(64),
            supplementary.repeat(32),
            "a\0b".to_owned(),
            "a\nb".to_owned(),
            "a\tb".to_owned(),
            "\u{001f}".to_owned(),
        ];
        let id = StableEndpointId::from_virtual_endpoint_id(MICROPHONE.parse().unwrap());
        for name in valid_names {
            assert!(create_request(id, Intent::Microphone, Channels::Stereo, &name, false).is_ok());
            assert_eq!(
                microphone_target(MICROPHONE, &name, PLAYBACK).unwrap().name,
                name
            );
        }
        for name in invalid_names {
            assert!(
                create_request(id, Intent::Microphone, Channels::Stereo, &name, false).is_err()
            );
            assert!(microphone_target(MICROPHONE, &name, PLAYBACK).is_err());
        }
    }

    fn assert_topology(project: &EngineProject, expected: &OutputTarget, role: ClockRole) {
        project.validate().unwrap();
        assert_eq!(project.processing.engine.processing_sr, 48_000.0);
        assert_eq!(project.processing, Default::default());
        assert!(project.parameters.is_empty());
        assert!(project.strips.is_empty());
        assert_eq!(project.graph.nodes().len(), 3);
        assert_eq!(project.nodes.len(), 3);
        assert_eq!(project.graph.edges().len(), 2);
        assert!(
            project
                .graph
                .nodes()
                .iter()
                .all(|node| node.channels() == 2)
        );
        let mut source = None;
        let mut bus = None;
        let mut sink = None;
        for node in project.graph.nodes() {
            match (&project.nodes[&node.id()], node.kind()) {
                (NodeConfig::Input(input), NodeKind::Source) => {
                    assert_eq!(input.channels, 2);
                    assert_eq!(input.device, Default::default());
                    assert_eq!(
                        input.source,
                        moiren_engine::node::InputSource::VirtualPlayback {
                            endpoint_id: PLAYBACK.parse().unwrap(),
                        }
                    );
                    assert!(source.replace(node.id()).is_none());
                }
                (NodeConfig::Bus, NodeKind::Bus) => {
                    assert_eq!(node.inputs().len(), 1);
                    assert!(bus.replace(node.id()).is_none());
                }
                (NodeConfig::Output(output), NodeKind::Sink) => {
                    assert_eq!(output.channels, 2);
                    assert_eq!(&output.target, expected);
                    assert_eq!(output.device.clock_role, role);
                    assert!(output.device.hardware_sample_rate.is_none());
                    assert!(output.device.period_frames.is_none());
                    assert!(sink.replace(node.id()).is_none());
                }
                other => panic!("unexpected route node: {other:?}"),
            }
        }
        let [source, bus, sink] = [source.unwrap(), bus.unwrap(), sink.unwrap()];
        for (from, to) in [(source, bus), (bus, sink)] {
            let edge = project
                .graph
                .edges()
                .iter()
                .find(|edge| edge.src() == from && edge.dst() == to)
                .expect("exact Source -> Bus -> Sink edge");
            assert_eq!(*edge.params(), Default::default());
        }
    }

    #[test]
    fn physical_route_preserves_pinned_target_and_is_the_only_master() {
        let target = OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
            endpoint_id: "opaque render ID".into(),
            stable_id: Some("opaque stable ID".into()),
        });
        let project = minimal_project(PLAYBACK.parse().unwrap(), target.clone()).unwrap();
        assert_topology(&project, &target, ClockRole::Master);
    }

    #[test]
    fn physical_route_preserves_explicit_follow_default_selector() {
        let target = OutputTarget::Physical(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Communications,
        ));
        let project = minimal_project(PLAYBACK.parse().unwrap(), target.clone()).unwrap();
        assert_topology(&project, &target, ClockRole::Master);
    }

    #[test]
    fn microphone_route_is_exact_bus_route_with_only_followers() {
        let target = OutputTarget::VirtualMicrophone {
            endpoint_id: MICROPHONE.parse().unwrap(),
        };
        let project = minimal_project(PLAYBACK.parse().unwrap(), target.clone()).unwrap();
        assert_topology(&project, &target, ClockRole::Follower);
    }

    #[test]
    fn project_refuses_same_virtual_id_and_software_output() {
        let playback = PLAYBACK.parse().unwrap();
        assert!(
            minimal_project(
                playback,
                OutputTarget::VirtualMicrophone {
                    endpoint_id: playback
                }
            )
            .is_err()
        );
        assert!(
            minimal_project(
                playback,
                OutputTarget::Software {
                    name: "sink".into()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn project_retains_existing_invalid_physical_selector_validation() {
        for endpoint_id in ["", " "] {
            let target = OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: endpoint_id.into(),
                stable_id: None,
            });
            assert!(minimal_project(PLAYBACK.parse().unwrap(), target).is_err());
        }
    }
}
