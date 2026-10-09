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

#[test]
fn continuous_capture_preserves_bounded_duration_and_endpoint_checks() {
    assert_eq!(CaptureOptions::continuous("id").duration, Duration::MAX);
    assert!(CaptureOptions::continuous("id").validate().is_ok());
    assert_eq!(
        CaptureOptions::continuous("").validate(),
        Err(CaptureError::InvalidEndpoint)
    );
    for duration in [Duration::from_secs(1), Duration::from_secs(600)] {
        assert!(
            CaptureOptions {
                endpoint_id: "id".into(),
                duration
            }
            .validate()
            .is_ok()
        );
    }
    for duration in [
        Duration::from_millis(999),
        Duration::from_secs(600) + Duration::from_nanos(1),
        Duration::MAX - Duration::from_nanos(1),
    ] {
        assert_eq!(
            CaptureOptions {
                endpoint_id: "id".into(),
                duration
            }
            .validate(),
            Err(CaptureError::InvalidDuration)
        );
    }
}
