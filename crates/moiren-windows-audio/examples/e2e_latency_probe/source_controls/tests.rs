use super::*;
use std::cell::RefCell;

#[test]
fn source_controls_dto_preserves_preparation_failures_and_absent_snapshot() {
    let controls = SourceControls {
        qualified: false,
        mmdevice_id: "owned-playback".into(),
        process_id: 42,
        process_creation_100ns: Reading::observed(0, 123456789),
        qpc_frequency_hz: Some(10_000_000),
        preparation: Preparation {
            session_control_hresult: Some(-123),
            session_volume_hresult: None,
            endpoint_volume_hresult: Some(0),
        },
        snapshot: None,
    };
    let value = serde_json::to_value(controls).unwrap();
    assert_eq!(value["qualified"], false);
    assert_eq!(value["mmdevice_id"], "owned-playback");
    assert_eq!(value["process_id"], 42);
    assert_eq!(value["process_creation_100ns"]["value"], 123456789);
    assert_eq!(value["qpc_frequency_hz"], 10_000_000);
    assert_eq!(value["preparation"]["session_control_hresult"], -123);
    assert!(value["preparation"]["session_volume_hresult"].is_null());
    assert_eq!(value["preparation"]["endpoint_volume_hresult"], 0);
    assert!(value["snapshot"].is_null());
    assert_eq!(value.as_object().unwrap().len(), 7);
}

struct Fake {
    calls: RefCell<Vec<&'static str>>,
    failure: bool,
}
impl Fake {
    fn read<T>(&self, stage: &'static str, value: T) -> Reading<T> {
        self.calls.borrow_mut().push(stage);
        Reading::observed(if self.failure { -123 } else { 0 }, value)
    }
}
impl ReadControls for Fake {
    fn session_state(&self) -> Reading<i32> {
        self.read("state", 1)
    }
    fn session_volume(&self) -> Reading<f32> {
        self.read("session_volume", 0.25)
    }
    fn session_mute(&self) -> Reading<bool> {
        self.read("session_mute", true)
    }
    fn endpoint_volume(&self) -> Reading<f32> {
        self.read("endpoint_volume", 0.5)
    }
    fn endpoint_mute(&self) -> Reading<bool> {
        self.read("endpoint_mute", false)
    }
}
#[test]
fn source_controls_actual_observation_brackets_exact_read_only_getters() {
    let reader = Fake {
        calls: RefCell::new(Vec::new()),
        failure: false,
    };
    let mut ticks = 100;
    let progress = SourceProgress {
        native_submitted_frames: 1536,
        submitted_nonzero_frames: 480,
        ..Default::default()
    };
    let snapshot = observe(
        &reader,
        progress.clone(),
        || {
            ticks += 1;
            Some(ticks)
        },
        || false,
    );
    assert_eq!(snapshot.session_volume.value, Some(0.25));
    assert_eq!(snapshot.session_mute.value, Some(true));
    assert_eq!(snapshot.endpoint_volume.value, Some(0.5));
    assert_eq!(snapshot.endpoint_mute.value, Some(false));
    assert_eq!(
        (snapshot.before_qpc, snapshot.after_qpc),
        (Some(101), Some(102))
    );
    assert_eq!(
        &*reader.calls.borrow(),
        &[
            "state",
            "session_volume",
            "session_mute",
            "endpoint_volume",
            "endpoint_mute"
        ]
    );
    assert_eq!(snapshot.source_progress, progress);
    assert!(snapshot.client_start_succeeded && !snapshot.stop_requested);
    assert_eq!(snapshot.phase, "after_first_nonzero_release");
}
#[test]
fn source_controls_disabled_has_no_preparation_or_global_calls() {
    let output: Option<()> = prepare_if(false, || panic!("disabled diagnostic must do nothing"));
    assert!(output.is_none());
}
#[test]
fn source_controls_getter_errors_remain_null_not_default_volume_or_mute() {
    let reader = Fake {
        calls: RefCell::new(Vec::new()),
        failure: true,
    };
    let snapshot = observe(&reader, SourceProgress::default(), || None, || false);
    assert_eq!(snapshot.session_volume.hresult, Some(-123));
    assert_eq!(snapshot.session_volume.value, None);
    assert_eq!(snapshot.session_mute.value, None);
    assert_eq!(snapshot.endpoint_volume.value, None);
    assert_eq!(snapshot.endpoint_mute.value, None);
    assert_eq!(snapshot.before_qpc, None);
}
#[test]
fn source_controls_stop_checked_before_each_getter_skips_remaining_calls() {
    let reader = Fake {
        calls: RefCell::new(Vec::new()),
        failure: false,
    };
    let mut checks = 0;
    let snapshot = observe(
        &reader,
        SourceProgress::default(),
        || None,
        || {
            checks += 1;
            checks >= 3
        },
    );
    assert_eq!(&*reader.calls.borrow(), &["state", "session_volume"]);
    assert!(snapshot.stop_requested);
    assert_eq!(snapshot.session_mute, Reading::default());
    assert_eq!(snapshot.endpoint_volume, Reading::default());
    assert_eq!(snapshot.endpoint_mute, Reading::default());
}
