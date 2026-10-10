use moiren_app::monitor::{MonitorConfig, prepare_monitor};
use moiren_engine::boundary::{ConstantSource, RtAudioSource};

#[test]
fn monitor_uses_compiler_graph_and_preserves_gain_pan_bindings() {
    let session = prepare_monitor(
        ConstantSource {
            channels: 2,
            value: 0.8,
        },
        MonitorConfig {
            gain: 0.5,
            pan: 1.0,
            max_block_frames: 8,
        },
    )
    .unwrap();
    assert!(session.compiled.bindings.node(session.gain_node).is_some());
    assert!(session.compiled.bindings.node(session.pan_node).is_some());
    let mut engine = session.compiled.engine;
    let mut output = session.output;
    for frames in [1, 8, 3] {
        engine.render(frames).unwrap();
        let mut samples = vec![99.0; frames * 2];
        assert_eq!(
            output
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            frames
        );
        for pair in samples.as_chunks::<2>().0 {
            assert_eq!(pair, &[0.0, 0.4]);
        }
    }
    assert_eq!(engine.timeline(), 12);
}

#[test]
fn monitor_rejects_bad_gain_pan_block_or_source_layout() {
    for config in [
        MonitorConfig {
            gain: f64::NAN,
            ..MonitorConfig::default()
        },
        MonitorConfig {
            gain: 1.1,
            ..MonitorConfig::default()
        },
        MonitorConfig {
            pan: -2.0,
            ..MonitorConfig::default()
        },
        MonitorConfig {
            max_block_frames: 0,
            ..MonitorConfig::default()
        },
    ] {
        assert!(
            prepare_monitor(
                ConstantSource {
                    channels: 2,
                    value: 0.1
                },
                config
            )
            .is_err()
        );
    }
    let source = ConstantSource {
        channels: 1,
        value: 0.1,
    };
    assert_eq!(
        <ConstantSource as RtAudioSource<f32>>::channel_count(&source),
        1
    );
    assert!(prepare_monitor(source, MonitorConfig::default()).is_err());
}

#[test]
fn monitor_gain_updates_use_the_existing_parameter_channel() {
    use moiren_core::protocol::*;
    use moiren_engine::processor::Gain;
    let mut session = prepare_monitor(
        ConstantSource {
            channels: 2,
            value: 0.8,
        },
        MonitorConfig {
            gain: 0.5,
            max_block_frames: 8,
            ..MonitorConfig::default()
        },
    )
    .unwrap();
    let reply = session.compiled.control.submit(
        ParameterRequest {
            request_id: 1,
            plan_revision: 1,
            timeline_epoch: 1,
            target: ParameterKey {
                processor: session.compiled.bindings.node(session.gain_node).unwrap(),
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::Frame(2),
            value: ParamValue::Float(0.0),
            ramp_frames: 0,
        },
        0,
    );
    assert_eq!(reply.code, ReplyCode::Accepted);
    session.compiled.engine.render(8).unwrap();
    let mut out = [99.0; 16];
    session.output.read_interleaved(&mut out).unwrap();
    assert_eq!(&out[..4], &[0.4; 4]);
    assert_eq!(&out[4..], &[0.0; 12]);
}

#[cfg(windows)]
#[test]
fn physical_packet_bridge_reaches_demand_renderer_through_real_graph() {
    use moiren_windows_audio::{clock_bridge::*, render::DemandRenderer};
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: 4,
        capacity_frames: 32,
        max_correction_ppm: 0.0,
        trim_on_prime: false,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    let pcm: Vec<u8> = [0.8f32, -0.4]
        .repeat(16)
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    ingress
        .push_packet(
            &pcm,
            CapturePacket {
                frames: 16,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    let graph = prepare_monitor(
        source,
        MonitorConfig {
            gain: 0.5,
            pan: -0.5,
            max_block_frames: 4,
        },
    )
    .unwrap();
    let mut renderer = DemandRenderer::new(graph.compiled.engine, graph.output).unwrap();
    let mut out = [99.0; 20];
    assert_eq!(renderer.render_interleaved(&mut out).unwrap().blocks, 3);
    for pair in out.as_chunks::<2>().0 {
        assert_eq!(pair, &[0.4, -0.1]);
    }
    assert_eq!(observer.snapshot().output_frames, 10);
    assert_eq!(renderer.timeline(), 10);
}

#[cfg(windows)]
#[test]
fn cancelled_monitor_preparation_never_opens_physical_devices() {
    use moiren_app::monitor::{MonitorConfig, MonitorOptions, prepare_monitor_with_stop};
    use moiren_windows_audio::{SessionDuration, StopSignal, capture::CaptureError};
    let stop = StopSignal::new().unwrap();
    stop.request_stop().unwrap();
    let result = prepare_monitor_with_stop(
        MonitorOptions {
            input_endpoint_id: "unopened input".into(),
            output_endpoint_id: "unopened output".into(),
            duration: SessionDuration::UntilStopped,
            config: MonitorConfig::default(),
        },
        stop,
    );
    assert!(matches!(
        result,
        Err(moiren_app::monitor::MonitorError::Capture(
            CaptureError::Cancelled
        ))
    ));
}
