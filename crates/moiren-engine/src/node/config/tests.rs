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
