use moiren_engine::{
    audio_config::{
        AudioBackend, AudioSettings, BUFFER_SIZES, ProcessingPrecision, ProcessingSettings,
        SAMPLE_RATES, SampleFormat, StreamSettings,
    },
    boundary::ClockRole,
    node::DeviceMode,
};

#[test]
fn professional_choices_and_defaults_are_exact() {
    assert_eq!(SAMPLE_RATES, [44_100, 48_000, 88_200, 96_000]);
    assert_eq!(BUFFER_SIZES, [64, 128, 256, 512, 1024, 2048, 4096]);
    let settings = AudioSettings::default();
    settings.validate().unwrap();
    assert_eq!(settings.backend, AudioBackend::Wasapi);
    assert_eq!(settings.processing.precision, ProcessingPrecision::F32);
    assert_eq!(settings.processing.sample_rate, 48_000);
}

#[test]
fn shared_native_periods_are_independent_of_engine_block_presets() {
    for frames in [64, 65, 144, 336, 441, 480, 4096] {
        let mut settings = AudioSettings::default();
        settings.output.buffer_frames = Some(frames);
        settings.validate().unwrap();
        assert_eq!(
            settings
                .output
                .device_options(AudioBackend::Wasapi, 48_000, ClockRole::Master)
                .period_frames,
            Some(frames as usize)
        );
        if !BUFFER_SIZES.contains(&frames) {
            settings.processing.block_frames = frames;
            assert!(settings.validate().is_err(), "Engine block {frames}");
            settings.processing.block_frames = 256;
            settings.output.mode = DeviceMode::Exclusive;
            assert!(settings.validate().is_err(), "Exclusive period {frames}");
            settings.output.mode = DeviceMode::Shared;
            settings.backend = AudioBackend::Asio;
            assert!(settings.validate().is_err(), "ASIO buffer {frames}");
        }
    }
    for frames in [0, 1, 63, 4097, u32::MAX] {
        let mut settings = AudioSettings::default();
        settings.output.buffer_frames = Some(frames);
        assert!(settings.validate().is_err(), "Shared period {frames}");
    }
}

#[test]
fn device_overrides_are_directional_and_reset_restores_defaults() {
    let mut settings = AudioSettings::default();
    let custom = StreamSettings {
        sample_rate: Some(96_000),
        format: SampleFormat::Pcm24,
        buffer_frames: Some(128),
        mode: DeviceMode::Exclusive,
    };
    settings.input_overrides.insert("interface".into(), custom);
    assert_eq!(settings.effective_input("interface"), custom);
    assert_eq!(settings.effective_input("other"), settings.input);
    assert_eq!(settings.effective_output("interface"), settings.output);
    settings.input.sample_rate = Some(44_100);
    assert_eq!(settings.effective_input("interface"), custom);
    settings.input_overrides.remove("interface");
    assert_eq!(settings.effective_input("interface"), settings.input);
}

#[test]
fn unsupported_rates_buffers_and_device_keys_are_rejected() {
    for invalid in [0, 44_000, 88_000, 192_000] {
        let mut settings = AudioSettings::default();
        settings.processing.sample_rate = invalid;
        assert!(settings.validate().is_err(), "rate {invalid}");
    }
    for invalid in [0, 32, 4097, 8192] {
        let mut settings = AudioSettings::default();
        settings.input.buffer_frames = Some(invalid);
        assert!(settings.validate().is_err(), "buffer {invalid}");
        settings.input.buffer_frames = Some(256);
        settings.processing.block_frames = invalid;
        assert!(settings.validate().is_err(), "block {invalid}");
    }
    for invalid in [65, 336, 480] {
        let mut settings = AudioSettings::default();
        settings.processing.block_frames = invalid;
        assert!(settings.validate().is_err(), "block {invalid}");
    }
    for key in ["", " ", "driver\0id"] {
        let mut settings = AudioSettings::default();
        settings
            .output_overrides
            .insert(key.into(), settings.output);
        assert!(settings.validate().is_err());
    }
}

#[test]
fn resolved_device_options_keep_api_format_and_period_units() {
    let settings = AudioSettings {
        backend: AudioBackend::Asio,
        processing: ProcessingSettings {
            sample_rate: 88_200,
            ..ProcessingSettings::default()
        },
        ..AudioSettings::default()
    };
    let stream = StreamSettings {
        sample_rate: None,
        format: SampleFormat::Float64,
        buffer_frames: Some(512),
        mode: DeviceMode::Exclusive,
    };
    let device = stream.device_options(
        settings.backend,
        settings.processing.sample_rate,
        ClockRole::Master,
    );
    assert_eq!(device.backend, AudioBackend::Asio);
    assert_eq!(device.hardware_sample_rate, Some(88_200));
    assert_eq!(device.period_frames, Some(512));
    assert_eq!(device.sample_format, SampleFormat::Native);
    assert_eq!(device.clock_role, ClockRole::Master);
    let wasapi = stream.device_options(
        AudioBackend::Wasapi,
        settings.processing.sample_rate,
        ClockRole::Master,
    );
    assert_eq!(wasapi.sample_format, SampleFormat::Float64);
}
