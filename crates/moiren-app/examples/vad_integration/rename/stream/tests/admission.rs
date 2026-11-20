use super::*;

#[test]
fn replacement_accepts_pre_client_admission_without_claiming_pcm_completion() {
    let old = runtime(false);
    let mut fresh = runtime(true);
    fresh.native_stamp.as_mut().unwrap()[3] = 0;
    assert!(replacement(&old, &fresh, "new", 4).is_ok());

    let mut phase = phase(true);
    phase.admission = Some(fresh.clone());
    assert!(phase.require_complete().is_ok());
    phase.confirmed = Some(fresh);
    assert!(
        phase.require_complete().is_err(),
        "pre-client state is not PCM confirmation"
    );

    let mut old_without_stream = old;
    old_without_stream.native_stamp.as_mut().unwrap()[3] = 0;
    assert!(replacement(&old_without_stream, &runtime(true), "new", 4).is_err());
}

#[test]
fn pre_client_admission_still_requires_identity_epoch_and_an_open_gate() {
    let mut idle = runtime(true);
    idle.native_stamp.as_mut().unwrap()[3] = 0;
    assert!(running(&idle).is_ok());
    for slot in [0, 1, 2, 4] {
        let mut invalid = idle.clone();
        invalid.native_stamp.as_mut().unwrap()[slot] = 0;
        assert!(running(&invalid).is_err(), "missing stamp component {slot}");
    }
    idle.gate_open = false;
    assert!(running(&idle).is_err());
}
