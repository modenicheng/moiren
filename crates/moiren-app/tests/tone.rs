use moiren_app::tone::*;
use moiren_core::protocol::*;
use moiren_engine::processor::Gain;

#[test]
fn renderer_scalar_timeline_observes_real_blocks_without_telemetry() {
    let tone = prepare_tone(ToneConfig::default()).unwrap();
    let mut renderer =
        moiren_windows_audio::render::DemandRenderer::new(tone.compiled.engine, tone.output)
            .unwrap();
    let observer = renderer.timeline_observer();
    assert_eq!(observer.snapshot().frame, 0);
    renderer.render_interleaved(&mut [0.; 14]).unwrap();
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.frame, 7);
    assert_eq!(snapshot.epoch, 1);
    assert_eq!(snapshot.revision, 1);
    renderer.render_interleaved(&mut []).unwrap();
    assert_eq!(observer.snapshot().frame, 7);
}

#[test]
fn tone_graph_is_continuous_across_variable_blocks_and_pan_balances_stereo() {
    let mut tone = prepare_tone(ToneConfig {
        gain: 0.05,
        pan: 0.5,
        ..ToneConfig::default()
    })
    .unwrap();
    let mut frame = 0;
    for frames in [3, 1, 8, 7] {
        tone.compiled.engine.render(frames).unwrap();
        let mut samples = vec![0.0; frames * 2];
        assert_eq!(
            tone.output
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            frames
        );
        for pair in samples.as_chunks::<2>().0 {
            let expected = 0.05 * (std::f64::consts::TAU * 440.0 * frame as f64 / 48000.0).sin();
            assert!((f64::from(pair[0]) - expected * 0.5).abs() < 1e-7);
            assert!((f64::from(pair[1]) - expected).abs() < 1e-7);
            frame += 1;
        }
    }
}

#[test]
fn tone_gain_is_bound_to_the_existing_realtime_control_channel() {
    let mut tone = prepare_tone(ToneConfig::default()).unwrap();
    assert_eq!(
        tone.compiled
            .control
            .submit(
                ParameterRequest {
                    request_id: 1,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    target: ParameterKey {
                        processor: tone.compiled.bindings.node(tone.gain_node).unwrap(),
                        parameter: Gain::LEVEL
                    },
                    at: ApplyAt::Frame(2),
                    value: ParamValue::Float(0.0),
                    ramp_frames: 0,
                },
                0
            )
            .code,
        ReplyCode::Accepted
    );
    tone.compiled.engine.render(8).unwrap();
    let mut samples = [1.0; 16];
    tone.output.read_interleaved(&mut samples).unwrap();
    assert!(samples[2] > 0.0);
    assert_eq!(&samples[4..], &[0.0; 12]);
}

#[test]
fn invalid_tone_values_fail_before_preparing_the_engine() {
    for config in [
        ToneConfig {
            frequency_hz: 0.0,
            ..ToneConfig::default()
        },
        ToneConfig {
            frequency_hz: 24000.0,
            ..ToneConfig::default()
        },
        ToneConfig {
            frequency_hz: f64::NAN,
            ..ToneConfig::default()
        },
        ToneConfig {
            gain: 1.1,
            ..ToneConfig::default()
        },
        ToneConfig {
            pan: -1.1,
            ..ToneConfig::default()
        },
        ToneConfig {
            max_block_frames: 0,
            ..ToneConfig::default()
        },
    ] {
        assert!(matches!(
            prepare_tone(config),
            Err(ToneError::InvalidConfig)
        ));
    }
}
