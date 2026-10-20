use crate::{
    boundary::{CaptureIntent, ClockRole},
    node::*,
};

fn options() -> DeviceOptions {
    DeviceOptions {
        mode: DeviceMode::Shared,
        hardware_sample_rate: Some(48_000),
        period_frames: Some(128),
        clock_role: ClockRole::Follower,
        ..Default::default()
    }
}

#[test]
fn physical_software_and_application_configs_are_control_data() {
    let endpoint = EndpointSelector::PinnedEndpoint {
        endpoint_id: "opaque Endpoint ID".into(),
        stable_id: Some("opaque Stable ID".into()),
    };
    let sources = [
        InputSource::Physical(endpoint.clone()),
        InputSource::Physical(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Communications,
        )),
        InputSource::Application {
            executable: "C:/apps/player.exe".into(),
            intent: CaptureIntent::PassiveTap,
        },
        InputSource::Software {
            name: "offline".into(),
        },
    ];
    for source in sources {
        InputConfig {
            source,
            channels: 2,
            device: options(),
        }
        .validate()
        .unwrap();
    }
    for target in [
        OutputTarget::Physical(endpoint),
        OutputTarget::Software {
            name: "monitor".into(),
        },
    ] {
        OutputConfig {
            target,
            channels: 6,
            device: options(),
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn invalid_config_is_rejected_before_preparation() {
    let mut config = InputConfig {
        source: InputSource::Software {
            name: "offline".into(),
        },
        channels: 0,
        device: options(),
    };
    assert_eq!(config.validate(), Err(IoConfigError::InvalidChannels));
    config.channels = 2;
    config.device.hardware_sample_rate = Some(0);
    assert_eq!(config.validate(), Err(IoConfigError::InvalidDeviceOptions));
    config.device.hardware_sample_rate = Some(48_000);
    config.device.period_frames = Some(0);
    assert_eq!(config.validate(), Err(IoConfigError::InvalidDeviceOptions));
    config.device.period_frames = None;
    for source in [
        InputSource::Software { name: " ".into() },
        InputSource::Application {
            executable: "".into(),
            intent: CaptureIntent::RoutedInput,
        },
        InputSource::Physical(EndpointSelector::PinnedEndpoint {
            endpoint_id: "".into(),
            stable_id: None,
        }),
        InputSource::Physical(EndpointSelector::PinnedEndpoint {
            endpoint_id: "id".into(),
            stable_id: Some("".into()),
        }),
    ] {
        config.source = source;
        assert_eq!(config.validate(), Err(IoConfigError::InvalidSelector));
    }
    let output = OutputConfig {
        target: OutputTarget::Software { name: "".into() },
        channels: 2,
        device: options(),
    };
    assert_eq!(output.validate(), Err(IoConfigError::InvalidSelector));
}

#[test]
fn virtual_endpoints_accept_only_mono_or_stereo_without_limiting_other_resources() {
    let endpoint_id = VirtualEndpointId::from_bytes([0x81; 16]);
    for channels in [0, 1, 2, 3, 6, usize::MAX] {
        let input = InputConfig {
            source: InputSource::VirtualPlayback { endpoint_id },
            channels,
            device: options(),
        };
        let output = OutputConfig {
            target: OutputTarget::VirtualMicrophone { endpoint_id },
            channels,
            device: options(),
        };
        let expected = match channels {
            0 => Err(IoConfigError::InvalidChannels),
            1 | 2 => Ok(()),
            _ => Err(IoConfigError::InvalidVirtualChannels),
        };
        assert_eq!(input.validate(), expected, "input channels: {channels}");
        assert_eq!(output.validate(), expected, "output channels: {channels}");
    }
    for source in [
        InputSource::Physical(EndpointSelector::FollowDefault(DefaultDeviceRole::Console)),
        InputSource::Application {
            executable: "player.exe".into(),
            intent: CaptureIntent::PassiveTap,
        },
        InputSource::Software { name: "six".into() },
    ] {
        InputConfig {
            source,
            channels: 6,
            device: options(),
        }
        .validate()
        .unwrap();
    }
    for target in [
        OutputTarget::Physical(EndpointSelector::FollowDefault(DefaultDeviceRole::Console)),
        OutputTarget::Software { name: "six".into() },
    ] {
        OutputConfig {
            target,
            channels: 6,
            device: options(),
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn virtual_intent_keeps_device_option_validation_without_selecting_a_global_backend() {
    use crate::audio_config::AudioBackend;

    let endpoint_id = VirtualEndpointId::from_bytes([1; 16]);
    for backend in AudioBackend::ALL {
        let device = DeviceOptions {
            backend,
            ..options()
        };
        let mut input = InputConfig {
            source: InputSource::VirtualPlayback { endpoint_id },
            channels: 1,
            device,
        };
        let mut output = OutputConfig {
            target: OutputTarget::VirtualMicrophone { endpoint_id },
            channels: 2,
            device,
        };
        input.validate().unwrap();
        output.validate().unwrap();
        input.device.hardware_sample_rate = Some(0);
        output.device.period_frames = Some(0);
        assert_eq!(input.validate(), Err(IoConfigError::InvalidDeviceOptions));
        assert_eq!(output.validate(), Err(IoConfigError::InvalidDeviceOptions));
    }
}
