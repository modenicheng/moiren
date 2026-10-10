use moiren_app::service::{
    DispatchError, InputSelection, RunLimit, ServiceCore, SessionPhase, SessionSpec,
};
fn tone_spec() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "test-output".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.05,
        pan: 0.0,
        max_block_frames: 256,
    }
}
#[test]
fn stopped_or_superseded_start_never_gets_permission_to_play() {
    let mut c = ServiceCore::default();
    let first = c.start(tone_spec()).unwrap();
    let second = c.start(tone_spec()).unwrap();
    assert!(!c.may_activate(first));
    assert!(c.may_activate(second));
    c.stop().unwrap();
    assert!(!c.may_activate(second));
    c.begin_exit();
    assert_eq!(c.phase(), SessionPhase::Exiting);
    assert_eq!(c.start(tone_spec()), Err(DispatchError::Exiting));
}
#[test]
fn actual_owner_survives_desired_replacement() {
    let mut c = ServiceCore::default();
    let a = c.start(tone_spec()).unwrap();
    assert!(c.activated(a));
    assert_eq!(c.phase(), SessionPhase::Starting);
    assert!(c.owner_started(a));
    let b = c.start(tone_spec()).unwrap();
    assert_eq!(c.owner_generation(), Some(a));
    assert_eq!(c.phase(), SessionPhase::Stopping);
    assert!(!c.may_activate(b));
    assert!(!c.owner_reaped(b));
    assert!(!c.owner_started(a));
    assert!(c.owner_reaped(a));
    assert_eq!(c.phase(), SessionPhase::Starting);
    assert!(c.may_activate(b));
}
#[test]
fn validation_and_stale_failure() {
    let mut c = ServiceCore::default();
    let mut bad = tone_spec();
    bad.gain = f64::NAN;
    assert_eq!(c.start(bad), Err(DispatchError::InvalidConfig));
    let a = c.start(tone_spec()).unwrap();
    let b = c.start(tone_spec()).unwrap();
    assert!(!c.failed(a, "old".into()));
    assert_eq!(c.generation(), b);
    assert_eq!(c.phase(), SessionPhase::Starting);
}
#[test]
fn elapsed_requires_real_owner_confirmation_and_exit_waits_for_reap() {
    let mut c = ServiceCore::default();
    let g = c.start(tone_spec()).unwrap();
    assert_eq!(c.elapsed(), std::time::Duration::ZERO);
    assert!(c.activated(g));
    assert_eq!(c.elapsed(), std::time::Duration::ZERO);
    assert!(c.owner_started(g));
    c.begin_exit();
    assert!(!c.finish_exit());
    assert_eq!(c.owner_generation(), Some(g));
    assert!(!c.owner_started(g));
    assert!(c.owner_reaped(g));
    assert!(c.finish_exit());
    assert_eq!(c.elapsed(), std::time::Duration::ZERO);
}
#[test]
fn rejects_invalid_domains_and_preserves_intent() {
    let mut c = ServiceCore::default();
    let g = c.start(tone_spec()).unwrap();
    let mut cases = Vec::new();
    let mut s = tone_spec();
    s.limit = RunLimit::Seconds(0);
    cases.push(s);
    let mut s = tone_spec();
    s.limit = RunLimit::Seconds(601);
    cases.push(s);
    let mut s = tone_spec();
    s.pan = 1.1;
    cases.push(s);
    let mut s = tone_spec();
    s.max_block_frames = 4097;
    cases.push(s);
    let mut s = tone_spec();
    s.output_endpoint_id = "\0".into();
    cases.push(s);
    let mut s = tone_spec();
    s.source = InputSelection::Tone {
        frequency_hz: 24000.,
    };
    cases.push(s);
    for bad in cases {
        assert_eq!(c.start(bad), Err(DispatchError::InvalidConfig));
        assert_eq!(c.generation(), g);
        assert!(c.may_activate(g));
    }
}
