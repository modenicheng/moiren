use moiren_windows_audio::{
    capture::CaptureError,
    process_loopback::{ProcessIdentity, ProcessLoopbackOptions},
};
use std::time::Duration;

fn options(pid: u32, creation_time_100ns: u64) -> ProcessLoopbackOptions {
    ProcessLoopbackOptions {
        target: ProcessIdentity {
            pid,
            creation_time_100ns,
            executable_name: "tone.exe".into(),
        },
        duration: Duration::from_secs(10),
    }
}
#[test]
fn selection_requires_pid_and_creation_time_and_rejects_self() {
    assert_eq!(options(0, 1).validate(), Err(CaptureError::InvalidProcess));
    assert_eq!(
        options(123, 0).validate(),
        Err(CaptureError::InvalidProcess)
    );
    assert_eq!(
        options(std::process::id(), 1).validate(),
        Err(CaptureError::FeedbackTarget)
    );
    let mut valid = options(123, 1);
    assert!(valid.validate().is_ok());
    valid.duration = Duration::from_secs(601);
    assert_eq!(valid.validate(), Err(CaptureError::InvalidDuration));
}
#[test]
fn identity_uses_creation_time_rather_than_name() {
    let selected = options(123, 1).target;
    let mut reopened = selected.clone();
    reopened.executable_name = "renamed.exe".into();
    assert!(selected.same_process(&reopened));
    reopened.creation_time_100ns = 2;
    assert!(!selected.same_process(&reopened));
    reopened = selected.clone();
    reopened.pid = 124;
    assert!(!selected.same_process(&reopened));
}
