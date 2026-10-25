use super::*;

struct ChannelSource(usize);
impl RtAudioSource<f64> for ChannelSource {
    fn channel_count(&self) -> usize {
        self.0
    }
    fn read(
        &mut self,
        _: &ProcessContext,
        mut output: moiren_engine::buffer::AudioBlockMut<'_, f64>,
    ) -> BoundaryReport {
        let frames = output.frames();
        for (channel, samples) in output.channels_mut().enumerate() {
            samples.fill((channel + 1) as f64 * 0.1);
        }
        BoundaryReport {
            transferred_frames: frames,
            ..BoundaryReport::default()
        }
    }
}

#[test]
fn mixer_channel_modes_and_faders_render_without_heap_activity() {
    for channels in [1, 2, 4] {
        for mode in [
            ChannelMode::Normal,
            ChannelMode::Mono,
            ChannelMode::Solo(channels - 1),
            ChannelMode::Swap,
        ] {
            for muted in [false, true] {
                let mut graph = LogicalGraph::new();
                let source = graph.create_node(NodeKind::Source, channels).unwrap();
                let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
                connect(&mut graph, source, sink, SendParams::default());
                let (writer, mut reader) = audio_bridge::<f64>(channels, 8, 4096).unwrap();
                let mut bindings = NodeBindings::new();
                bindings
                    .bind_source(source, ChannelSource(channels))
                    .unwrap();
                bindings.bind_strip(
                    source,
                    StripConfig {
                        gain: 0.5,
                        balance: if channels == 2 { 0.0 } else { 0.75 },
                        muted,
                        mode,
                    },
                );
                bindings.bind_strip(
                    sink,
                    StripConfig {
                        gain: 0.5,
                        ..StripConfig::default()
                    },
                );
                bindings.bind_sink(sink, writer).unwrap();
                let mut compiled = compile(&graph, bindings, config()).unwrap();
                let (result, counts) = allocation::track_allocations(|| compiled.engine.render(4));
                result.unwrap();
                assert_eq!(counts, (0, 0));
                let mut samples = vec![0.; channels * 4];
                reader.read_interleaved(&mut samples).unwrap();
                for frame in samples.chunks_exact(channels) {
                    for (channel, sample) in frame.iter().enumerate() {
                        let expected = if muted {
                            0.
                        } else {
                            0.025
                                * match mode {
                                    ChannelMode::Normal => (channel + 1) as f64,
                                    ChannelMode::Mono => (channels + 1) as f64 / 2.,
                                    ChannelMode::Solo(selected) => (selected + 1) as f64,
                                    ChannelMode::Swap => (channels - channel) as f64,
                                }
                        };
                        assert!(
                            (sample - expected).abs() < 1e-9,
                            "{channels} {mode:?}: {sample} != {expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn strip_balance_rejects_nonfinite_and_out_of_range_values() {
    for balance in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.01, 1.01] {
        assert!(
            !StripConfig {
                balance,
                ..Default::default()
            }
            .valid(2)
        );
    }
    for balance in [-1.0, 0.0, 1.0] {
        assert!(
            StripConfig {
                balance,
                ..Default::default()
            }
            .valid(2)
        );
    }
}

#[test]
fn strip_balance_follows_channel_transform_and_precedes_gain_for_pre_and_post_sends() {
    for mode in [
        ChannelMode::Normal,
        ChannelMode::Mono,
        ChannelMode::Solo(0),
        ChannelMode::Solo(1),
        ChannelMode::Swap,
    ] {
        for balance in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            for muted in [false, true] {
                let mut graph = LogicalGraph::new();
                let source = graph.create_node(NodeKind::Source, 2).unwrap();
                let pre = graph.create_node(NodeKind::Sink, 2).unwrap();
                let post = graph.create_node(NodeKind::Sink, 2).unwrap();
                connect(
                    &mut graph,
                    source,
                    pre,
                    SendParams {
                        tap: SendTap::PreFader,
                        ..SendParams::default()
                    },
                );
                connect(&mut graph, source, post, SendParams::default());
                let (pre_writer, mut pre_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
                let (post_writer, mut post_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
                let mut bindings = NodeBindings::new();
                bindings.bind_source(source, ChannelSource(2)).unwrap();
                bindings.bind_strip(
                    source,
                    StripConfig {
                        gain: 0.25,
                        balance,
                        muted,
                        mode,
                    },
                );
                bindings.bind_sink(pre, pre_writer).unwrap();
                bindings.bind_sink(post, post_writer).unwrap();
                let mut compiled = compile(&graph, bindings, config()).unwrap();
                let (result, counts) = allocation::track_allocations(|| compiled.engine.render(4));
                result.unwrap();
                assert_eq!(counts, (0, 0));
                let mut before = [0.; 8];
                let mut after = [0.; 8];
                pre_reader.read_interleaved(&mut before).unwrap();
                post_reader.read_interleaved(&mut after).unwrap();
                let transformed = match mode {
                    ChannelMode::Normal => [0.1, 0.2],
                    ChannelMode::Mono => [0.15, 0.15],
                    ChannelMode::Solo(0) => [0.1, 0.1],
                    ChannelMode::Solo(1) => [0.2, 0.2],
                    ChannelMode::Swap => [0.2, 0.1],
                    _ => unreachable!(),
                };
                for frame in 0..4 {
                    for channel in 0..2 {
                        let attenuation = if channel == 0 {
                            1.0 - balance.max(0.0)
                        } else {
                            1.0 + balance.min(0.0)
                        };
                        let expected = if muted {
                            0.0
                        } else {
                            transformed[channel] * attenuation
                        };
                        assert!(
                            (before[frame * 2 + channel] - expected).abs() < 1e-12,
                            "{mode:?} balance {balance}, channel {channel}: PRE differs"
                        );
                        assert!(
                            (after[frame * 2 + channel] - expected * 0.25).abs() < 1e-12,
                            "{mode:?} balance {balance}, channel {channel}: POST differs"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn strip_pre_fader_bypasses_only_strip_gain_and_respects_mute() {
    for muted in [false, true] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let pre = graph.create_node(NodeKind::Sink, 2).unwrap();
        let post = graph.create_node(NodeKind::Sink, 2).unwrap();
        connect(
            &mut graph,
            source,
            pre,
            SendParams {
                tap: SendTap::PreFader,
                gain: 0.5,
                ..SendParams::default()
            },
        );
        connect(&mut graph, source, post, SendParams::default());
        let (pre_writer, mut pre_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let (post_writer, mut post_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, 2);
        bindings.bind_strip(
            source,
            StripConfig {
                gain: 0.25,
                muted,
                ..StripConfig::default()
            },
        );
        bindings.bind_sink(pre, pre_writer).unwrap();
        bindings.bind_sink(post, post_writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        compiled.engine.render(4).unwrap();
        let mut before = [0.; 8];
        let mut after = [0.; 8];
        pre_reader.read_interleaved(&mut before).unwrap();
        post_reader.read_interleaved(&mut after).unwrap();
        assert_eq!(before, [if muted { 0. } else { 0.1875 }; 8]);
        assert_eq!(after, [if muted { 0. } else { 0.09375 }; 8]);
    }
}

#[test]
fn pre_fader_send_uses_gain_input_without_changing_the_post_fader_route() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let pre = graph.create_node(NodeKind::Sink, 2).unwrap();
    let post = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, gain, SendParams::default());
    connect(
        &mut graph,
        gain,
        pre,
        SendParams {
            tap: SendTap::PreFader,
            gain: 0.5,
            ..SendParams::default()
        },
    );
    connect(&mut graph, gain, post, SendParams::default());
    let (pre_writer, mut pre_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let (post_writer, mut post_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_gain(gain, 0.25).unwrap();
    bindings.bind_sink(pre, pre_writer).unwrap();
    bindings.bind_sink(post, post_writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    compiled.engine.render(4).unwrap();
    let mut before = [0.; 8];
    let mut after = [0.; 8];
    pre_reader.read_interleaved(&mut before).unwrap();
    post_reader.read_interleaved(&mut after).unwrap();
    assert_eq!(before, [0.1875; 8]);
    assert_eq!(after, [0.09375; 8]);
}

#[test]
fn sends_preserve_fan_out_and_accept_gain_above_the_node_gain_limit() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let raw = graph.create_node(NodeKind::Sink, 2).unwrap();
    let sent = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, raw, SendParams::default());
    let edge = connect(
        &mut graph,
        source,
        sent,
        SendParams {
            gain: 32.0,
            pan: 0.5,
            ..SendParams::default()
        },
    );
    let (raw_writer, mut raw_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let (send_writer, mut send_reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    constant(&mut bindings, source, 2);
    bindings.bind_sink(raw, raw_writer).unwrap();
    bindings.bind_sink(sent, send_writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let keys = compiled.bindings.edge(edge).unwrap();
    for (request_id, target, value) in [
        (1, keys.gain, ParamValue::Float(2.0)),
        (2, keys.pan, ParamValue::Float(-1.0)),
        (3, keys.mute, ParamValue::Bool(true)),
    ] {
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id,
                        plan_revision: 7,
                        timeline_epoch: 3,
                        target,
                        at: ApplyAt::Frame(2),
                        value,
                        ramp_frames: 0,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    compiled.engine.render(4).unwrap();
    let mut raw_samples = [0.0; 8];
    let mut sent_samples = [0.0; 8];
    raw_reader.read_interleaved(&mut raw_samples).unwrap();
    send_reader.read_interleaved(&mut sent_samples).unwrap();
    assert_eq!(raw_samples, [0.375; 8]);
    assert_eq!(sent_samples, [6.0, 12.0, 6.0, 12.0, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn edge_gain_and_pan_ramps_cross_blocks_and_nonstereo_pan_updates_are_rejected() {
    for channels in [1, 2, 4] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, channels).unwrap();
        let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
        let edge = connect(&mut graph, source, sink, SendParams::default());
        let (writer, mut reader) = audio_bridge::<f64>(channels, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        constant(&mut bindings, source, channels);
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        let keys = compiled.bindings.edge(edge).unwrap();
        let request = ParameterRequest {
            request_id: 1,
            plan_revision: 7,
            timeline_epoch: 3,
            target: keys.pan,
            at: ApplyAt::Frame(1),
            value: ParamValue::Float(1.0),
            ramp_frames: 4,
        };
        assert_eq!(
            compiled.control.submit(request, 0).code,
            if channels == 2 {
                ReplyCode::Accepted
            } else {
                ReplyCode::InvalidValue
            }
        );
        assert_eq!(
            compiled
                .control
                .submit(
                    ParameterRequest {
                        request_id: 2,
                        target: keys.gain,
                        value: ParamValue::Float(0.0),
                        ..request
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
        for frames in [2, 1, 3] {
            compiled.engine.render(frames).unwrap();
        }
        let mut samples = vec![0.0; channels * 6];
        reader.read_interleaved(&mut samples).unwrap();
        let expected = (0..6)
            .flat_map(|frame| {
                let progress = if frame == 0 {
                    0.0
                } else {
                    (frame as f64 / 4.0).min(1.0)
                };
                (0..channels).map(move |channel| {
                    let balance = if channels == 2 && channel == 0 {
                        1.0 - progress
                    } else {
                        1.0
                    };
                    0.375 * (1.0 - progress) * balance
                })
            })
            .collect::<Vec<_>>();
        assert_samples_close(&samples, &expected);
    }
}

#[test]
fn send_attenuation_precedes_gain_overflow() {
    for (pan, expected_left) in [(1.0, 0.0), (0.5, f64::MAX)] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        connect(
            &mut graph,
            source,
            sink,
            SendParams {
                gain: f64::MAX,
                pan,
                ..SendParams::default()
            },
        );
        let (writer, mut reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 2.0,
                },
            )
            .unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut compiled = compile(&graph, bindings, config()).unwrap();
        compiled.engine.render(1).unwrap();
        let mut samples = [0.0; 2];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples[0], expected_left);
        assert_eq!(samples[1], f64::INFINITY);
    }
}

fn strip_set(
    control: &mut moiren_engine::control::ControlPort,
    next_id: &mut u64,
    key: ParameterKey,
    value: ParamValue,
    ramp_frames: u32,
    observed: u64,
) -> ReplyCode {
    *next_id += 1;
    control
        .submit(
            ParameterRequest {
                request_id: *next_id,
                plan_revision: 7,
                timeline_epoch: 3,
                target: key,
                at: ApplyAt::NextBlock,
                value,
                ramp_frames,
            },
            observed,
        )
        .code
}

#[test]
fn strip_parameters_reach_pre_and_post_instances_and_mode_rejects_ramps() {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let pre = graph.create_node(NodeKind::Sink, 2).unwrap();
    let post = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(
        &mut graph,
        source,
        pre,
        SendParams {
            tap: SendTap::PreFader,
            ..SendParams::default()
        },
    );
    connect(&mut graph, source, post, SendParams::default());
    let (pre_writer, mut pre_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let (post_writer, mut post_reader) = audio_bridge::<f64>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, ChannelSource(2)).unwrap();
    bindings.bind_strip(source, StripConfig::default());
    bindings.bind_sink(pre, pre_writer).unwrap();
    bindings.bind_sink(post, post_writer).unwrap();
    let mut compiled = compile(&graph, bindings, config()).unwrap();
    let instances: Vec<_> = compiled.bindings.strips(source).unwrap().to_vec();
    // One PRE instance (no gain key) and one POST instance per output stream.
    assert_eq!(instances.len(), 2);
    let post_keys = instances
        .iter()
        .find(|keys| keys.gain.is_some())
        .expect("POST strip instance");

    let mut id = 0;
    let mut frame = 0;
    let mut phase = |compiled: &mut CompiledGraph<f64>,
                     frame: &mut u64,
                     expected_pre: [f64; 8],
                     expected_post: [f64; 8]| {
        compiled.engine.render(4).unwrap();
        *frame += 4;
        let mut before = [0.; 8];
        let mut after = [0.; 8];
        pre_reader.read_interleaved(&mut before).unwrap();
        post_reader.read_interleaved(&mut after).unwrap();
        assert_eq!(before, expected_pre);
        assert_eq!(after, expected_post);
    };
    phase(
        &mut compiled,
        &mut frame,
        [0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2],
        [0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2],
    );

    // Mute fans out to every instance.
    for keys in &instances {
        assert_eq!(
            strip_set(
                &mut compiled.control,
                &mut id,
                keys.effective_mute,
                ParamValue::Bool(true),
                0,
                frame
            ),
            ReplyCode::Accepted
        );
    }
    phase(&mut compiled, &mut frame, [0.; 8], [0.; 8]);

    // Unmute and swap channels; the shared mode index follows each instance.
    for keys in &instances {
        strip_set(
            &mut compiled.control,
            &mut id,
            keys.effective_mute,
            ParamValue::Bool(false),
            0,
            frame,
        );
        strip_set(
            &mut compiled.control,
            &mut id,
            keys.mode,
            ParamValue::Enum(4),
            0,
            frame,
        );
    }
    phase(
        &mut compiled,
        &mut frame,
        [0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1],
        [0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1],
    );

    // Stereo balance attenuates the left channel fully at +1.
    for keys in &instances {
        strip_set(
            &mut compiled.control,
            &mut id,
            keys.balance,
            ParamValue::Float(1.0),
            0,
            frame,
        );
    }
    phase(
        &mut compiled,
        &mut frame,
        [0.0, 0.1, 0.0, 0.1, 0.0, 0.1, 0.0, 0.1],
        [0.0, 0.1, 0.0, 0.1, 0.0, 0.1, 0.0, 0.1],
    );

    // Gain exists only on the POST instance; the PRE tap stays at unity.
    strip_set(
        &mut compiled.control,
        &mut id,
        post_keys.gain.unwrap(),
        ParamValue::Float(0.5),
        0,
        frame,
    );
    phase(
        &mut compiled,
        &mut frame,
        [0.0, 0.1, 0.0, 0.1, 0.0, 0.1, 0.0, 0.1],
        [0.0, 0.05, 0.0, 0.05, 0.0, 0.05, 0.0, 0.05],
    );

    // Discrete domains never accept ramps.
    assert_eq!(
        strip_set(
            &mut compiled.control,
            &mut id,
            post_keys.mode,
            ParamValue::Enum(0),
            4,
            frame
        ),
        ReplyCode::InvalidValue
    );
}
