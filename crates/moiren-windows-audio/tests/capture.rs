use moiren_windows_audio::capture::{CaptureError, CaptureOptions};
use std::time::Duration;

#[test]
fn capture_requires_pinned_id_and_bounded_duration_before_spawning() {
    for id in ["", "  ", "bad\0id"] {
        assert_eq!(
            CaptureOptions {
                endpoint_id: id.into(),
                duration: Duration::from_secs(10)
            }
            .validate(),
            Err(CaptureError::InvalidEndpoint)
        );
    }
    for seconds in [0, 601] {
        assert_eq!(
            CaptureOptions {
                endpoint_id: "id".into(),
                duration: Duration::from_secs(seconds)
            }
            .validate(),
            Err(CaptureError::InvalidDuration)
        );
    }
    assert!(
        CaptureOptions {
            endpoint_id: "opaque".into(),
            duration: Duration::from_secs(1)
        }
        .validate()
        .is_ok()
    );
}
