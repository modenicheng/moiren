use moiren_app::{AppConfig, AppError, OfflineApp};
use moiren_core::protocol::ReplyCode;

#[test]
fn application_runs_input_gain_output_across_variable_blocks() {
    let mut app = OfflineApp::new(AppConfig {
        max_block_frames: 3,
        initial_gain: 0.5,
        ..AppConfig::default()
    })
    .unwrap();
    let input = [
        1.0, 10.0, 2.0, 20.0, 3.0, 30.0, 4.0, 40.0, 5.0, 50.0, 6.0, 60.0, 7.0, 70.0,
    ];
    let mut output = [99.0; 14];
    let report = app.process_interleaved(&input, &mut output).unwrap();
    assert_eq!((report.frames, report.blocks, report.segments), (7, 3, 3));
    assert_eq!(output, input.map(|value| value * 0.5));
    assert_eq!(app.timeline(), 7);
    let input_status = app.input_status().unwrap();
    let output_status = app.output_status().unwrap();
    assert_eq!(input_status.total_transferred_frames, 7);
    assert_eq!(output_status.total_transferred_frames, 7);
    assert_eq!(input_status.total_xruns + output_status.total_xruns, 0);
    assert_eq!(
        input_status.total_discontinuities + output_status.total_discontinuities,
        0
    );
    app.stop();
}

#[test]
fn gain_changes_use_control_queue_and_ramp_across_blocks() {
    let mut app = OfflineApp::new(AppConfig {
        channels: 1,
        max_block_frames: 2,
        initial_gain: 1.0,
        ..AppConfig::default()
    })
    .unwrap();
    assert_eq!(app.set_gain(0.0, 4).unwrap().code, ReplyCode::Accepted);
    assert!(app.poll_applied().is_none());
    let mut output = [99.0; 6];
    app.process_interleaved(&[1.0; 6], &mut output).unwrap();
    assert_eq!(output, [0.75, 0.5, 0.25, 0.0, 0.0, 0.0]);
    assert_eq!(app.poll_applied().unwrap().code, ReplyCode::Applied);
    assert!(matches!(
        app.set_gain(f64::NAN, 0),
        Err(AppError::ParameterRejected(ReplyCode::InvalidValue))
    ));
}

#[test]
fn malformed_requests_and_empty_requests_do_not_advance_or_consume_audio() {
    let mut app = OfflineApp::new(AppConfig::default()).unwrap();
    assert!(matches!(
        app.process_interleaved(&[1.0], &mut [99.0]),
        Err(AppError::InvalidSamples)
    ));
    assert!(matches!(
        app.process_interleaved(&[1.0, 2.0], &mut [99.0; 4]),
        Err(AppError::InvalidSamples)
    ));
    assert_eq!(app.process_interleaved(&[], &mut []).unwrap().frames, 0);
    assert_eq!(app.timeline(), 0);
    let mut output = [99.0; 2];
    app.process_interleaved(&[1.0, 2.0], &mut output).unwrap();
    assert_eq!(output, [0.5, 1.0]);
}

#[test]
fn invalid_app_config_is_rejected_before_starting() {
    for config in [
        AppConfig {
            channels: 0,
            ..AppConfig::default()
        },
        AppConfig {
            processing_sr: f64::NAN,
            ..AppConfig::default()
        },
        AppConfig {
            max_block_frames: 0,
            ..AppConfig::default()
        },
        AppConfig {
            initial_gain: 17.0,
            ..AppConfig::default()
        },
    ] {
        assert!(matches!(
            OfflineApp::new(config),
            Err(AppError::InvalidConfig)
        ));
    }
}
