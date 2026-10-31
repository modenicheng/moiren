use super::*;

#[test]
fn builtin_settings_default_by_kind_and_invalid_edits_are_atomic() {
    use moiren_engine::processor::{ChannelMapperSettings, EqSettings};

    let mut service = ServiceGraph::default();
    for kind in [
        NodeKind::Eq,
        NodeKind::Gate,
        NodeKind::Limiter,
        NodeKind::Delay,
        NodeKind::ChannelMapper,
        NodeKind::Recorder,
        NodeKind::Meter,
        NodeKind::SpectrumAnalyzer,
        NodeKind::Rack,
    ] {
        service
            .edit(GraphEdit::AddNode { kind, channels: 2 })
            .unwrap();
    }

    let snapshot = service.snapshot();
    for node in snapshot.graph.nodes().iter().skip(4) {
        assert_eq!(
            snapshot.node(node.id()).unwrap().builtin,
            BuiltinState::for_kind(node.kind())
        );
    }

    let eq = snapshot
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Eq)
        .unwrap()
        .id();
    let before = service.snapshot();
    for invalid in [
        BuiltinState::Eq(EqSettings {
            q: 0.0,
            ..EqSettings::default()
        }),
        BuiltinState::ChannelMapper(ChannelMapperSettings::default()),
    ] {
        assert!(
            service
                .edit(GraphEdit::SetBuiltin {
                    node: eq,
                    state: invalid,
                })
                .is_err()
        );
        assert_eq!(
            service.snapshot().candidate_revision,
            before.candidate_revision
        );
        assert_eq!(service.snapshot().nodes, before.nodes);
    }

    service
        .edit(GraphEdit::SetBuiltin {
            node: eq,
            state: BuiltinState::None,
        })
        .unwrap();
    assert_eq!(
        service.snapshot().node(eq).unwrap().builtin,
        BuiltinState::for_kind(NodeKind::Eq)
    );
}

#[test]
fn compressor_edits_are_node_local_and_reject_invalid_domains_transactionally() {
    let mut service = ServiceGraph::default();
    for _ in 0..2 {
        service
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Compressor,
                channels: 2,
            })
            .unwrap();
    }
    let nodes = service.snapshot().nodes;
    let id = nodes[4].id;
    for parameter in CompressorParameter::ALL {
        let (min, max) = parameter.range();
        let value = (min + max) * 0.5;
        service
            .edit(GraphEdit::SetCompressor {
                node: id,
                parameter,
                value,
            })
            .unwrap();
        assert_eq!(
            service
                .snapshot()
                .node(id)
                .unwrap()
                .compressor
                .value(parameter),
            value
        );
        let before = service.snapshot();
        for value in [f64::NAN, f64::INFINITY, min - 1., max + 1.] {
            assert!(
                service
                    .edit(GraphEdit::SetCompressor {
                        node: id,
                        parameter,
                        value
                    })
                    .is_err()
            );
            assert_eq!(
                service.snapshot().candidate_revision,
                before.candidate_revision
            );
            assert_eq!(service.snapshot().nodes, before.nodes);
        }
        assert!(
            service
                .edit(GraphEdit::SetCompressor {
                    node: nodes[1].id,
                    parameter,
                    value
                })
                .is_err()
        );
    }
    assert_eq!(
        service.snapshot().node(nodes[5].id).unwrap().compressor,
        CompressorState::default()
    );
}

#[test]
fn default_project_resolves_processing_rate_and_profile_buffers() {
    use crate::service::{AudioLatencyProfile, AudioSettings};
    use moiren_engine::host::NodeConfig;
    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "native-44100-capture".into(),
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "native-render".into(),
        })
        .unwrap();
    let preparation = service.preparation();
    let project = preparation.engine_project(256).unwrap();
    let NodeConfig::Input(input) = &project.nodes[&source] else {
        panic!("input");
    };
    let NodeConfig::Output(output) = &project.nodes[&sink] else {
        panic!("output");
    };
    assert_eq!(input.device.hardware_sample_rate, Some(48_000));
    assert_eq!(output.device.hardware_sample_rate, Some(48_000));
    assert_eq!(input.device.period_frames, None);
    assert_eq!(output.device.period_frames, None);
    // Profiles map to a device buffer request on the power-of-two presets.
    let balanced = preparation
        .configured_engine_project(
            256,
            &AudioSettings::default(),
            AudioLatencyProfile::Balanced,
        )
        .unwrap();
    let NodeConfig::Input(input) = &balanced.nodes[&source] else {
        panic!("input");
    };
    let NodeConfig::Output(output) = &balanced.nodes[&sink] else {
        panic!("output");
    };
    assert_eq!(input.device.period_frames, Some(512));
    assert_eq!(output.device.period_frames, Some(512));
    // Explicit device buffers win over the profile mapping.
    let mut explicit = AudioSettings::default();
    explicit.output.buffer_frames = Some(128);
    let mixed = preparation
        .configured_engine_project(256, &explicit, AudioLatencyProfile::Low)
        .unwrap();
    let NodeConfig::Input(input) = &mixed.nodes[&source] else {
        panic!("input");
    };
    let NodeConfig::Output(output) = &mixed.nodes[&sink] else {
        panic!("output");
    };
    assert_eq!(input.device.period_frames, Some(256));
    assert_eq!(output.device.period_frames, Some(128));
    let mut settings = AudioSettings::default();
    settings.processing.sample_rate = 96_000;
    let professional = preparation
        .configured_engine_project(256, &settings, AudioLatencyProfile::Compatible)
        .unwrap();
    let NodeConfig::Input(input) = &professional.nodes[&source] else {
        panic!("input");
    };
    let NodeConfig::Output(output) = &professional.nodes[&sink] else {
        panic!("output");
    };
    assert_eq!(input.device.hardware_sample_rate, Some(96_000));
    assert_eq!(output.device.hardware_sample_rate, Some(96_000));
}

#[test]
fn configured_project_resolves_each_endpoint_and_preserves_clock_roles() {
    use crate::service::{
        AudioBackend, AudioLatencyProfile, AudioSettings, SampleFormat, StreamSettings,
    };
    use moiren_engine::{
        boundary::ClockRole,
        host::NodeConfig,
        node::{DeviceMode, InputSource},
    };
    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "capture".into(),
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "render".into(),
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Sink,
            channels: 2,
        })
        .unwrap();
    let second = service.snapshot().graph.nodes()[4].id();
    service
        .edit(GraphEdit::BindOutput {
            node: second,
            endpoint_id: "follower".into(),
        })
        .unwrap();
    let mut settings = AudioSettings {
        backend: AudioBackend::Asio,
        ..AudioSettings::default()
    };
    settings.processing.sample_rate = 88_200;
    settings.input = StreamSettings {
        sample_rate: Some(44_100),
        format: SampleFormat::Pcm16,
        buffer_frames: Some(128),
        mode: DeviceMode::Exclusive,
    };
    settings.output.format = SampleFormat::Float32;
    let input_override = StreamSettings {
        sample_rate: Some(96_000),
        format: SampleFormat::Pcm24,
        buffer_frames: Some(512),
        mode: DeviceMode::Exclusive,
    };
    let output_override = StreamSettings {
        sample_rate: Some(48_000),
        format: SampleFormat::Pcm32,
        buffer_frames: Some(1024),
        mode: DeviceMode::Exclusive,
    };
    settings
        .input_overrides
        .insert("capture".into(), input_override);
    settings
        .output_overrides
        .insert("render".into(), output_override);
    let preparation = service.preparation();
    let project = preparation
        .configured_engine_project(256, &settings, AudioLatencyProfile::Compatible)
        .unwrap();
    assert_eq!(project.processing.engine.processing_sr, 88_200.0);
    assert_eq!(project.processing.engine.max_block_frames, 256);
    let NodeConfig::Input(input) = &project.nodes[&source] else {
        panic!("physical input");
    };
    assert!(matches!(input.source, InputSource::Physical(_)));
    assert_eq!(
        input.device,
        input_override.device_options(AudioBackend::Asio, 88_200, ClockRole::Follower)
    );
    for (node, stream, role) in [
        (sink, output_override, ClockRole::Master),
        (second, settings.output, ClockRole::Follower),
    ] {
        let NodeConfig::Output(output) = &project.nodes[&node] else {
            panic!("physical output");
        };
        assert_eq!(
            output.device,
            stream.device_options(AudioBackend::Asio, 88_200, role)
        );
    }
    settings.input_overrides.remove("capture");
    let reset = preparation
        .configured_engine_project(256, &settings, AudioLatencyProfile::Compatible)
        .unwrap();
    let NodeConfig::Input(input) = &reset.nodes[&source] else {
        panic!("physical input");
    };
    assert_eq!(
        input.device,
        settings
            .input
            .device_options(AudioBackend::Asio, 88_200, ClockRole::Follower)
    );
}

#[test]
fn process_capture_and_software_tone_do_not_inherit_physical_stream_options() {
    use crate::service::{
        AudioBackend, AudioLatencyProfile, AudioSettings, SampleFormat, StreamSettings,
    };
    use moiren_engine::{host::NodeConfig, node::DeviceMode};
    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "render".into(),
        })
        .unwrap();
    let mut settings = AudioSettings {
        backend: AudioBackend::Asio,
        ..AudioSettings::default()
    };
    settings.processing.sample_rate = 44_100;
    settings.input = StreamSettings {
        mode: DeviceMode::Exclusive,
        sample_rate: Some(96_000),
        format: SampleFormat::Pcm24,
        buffer_frames: Some(64),
    };
    for selection in [
        InputSelection::Process {
            pid: 42,
            creation_time_100ns: 7,
        },
        InputSelection::Tone {
            frequency_hz: 440.0,
        },
    ] {
        let process = matches!(selection, InputSelection::Process { .. });
        service
            .edit(GraphEdit::BindSource {
                node: source,
                source: selection,
            })
            .unwrap();
        let project = service
            .preparation()
            .configured_engine_project(256, &settings, AudioLatencyProfile::Compatible)
            .unwrap();
        assert_eq!(project.processing.engine.processing_sr, 44_100.0);
        let NodeConfig::Input(input) = &project.nodes[&source] else {
            panic!("input");
        };
        assert_eq!(input.device.backend, AudioBackend::Wasapi);
        assert_eq!(input.device.mode, DeviceMode::Shared);
        assert_eq!(input.device.period_frames, None);
        assert_eq!(input.device.hardware_sample_rate, process.then_some(48_000));
        assert_eq!(
            input.device.sample_format,
            if process {
                SampleFormat::Float32
            } else {
                SampleFormat::Native
            }
        );
    }
}

#[test]
fn managed_project_translates_every_io_and_stereo_parameter() {
    use moiren_core::protocol::ParamValue;
    use moiren_engine::host::{NodeConfig, ParameterTarget};
    use moiren_engine::processor::Pan;
    let mut service = ServiceGraph::default();
    let (_, _, pan, sink) = ids(&service);
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "first".into(),
        })
        .unwrap();
    service
        .edit(GraphEdit::SetMono {
            node: pan,
            mono: true,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Source,
            channels: 2,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Sink,
            channels: 2,
        })
        .unwrap();
    let snapshot = service.snapshot();
    let extra_source = snapshot.graph.nodes()[4].id();
    let extra_sink = snapshot.graph.nodes()[5].id();
    service
        .edit(GraphEdit::BindSource {
            node: extra_source,
            source: InputSelection::Physical {
                endpoint_id: "capture".into(),
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: extra_sink,
            endpoint_id: "second".into(),
        })
        .unwrap();
    let project = service.preparation().engine_project(256).unwrap();
    assert_eq!(
        project
            .nodes
            .values()
            .filter(|n| matches!(n, NodeConfig::Input(_)))
            .count(),
        2
    );
    assert_eq!(
        project
            .nodes
            .values()
            .filter(|n| matches!(n, NodeConfig::Output(_)))
            .count(),
        2
    );
    assert!(project.parameters.iter().any(|p| p.target
        == ParameterTarget::Node {
            node: pan,
            parameter: Pan::MONO
        }
        && p.value == ParamValue::Bool(true)));
    project.validate().unwrap();
}

#[test]
fn virtual_microphone_only_projects_do_not_assign_a_physical_master_clock() {
    use crate::service::{ExportDirection, ExportIntent};
    use moiren_engine::{boundary::ClockRole, host::NodeConfig, node::VirtualEndpointId};

    let mut service = ServiceGraph::default();
    let sink = service.snapshot().graph.nodes()[3].id();
    service
        .edit(GraphEdit::SetExports(vec![ExportIntent {
            endpoint_id: VirtualEndpointId::from_bytes([0x91; 16]),
            direction: ExportDirection::Microphone,
            name: "Virtual microphone".into(),
            channels: 2,
            enabled: true,
            binding: Some(sink),
        }]))
        .unwrap();

    let project = service.preparation().engine_project(256).unwrap();
    let NodeConfig::Output(output) = &project.nodes[&sink] else {
        panic!("expected virtual microphone output");
    };
    assert!(matches!(
        output.target,
        moiren_engine::node::OutputTarget::VirtualMicrophone { .. }
    ));
    assert_eq!(output.device.clock_role, ClockRole::Follower);
}

#[test]
fn vad_export_bindings_use_the_app_side_input_and_output_directions() {
    use crate::service::{ExportDirection, ExportIntent};
    use moiren_engine::{
        host::NodeConfig,
        node::{InputSource, OutputTarget, VirtualEndpointId},
    };

    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "old-capture".into(),
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "old-render".into(),
        })
        .unwrap();
    let playback = VirtualEndpointId::from_bytes([0xa1; 16]);
    let microphone = VirtualEndpointId::from_bytes([0xa2; 16]);
    service
        .edit(GraphEdit::SetExports(vec![
            ExportIntent {
                endpoint_id: playback,
                direction: ExportDirection::Playback,
                name: "VAD Speaker".into(),
                channels: 2,
                enabled: true,
                binding: None,
            },
            ExportIntent {
                endpoint_id: microphone,
                direction: ExportDirection::Microphone,
                name: "VAD MIC".into(),
                channels: 2,
                enabled: true,
                binding: None,
            },
        ]))
        .unwrap();
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: playback,
            node: Some(source),
        })
        .unwrap();
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: microphone,
            node: Some(sink),
        })
        .unwrap();

    let snapshot = service.snapshot();
    assert_eq!(snapshot.node(source).unwrap().source, None);
    assert_eq!(snapshot.node(sink).unwrap().output_endpoint_id, None);
    assert_eq!(snapshot.exports[0].binding, Some(source));
    assert_eq!(snapshot.exports[1].binding, Some(sink));
    let project = service.preparation().engine_project(256).unwrap();
    let NodeConfig::Input(input) = &project.nodes[&source] else {
        panic!("expected input node");
    };
    assert!(
        matches!(input.source, InputSource::VirtualPlayback { endpoint_id } if endpoint_id == playback)
    );
    let NodeConfig::Output(output) = &project.nodes[&sink] else {
        panic!("expected output node");
    };
    assert!(
        matches!(output.target, OutputTarget::VirtualMicrophone { endpoint_id } if endpoint_id == microphone)
    );
}

#[test]
fn physical_rebinding_and_clear_atomically_remove_vad_export_bindings() {
    use crate::service::{ExportDirection, ExportIntent};
    use moiren_engine::node::VirtualEndpointId;

    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    let playback = VirtualEndpointId::from_bytes([0xb1; 16]);
    let microphone = VirtualEndpointId::from_bytes([0xb2; 16]);
    service
        .edit(GraphEdit::SetExports(vec![
            ExportIntent {
                endpoint_id: playback,
                direction: ExportDirection::Playback,
                name: "VAD Speaker".into(),
                channels: 2,
                enabled: true,
                binding: Some(source),
            },
            ExportIntent {
                endpoint_id: microphone,
                direction: ExportDirection::Microphone,
                name: "VAD MIC".into(),
                channels: 2,
                enabled: true,
                binding: Some(sink),
            },
        ]))
        .unwrap();
    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "physical-render".into(),
        })
        .unwrap();

    let snapshot = service.snapshot();
    assert!(
        snapshot
            .exports
            .iter()
            .all(|export| export.binding.is_none())
    );
    assert!(matches!(
        snapshot.node(source).unwrap().source,
        Some(InputSelection::Tone {
            frequency_hz: 440.0
        })
    ));
    assert_eq!(
        snapshot.node(sink).unwrap().output_endpoint_id.as_deref(),
        Some("physical-render")
    );

    service.edit(GraphEdit::ClearBinding(source)).unwrap();
    service.edit(GraphEdit::ClearBinding(sink)).unwrap();
    let snapshot = service.snapshot();
    assert_eq!(snapshot.node(source).unwrap().source, None);
    assert_eq!(snapshot.node(sink).unwrap().output_endpoint_id, None);
    assert!(
        snapshot
            .exports
            .iter()
            .all(|export| export.binding.is_none())
    );
}

fn ids(service: &ServiceGraph) -> (NodeId, NodeId, NodeId, NodeId) {
    let nodes = service.snapshot.graph.nodes();
    (nodes[0].id(), nodes[1].id(), nodes[2].id(), nodes[3].id())
}
fn render(service: &ServiceGraph) -> Vec<f32> {
    use moiren_engine::boundary::ConstantSource;
    use moiren_engine::{
        boundary::audio_bridge,
        compiler::{CompileConfig, compile},
        runtime::EngineConfig,
    };
    let preparation = service.preparation();
    let (input, sink) = preparation.io_nodes().expect("io nodes");
    let (writer, mut output) = audio_bridge(2, 32, 8 * 1024 * 1024).unwrap();
    let mut bindings = moiren_engine::compiler::NodeBindings::new();
    bindings
        .bind_source(
            input,
            ConstantSource {
                channels: 2,
                value: 1.0,
            },
        )
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    // Test-only binding for the kernels these tests exercise. Production
    // binding goes through the managed EngineHost project; Bus and IO kinds
    // need no kernel binding here.
    for (node, config) in preparation.mixer_configs() {
        bindings.bind_strip(node, config);
    }
    for node in preparation.graph.nodes() {
        let state = preparation
            .nodes
            .iter()
            .find(|state| state.id == node.id())
            .expect("validated node state");
        match node.kind() {
            NodeKind::Gain => bindings.bind_gain(node.id(), state.gain).unwrap(),
            NodeKind::Pan => {
                bindings
                    .bind_pan_with_stereo(node.id(), state.pan, state.mono, state.swap)
                    .unwrap();
            }
            _ => {}
        }
    }
    let config = CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 32,
            max_events_per_block: 64,
        },
        audio_byte_budget: 8 * 1024 * 1024,
        plan_revision: service.snapshot.candidate_revision,
        timeline_epoch: crate::timeline::next_epoch().expect("epoch"),
        control_capacity: 64,
        control_horizon_frames: 48_000,
    };
    let mut compiled = compile(&preparation.graph, bindings, config).unwrap();
    assert_eq!(
        compiled.engine.revision(),
        service.snapshot.candidate_revision
    );
    compiled.engine.render(8).unwrap();
    let mut samples = vec![0.0; 16];
    assert_eq!(
        output
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        8
    );
    samples
}

#[test]
fn rejected_edits_are_atomic_and_do_not_consume_revision_or_node_ids() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    let before = service.snapshot();
    let output = before.graph.get_node(pan).unwrap().outputs()[0].id();
    let input = before.graph.get_node(gain).unwrap().inputs()[0].id();
    assert!(
        service
            .edit(GraphEdit::Connect {
                source: output,
                destination: input
            })
            .is_err()
    );
    let after = service.snapshot();
    assert_eq!(before.graph, after.graph);
    assert_eq!(before.nodes, after.nodes);
    assert_eq!(before.candidate_revision, after.candidate_revision);
    assert!(after.last_error.is_some());
    assert!(
        service
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: f64::NAN
            })
            .is_err()
    );
    assert_eq!(before.graph, service.snapshot.graph);
}
#[test]
fn candidate_edit_does_not_claim_native_application_and_stale_start_cannot_confirm_it() {
    let mut service = ServiceGraph::default();
    let old = service.snapshot.candidate_revision;
    service.preparing(SessionGeneration(1), old);
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .unwrap();
    assert_eq!(service.snapshot.active_revision, None);
    service.running(SessionGeneration(2), old);
    assert_eq!(service.snapshot.active_revision, None);
    service.running(SessionGeneration(1), old);
    assert_eq!(service.snapshot.active_revision, Some(old));
    assert_ne!(
        service.snapshot.active_revision,
        Some(service.snapshot.candidate_revision)
    );
    service.stopped();
    assert_eq!(service.snapshot.active_revision, None);
}
#[test]
fn edited_gain_pan_and_edge_send_reach_real_compiled_audio() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetPan {
            node: pan,
            pan: 0.5,
        })
        .unwrap();
    assert_eq!(render(&service), [0.25, 0.5].repeat(8));
    let edge = service.snapshot.graph.edges()[2].id();
    service
        .edit(GraphEdit::SetSend {
            edge,
            params: SendParams {
                mute: true,
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(render(&service), vec![0.0; 16]);
    service.edit(GraphEdit::Disconnect(edge)).unwrap();
    assert_eq!(render(&service), vec![0.0; 16]);
}
#[test]
fn deleting_processors_and_reconnecting_a_direct_route_changes_audio() {
    let mut service = ServiceGraph::default();
    let (source, gain, pan, sink) = ids(&service);
    service.edit(GraphEdit::RemoveNode(gain)).unwrap();
    service.edit(GraphEdit::RemoveNode(pan)).unwrap();
    let graph = &service.snapshot.graph;
    let source = graph.get_node(source).unwrap().outputs()[0].id();
    let destination = graph.get_node(sink).unwrap().inputs()[0].id();
    service
        .edit(GraphEdit::Connect {
            source,
            destination,
        })
        .unwrap();
    assert_eq!(service.preparation().gain_pan_nodes(), None);
    assert_eq!(render(&service), vec![1.0; 16]);
}
#[test]
fn bus_input_and_additional_gain_nodes_are_compiled_with_individual_values() {
    let mut service = ServiceGraph::default();
    let (source, gain, pan, sink) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Gain,
            channels: 2,
        })
        .unwrap();
    let extra_gain = service.snapshot.graph.nodes().last().unwrap().id();
    service
        .edit(GraphEdit::SetGain {
            node: extra_gain,
            gain: 0.25,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .unwrap();
    let bus = service.snapshot.graph.nodes().last().unwrap().id();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    service.edit(GraphEdit::AddBusInput(bus)).unwrap();
    let old_output = service
        .snapshot
        .graph
        .edges()
        .iter()
        .find(|e| e.dst() == sink)
        .unwrap()
        .id();
    service.edit(GraphEdit::Disconnect(old_output)).unwrap();
    for (src, dst, input_index) in [
        (source, extra_gain, 0),
        (pan, bus, 0),
        (extra_gain, bus, 1),
        (bus, sink, 0),
    ] {
        let graph = &service.snapshot.graph;
        let source = graph.get_node(src).unwrap().outputs()[0].id();
        let destination = graph.get_node(dst).unwrap().inputs()[input_index].id();
        service
            .edit(GraphEdit::Connect {
                source,
                destination,
            })
            .unwrap();
    }
    assert_eq!(render(&service), vec![0.75; 16]);
}
#[test]
fn legacy_single_io_restriction_does_not_limit_managed_graph_preparation() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Source,
            channels: 2,
        })
        .unwrap();
    assert!(
        service
            .preparation()
            .io_nodes()
            .unwrap_err()
            .contains("one Source and one Sink node")
    );
    assert!(service.preparation().primary_io_nodes().is_ok());
    assert_eq!(service.snapshot.graph.nodes().len(), 5);
    assert_eq!(service.snapshot.active_revision, None);
}

#[test]
fn mono_and_swap_candidate_values_are_compiled_and_reject_other_node_kinds() {
    let mut service = ServiceGraph::default();
    let (_, gain, pan, _) = ids(&service);
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 1.0,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetPan {
            node: pan,
            pan: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetSwap {
            node: pan,
            swap: true,
        })
        .unwrap();
    assert_eq!(render(&service), [1.0, 0.5].repeat(8));
    service
        .edit(GraphEdit::SetMono {
            node: pan,
            mono: true,
        })
        .unwrap();
    assert_eq!(render(&service), [0.75, 0.75].repeat(8));
    let before = service.snapshot();
    assert!(
        service
            .edit(GraphEdit::SetMono {
                node: gain,
                mono: true
            })
            .is_err()
    );
    assert_eq!(before.nodes, service.snapshot.nodes);
    assert_eq!(
        before.candidate_revision,
        service.snapshot.candidate_revision
    );
}

#[test]
fn rejected_edits_record_located_faults_that_clear_on_success() {
    let mut service = ServiceGraph::default();
    let node = service.snapshot().nodes[0].id;
    assert!(
        service
            .edit(GraphEdit::SetMixer {
                node,
                state: MixerState {
                    gain: 20.,
                    ..Default::default()
                },
            })
            .is_err()
    );
    let snapshot = service.snapshot();
    let fault = snapshot
        .faults
        .iter()
        .find(|fault| fault.subject == GraphSubject::Node(node))
        .expect("located fault");
    assert!(fault.hint.is_some());
    // The next successful edit replaces the draft and clears the group.
    assert!(
        service
            .edit(GraphEdit::SetMixer {
                node,
                state: MixerState::default(),
            })
            .is_ok()
    );
    assert!(service.snapshot().faults.is_empty());
}

#[test]
fn undo_journal_restores_content_with_monotonic_revisions() {
    let mut service = ServiceGraph::default();
    let gain = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Gain)
        .unwrap()
        .id();
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    let before_add = service.snapshot();
    assert!(before_add.can_undo);
    assert!(!before_add.can_redo);

    assert!(service.undo());
    let undone = service.snapshot();
    assert_eq!(undone.nodes, {
        let mut expected = before_add.nodes.clone();
        expected.pop();
        expected
    });
    assert!(undone.candidate_revision > before_add.candidate_revision);
    assert!(undone.can_undo);
    assert!(undone.can_redo);

    assert!(service.undo());
    assert_eq!(
        service.snapshot().node(gain).unwrap().gain,
        ServiceGraph::default().snapshot().node(gain).unwrap().gain,
    );
    assert!(!service.snapshot().can_undo);
    assert!(service.redo());
    assert_eq!(
        service.snapshot().node(gain).unwrap().gain,
        0.5,
        "redo replays the first undone edit"
    );
    assert!(service.snapshot().can_redo, "the AddNode step still awaits");
    assert!(service.redo());
    assert_eq!(service.snapshot().nodes.len(), before_add.nodes.len());
    assert!(!service.snapshot().can_redo);
    // A fresh edit invalidates the redo history.
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.75,
        })
        .unwrap();
    assert!(!service.snapshot().can_redo);
    assert!(service.undo());
    assert_eq!(service.snapshot().node(gain).unwrap().gain, 0.5);
}

#[test]
fn undo_journal_coalesces_rapid_same_target_edits() {
    let mut service = ServiceGraph::default();
    let gain = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Gain)
        .unwrap()
        .id();
    // A drag emits a burst of same-target edits within the window.
    for gain_value in [0.9, 0.8, 0.7, 0.6] {
        service
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: gain_value,
            })
            .unwrap();
    }
    assert!(service.undo());
    assert_eq!(
        service.snapshot().node(gain).unwrap().gain,
        ServiceGraph::default().snapshot().node(gain).unwrap().gain,
        "the whole burst collapses into one undo step"
    );
    assert!(!service.snapshot().can_undo, "only one entry was recorded");

    // Distinct targets never coalesce with each other.
    let pan = service
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Pan)
        .unwrap()
        .id();
    service
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .unwrap();
    service
        .edit(GraphEdit::SetPan {
            node: pan,
            pan: -0.5,
        })
        .unwrap();
    assert!(service.undo());
    assert_eq!(service.snapshot().node(pan).unwrap().pan, 0.);
    assert!(service.snapshot().can_undo);
}

#[test]
fn structural_edits_never_coalesce_in_the_undo_journal() {
    let mut service = ServiceGraph::default();
    for _ in 0..3 {
        service
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Bus,
                channels: 2,
            })
            .unwrap();
    }
    let added = service.snapshot().nodes.len();
    assert!(service.undo());
    assert_eq!(service.snapshot().nodes.len(), added - 1);
    assert!(service.undo());
    assert_eq!(service.snapshot().nodes.len(), added - 2);
}

#[test]
fn replace_records_one_undo_step_and_cannot_rewind_revisions() {
    let mut service = ServiceGraph::default();
    let revision_before = service.snapshot().candidate_revision;
    let replacement = ServiceGraph::default();
    let replacement_snapshot = replacement.snapshot();
    service
        .replace_with_exports(
            replacement_snapshot.graph,
            replacement_snapshot.nodes,
            Vec::new(),
        )
        .unwrap();
    assert!(service.snapshot().candidate_revision > revision_before);
    assert!(service.undo());
    let undone = service.snapshot();
    assert_eq!(undone.nodes.len(), 4, "default session restored");
    assert!(undone.candidate_revision > revision_before);
}

#[test]
fn undo_clears_faults_like_a_successful_edit() {
    let mut service = ServiceGraph::default();
    service
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Eq,
            channels: 2,
        })
        .unwrap();
    // A rejected rebuild leaves a fault behind while the journal is intact.
    service.record_error("This channel is missing a usable processor configuration");
    assert!(!service.snapshot().faults.is_empty());
    assert!(service.snapshot().last_error.is_some());
    assert!(service.undo());
    assert!(
        service.snapshot().faults.is_empty(),
        "a restored candidate clears the fault group"
    );
    assert!(service.snapshot().last_error.is_none());
}

#[test]
fn clearing_virtual_io_releases_the_export_binding() {
    use crate::service::ExportDirection;
    for direction in [ExportDirection::Playback, ExportDirection::Microphone] {
        let mut service = ServiceGraph::default();
        let (source, _, _, sink) = ids(&service);
        let node = if direction == ExportDirection::Playback {
            source
        } else {
            sink
        };
        service
            .edit(GraphEdit::SetExports(vec![ExportIntent {
                endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x42; 16]),
                direction,
                name: "Virtual IO".into(),
                channels: 2,
                enabled: true,
                binding: Some(node),
            }]))
            .unwrap();
        service.edit(GraphEdit::ClearBinding(node)).unwrap();
        assert_eq!(service.snapshot().exports[0].binding, None);
    }
}

#[test]
fn vad_exports_bind_to_opposite_app_io_directions_and_replace_physical_bindings_atomically() {
    use crate::service::{ExportDirection, ExportIntent};
    use moiren_engine::{
        host::NodeConfig,
        node::{InputSource, OutputTarget, VirtualEndpointId},
    };

    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Physical {
                endpoint_id: "capture-device".into(),
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "render-device".into(),
        })
        .unwrap();

    let playback = VirtualEndpointId::from_bytes([0xa1; 16]);
    let microphone = VirtualEndpointId::from_bytes([0xa2; 16]);
    service
        .edit(GraphEdit::SetExports(vec![
            ExportIntent {
                endpoint_id: playback,
                direction: ExportDirection::Playback,
                name: "VAD Speaker".into(),
                channels: 2,
                enabled: true,
                binding: None,
            },
            ExportIntent {
                endpoint_id: microphone,
                direction: ExportDirection::Microphone,
                name: "VAD MIC".into(),
                channels: 2,
                enabled: true,
                binding: None,
            },
        ]))
        .unwrap();
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: playback,
            node: Some(source),
        })
        .unwrap();
    service
        .edit(GraphEdit::BindExport {
            endpoint_id: microphone,
            node: Some(sink),
        })
        .unwrap();

    let snapshot = service.snapshot();
    assert_eq!(snapshot.node(source).unwrap().source, None);
    assert_eq!(snapshot.node(sink).unwrap().output_endpoint_id, None);
    assert_eq!(snapshot.exports[0].binding, Some(source));
    assert_eq!(snapshot.exports[1].binding, Some(sink));
    let project = service.preparation().engine_project(256).unwrap();
    let NodeConfig::Input(input) = &project.nodes[&source] else {
        panic!("expected input node");
    };
    assert!(
        matches!(input.source, InputSource::VirtualPlayback { endpoint_id } if endpoint_id == playback)
    );
    let NodeConfig::Output(output) = &project.nodes[&sink] else {
        panic!("expected output node");
    };
    assert!(
        matches!(output.target, OutputTarget::VirtualMicrophone { endpoint_id } if endpoint_id == microphone)
    );

    service
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
        })
        .unwrap();
    service
        .edit(GraphEdit::BindOutput {
            node: sink,
            endpoint_id: "physical-output-again".into(),
        })
        .unwrap();
    assert!(
        service
            .snapshot()
            .exports
            .iter()
            .all(|export| export.binding.is_none())
    );
    let project = service.preparation().engine_project(256).unwrap();
    let NodeConfig::Output(output) = &project.nodes[&sink] else {
        panic!("expected physical output");
    };
    assert!(
        matches!(&output.target, OutputTarget::Physical(moiren_engine::node::EndpointSelector::PinnedEndpoint { endpoint_id, .. }) if endpoint_id == "physical-output-again")
    );
}

#[test]
fn vad_export_binding_rejects_wrong_node_shape_without_partial_changes() {
    use crate::service::{ExportDirection, ExportIntent};
    use moiren_engine::node::VirtualEndpointId;

    let mut service = ServiceGraph::default();
    let (source, _, _, sink) = ids(&service);
    let playback = VirtualEndpointId::from_bytes([0xb1; 16]);
    service
        .edit(GraphEdit::SetExports(vec![ExportIntent {
            endpoint_id: playback,
            direction: ExportDirection::Playback,
            name: "VAD Speaker".into(),
            channels: 2,
            enabled: true,
            binding: None,
        }]))
        .unwrap();
    let before = service.snapshot();
    assert!(
        service
            .edit(GraphEdit::BindExport {
                endpoint_id: playback,
                node: Some(sink),
            })
            .is_err()
    );
    let after = service.snapshot();
    assert_eq!(after.exports, before.exports);
    assert_eq!(after.nodes, before.nodes);
    assert_eq!(after.candidate_revision, before.candidate_revision);
    assert_ne!(source, sink);
}
