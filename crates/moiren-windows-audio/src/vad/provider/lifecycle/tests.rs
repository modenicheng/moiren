use super::*;
use crate::vad::{Capabilities, EndpointSnapshot};

fn id() -> VirtualEndpointId {
    VirtualEndpointId::from_bytes([0x31; 16])
}
fn spec() -> ExportSpec {
    ExportSpec {
        id: id(),
        intent: Intent::Playback,
        channels: Channels::Stereo,
        name: "Named application playback".into(),
        allow_default: false,
    }
}
fn snapshot(state: PdoState, stream: u64, gate: u64, open: bool) -> EndpointSnapshot {
    snapshot_generation(state, stream, gate, open, 11)
}
fn snapshot_generation(
    state: PdoState,
    stream: u64,
    gate: u64,
    open: bool,
    generation: u64,
) -> EndpointSnapshot {
    snapshot_native(
        state,
        stream,
        gate,
        open,
        generation,
        (StreamState::Running, true, state == PdoState::Present),
    )
}
fn snapshot_native(
    state: PdoState,
    stream: u64,
    gate: u64,
    open: bool,
    generation: u64,
    (stream_state, policy_prepared, audio_ready): (StreamState, bool, bool),
) -> EndpointSnapshot {
    snapshot_native_with_status(
        state,
        stream,
        gate,
        open,
        generation,
        (stream_state, policy_prepared, audio_ready),
        0,
    )
}
fn snapshot_native_with_status(
    state: PdoState,
    stream: u64,
    gate: u64,
    open: bool,
    generation: u64,
    (stream_state, policy_prepared, audio_ready): (StreamState, bool, bool),
    last_status: u32,
) -> EndpointSnapshot {
    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(b: &mut [u8], o: usize, v: u64) {
        b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    let mut caps = [0; 64];
    for (o, v) in [
        (0, 64),
        (4, 2),
        (8, 31),
        (12, 64),
        (16, 1),
        (20, 48000),
        (24, 3),
        (28, 6),
        (32, 6),
        (36, 4),
        (48, 4800),
    ] {
        put32(&mut caps, o, v);
    }
    put64(&mut caps, 40, 7);
    let caps = Capabilities::decode(&caps).unwrap();
    let mut bytes = [0; 128];
    for (o, v) in [
        (0, 128),
        (4, 2),
        (16, state as u32),
        (20, 1),
        (24, 2),
        (28, u32::from(open)),
        (32, u32::from(policy_prepared)),
        (36, u32::from(audio_ready)),
        (
            88,
            if stream_state == StreamState::Closed {
                0
            } else {
                2
            },
        ),
        (
            92,
            if stream_state == StreamState::Closed {
                0
            } else {
                480
            },
        ),
        (96, last_status),
        (100, stream_state as u32),
    ] {
        put32(&mut bytes, o, v);
    }
    for (o, v) in [
        (8, generation),
        (40, 7),
        (48, generation),
        (56, stream),
        (64, gate),
    ] {
        put64(&mut bytes, o, v);
    }
    bytes[72..88].copy_from_slice(&StableEndpointId::from_virtual_endpoint_id(id()).to_le_bytes());
    EndpointSnapshot::decode(&bytes, caps).unwrap()
}

struct Fake {
    current: EndpointSnapshot,
    creates: Vec<ExportSpec>,
    gates: Vec<bool>,
    remove_calls: usize,
    fail_create: bool,
    defer_create: bool,
    fail_remove: bool,
    revoke_on_remove_failure: bool,
    fail_query: bool,
    creating: bool,
    revoke_on_open: Option<Arc<Admission>>,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            current: snapshot(PdoState::Present, 3, 5, false),
            creates: Vec::new(),
            gates: Vec::new(),
            remove_calls: 0,
            fail_create: false,
            defer_create: false,
            fail_remove: false,
            revoke_on_remove_failure: false,
            fail_query: false,
            creating: false,
            revoke_on_open: None,
        }
    }
}
impl EndpointControl for Fake {
    fn create(&mut self, spec: &ExportSpec) -> Result<Option<EndpointSnapshot>, BackendError> {
        self.creates.push(spec.clone());
        if self.defer_create {
            return Ok(None);
        }
        if self.fail_create {
            return Err(unavailable("CREATE timeout"));
        }
        self.current = snapshot_generation(
            if self.creating {
                PdoState::Creating
            } else {
                PdoState::Present
            },
            3,
            5,
            false,
            10 + self.creates.len() as u64,
        );
        Ok(Some(self.current))
    }
    fn query(&mut self, _: EndpointSnapshot) -> Result<EndpointSnapshot, BackendError> {
        if self.fail_query {
            return Err(unavailable("QUERY timeout"));
        }
        Ok(self.current)
    }
    fn gate(
        &mut self,
        previous: EndpointSnapshot,
        open: bool,
    ) -> Result<EndpointSnapshot, BackendError> {
        assert_eq!(stamp(previous), stamp(self.current));
        self.gates.push(open);
        if open && (!previous.policy_prepared() || !previous.audio_ready()) {
            return Err(unavailable("SET_GATE requires prepared policy and audio"));
        }
        self.current = snapshot_native(
            PdoState::Present,
            previous.stream_generation(),
            previous.gate_epoch() + u64::from(previous.gate_open() && !open),
            open,
            previous.endpoint_generation(),
            (
                previous.stream_state(),
                previous.policy_prepared(),
                previous.audio_ready(),
            ),
        );
        if open && let Some(authority) = &self.revoke_on_open {
            authority.set(None);
        }
        Ok(self.current)
    }
    fn remove(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot, BackendError> {
        self.remove_calls += 1;
        if self.fail_remove {
            if self.revoke_on_remove_failure {
                // The real driver revokes before asking ACX to remove the
                // child. ACX can reject that call after State became Removing.
                self.current = snapshot_generation(
                    PdoState::Removing,
                    previous.stream_generation(),
                    previous.gate_epoch(),
                    false,
                    previous.endpoint_generation(),
                );
            }
            return Err(unavailable("REMOVE timeout"));
        }
        self.current = snapshot_generation(
            PdoState::Removing,
            previous.stream_generation(),
            previous.gate_epoch(),
            false,
            previous.endpoint_generation(),
        );
        Ok(self.current)
    }
}
fn prepared(lifecycle: &mut Lifecycle<Fake>, admission: Arc<Admission>) -> Arc<SessionState> {
    let session = Arc::new(SessionState::new(admission));
    session.active.store(true, Ordering::Release);
    lifecycle
        .endpoints
        .get_mut(&id())
        .unwrap()
        .sessions
        .push(session.clone());
    session
}

#[test]
fn named_creation_and_engine_activation_do_not_admit_native_audio() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let admission = Arc::new(Admission::new());
    let session = prepared(&mut lifecycle, admission.clone());
    assert!(session.prepared());
    assert!(!session.admitted());
    assert!(!lifecycle.poll()[0].snapshot.unwrap().gate_open());
    assert_eq!(
        lifecycle.control.creates[0].name,
        "Named application playback"
    );
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, 1, &admission)
            .is_err()
    );
    assert!(lifecycle.control.gates.is_empty());
}

#[test]
fn prepared_idle_endpoint_can_be_admitted_without_a_native_client() {
    for stream_state in [StreamState::Closed, StreamState::Paused] {
        let mut lifecycle = Lifecycle::new(Fake::default());
        lifecycle.create(spec()).unwrap();
        lifecycle.control.current = snapshot_native(
            PdoState::Present,
            u64::from(stream_state != StreamState::Closed),
            5,
            false,
            11,
            (stream_state, true, true),
        );
        let view = lifecycle.poll().remove(0);
        let expected = stamp(view.snapshot.unwrap());
        let admission = Arc::new(Admission::new());
        let ticket = admission.set(Some(9));
        assert!(
            lifecycle
                .gate(id(), expected, true, 9, ticket, &admission)
                .is_err()
        );
        assert!(
            lifecycle.control.gates.is_empty(),
            "an owner must be prepared first"
        );

        let session = prepared(&mut lifecycle, admission.clone());
        lifecycle
            .gate(id(), expected, true, 9, ticket, &admission)
            .unwrap();
        assert!(session.admitted());
        assert_eq!(lifecycle.control.gates, vec![true]);
        assert_eq!(lifecycle.control.current.stream_state(), stream_state);
        assert!(lifecycle.control.current.gate_open());
        let (current, changed) = super::super::transport::poll(
            &mut lifecycle.control,
            lifecycle.endpoints[&id()].snapshot.unwrap(),
            &session,
        )
        .unwrap();
        assert!(!changed);
        assert!(
            current.gate_open(),
            "idle polling must preserve valid admission"
        );
    }
}

#[test]
fn endpoint_policy_and_audio_readiness_are_required_even_with_a_prepared_owner() {
    for (policy, audio) in [(false, false), (true, false)] {
        let mut lifecycle = Lifecycle::new(Fake::default());
        lifecycle.create(spec()).unwrap();
        lifecycle.control.current = snapshot_native(
            PdoState::Present,
            3,
            5,
            false,
            11,
            (StreamState::Running, policy, audio),
        );
        let expected = stamp(lifecycle.poll()[0].snapshot.unwrap());
        let admission = Arc::new(Admission::new());
        prepared(&mut lifecycle, admission.clone());
        let ticket = admission.set(Some(9));
        assert!(
            lifecycle
                .gate(id(), expected, true, 9, ticket, &admission)
                .is_err()
        );
        assert!(lifecycle.control.gates.is_empty());
    }
}

#[test]
fn native_client_reopening_fences_idle_admission_until_a_fresh_dispatch() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    lifecycle.control.current = snapshot_native(
        PdoState::Present,
        0,
        5,
        false,
        11,
        (StreamState::Closed, true, true),
    );
    let old_stamp = stamp(lifecycle.poll()[0].snapshot.unwrap());
    let admission = Arc::new(Admission::new());
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    lifecycle
        .gate(id(), old_stamp, true, 9, ticket, &admission)
        .unwrap();
    let previous = lifecycle.control.current;
    lifecycle.control.current = snapshot(PdoState::Present, 1, 6, false);
    let (_, changed) =
        super::super::transport::poll(&mut lifecycle.control, previous, &session).unwrap();
    assert!(changed);
    assert!(!session.admitted());
    assert!(!lifecycle.control.current.gate_open());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 1);
    assert!(
        lifecycle
            .gate(id(), old_stamp, true, 9, ticket, &admission)
            .is_err()
    );
    assert_eq!(
        lifecycle.control.gates,
        vec![true],
        "never replay an old open"
    );
    let fresh_stamp = stamp(lifecycle.poll()[0].snapshot.unwrap());
    assert!(
        lifecycle
            .gate(id(), fresh_stamp, true, 8, ticket, &admission)
            .is_err()
    );
    lifecycle
        .gate(id(), fresh_stamp, true, 9, ticket, &admission)
        .unwrap();
    assert!(session.admitted());
    assert!(lifecycle.control.current.gate_open());
}

#[test]
fn every_snapshot_epoch_and_active_revision_are_checked_before_open() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let admission = Arc::new(Admission::new());
    prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    for index in 0..5 {
        let mut expected = [11, 7, 11, 3, 5];
        expected[index] += 1;
        assert!(
            lifecycle
                .gate(id(), expected, true, 9, ticket, &admission)
                .is_err()
        );
    }
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 8, ticket, &admission)
            .is_err()
    );
    lifecycle.control.current = snapshot(PdoState::Present, 4, 6, false);
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .is_err()
    );
    assert!(
        lifecycle.control.gates.is_empty(),
        "stale actions must never reach SET_GATE"
    );
}

#[test]
fn stop_racing_overlapped_open_closes_result_and_keeps_endpoint() {
    let admission = Arc::new(Admission::new());
    let mut lifecycle = Lifecycle::new(Fake {
        revoke_on_open: Some(admission.clone()),
        ..Fake::default()
    });
    lifecycle.create(spec()).unwrap();
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .is_err()
    );
    assert_eq!(lifecycle.control.gates, vec![true, false]);
    assert!(!session.admitted());
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Present
    );
    assert_eq!(lifecycle.control.remove_calls, 0);
}

#[test]
fn removal_requires_pcm_drain_and_keeps_tracking_on_timeout() {
    let mut lifecycle = Lifecycle::new(Fake {
        fail_remove: true,
        ..Fake::default()
    });
    lifecycle.create(spec()).unwrap();
    let session = prepared(&mut lifecycle, Arc::new(Admission::new()));
    assert!(lifecycle.retire(id(), [11, 7, 11, 3, 5]).is_err());
    assert_eq!(lifecycle.control.remove_calls, 0);
    session.drained.store(true, Ordering::Release);
    assert!(lifecycle.retire(id(), [11, 7, 11, 3, 5]).is_err());
    let views = lifecycle.poll();
    assert!(views[0].removing);
    assert_eq!(views[0].snapshot.unwrap().state(), PdoState::Present);
    let mut changed = spec();
    changed.allow_default = true;
    assert!(lifecycle.create(changed).is_err());
    assert_eq!(lifecycle.control.creates.len(), 1);
}

#[test]
fn failed_remove_retries_revoked_state_until_submission_is_acknowledged() {
    let mut lifecycle = Lifecycle::new(Fake {
        fail_remove: true,
        revoke_on_remove_failure: true,
        ..Fake::default()
    });
    lifecycle.create(spec()).unwrap();
    assert!(lifecycle.retire(id(), [11, 7, 11, 3, 5]).is_err());
    assert_eq!(lifecycle.control.current.state(), PdoState::Removing);
    let views = lifecycle.poll();
    assert_eq!(
        lifecycle.control.remove_calls, 2,
        "Removing reports revocation, not successful ACX submission"
    );
    assert!(
        views[0].error.is_some(),
        "QUERY must not hide a failed REMOVE"
    );
    assert!(views[0].removing);
    assert!(lifecycle.create(spec()).is_err());

    lifecycle.control.fail_remove = false;
    let views = lifecycle.poll();
    assert_eq!(lifecycle.control.remove_calls, 3);
    assert!(views[0].error.is_none());
    assert_eq!(views[0].snapshot.unwrap().state(), PdoState::Removing);
    for _ in 0..4 {
        lifecycle.poll();
    }
    assert_eq!(
        lifecycle.control.remove_calls, 3,
        "accepted asynchronous removal must only be observed, not resubmitted"
    );
    assert!(lifecycle.create(spec()).is_err());
    lifecycle.control.current = snapshot_generation(PdoState::Removed, 3, 5, false, 11);
    lifecycle.poll();
    lifecycle.create(spec()).unwrap();
}

#[test]
fn explicit_retry_of_failed_remove_is_not_suppressed_by_revoked_snapshot() {
    let mut lifecycle = Lifecycle::new(Fake {
        fail_remove: true,
        revoke_on_remove_failure: true,
        ..Fake::default()
    });
    lifecycle.create(spec()).unwrap();
    assert!(lifecycle.retire(id(), [11, 7, 11, 3, 5]).is_err());
    lifecycle.control.fail_remove = false;
    lifecycle.retire(id(), [11, 7, 11, 3, 5]).unwrap();
    assert_eq!(lifecycle.control.remove_calls, 2);
    lifecycle.retire(id(), [11, 7, 11, 3, 5]).unwrap();
    assert_eq!(lifecycle.control.remove_calls, 2);
}

#[test]
fn eligibility_and_name_recreate_only_after_observed_absence() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let mut changed = spec();
    changed.name = "Replacement name".into();
    changed.allow_default = true;
    assert!(lifecycle.create(changed.clone()).is_err());
    lifecycle.retire(id(), [11, 7, 11, 3, 5]).unwrap();
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Removing
    );
    assert!(lifecycle.create(changed.clone()).is_err());
    lifecycle.control.current = snapshot(PdoState::Removed, 3, 5, false);
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Removed
    );
    lifecycle.create(changed).unwrap();
    assert_eq!(lifecycle.control.creates.len(), 2);
    assert!(lifecycle.control.creates[1].allow_default);
    assert_eq!(lifecycle.control.creates[1].name, "Replacement name");
}

#[test]
fn deferred_create_can_retry_after_the_prior_session_child_is_destroyed() {
    let mut lifecycle = Lifecycle::new(Fake {
        defer_create: true,
        ..Fake::default()
    });
    assert!(lifecycle.create(spec()).is_err());
    assert!(lifecycle.create(spec()).is_err());
    assert_eq!(
        lifecycle.control.creates.len(),
        2,
        "a confirmed no-resource reply must not become an uncertain CREATE"
    );
    assert!(lifecycle.poll().is_empty(), "no new endpoint was created");
    lifecycle.control.defer_create = false;
    lifecycle.create(spec()).unwrap();
    assert_eq!(lifecycle.control.creates.len(), 3);
    let view = lifecycle.poll().remove(0);
    assert_eq!(view.snapshot.unwrap().state(), PdoState::Present);
    assert!(!view.snapshot.unwrap().gate_open());
    assert!(view.error.is_none());
}

#[test]
fn uncertain_create_is_tracked_and_never_retried_as_absent() {
    let mut lifecycle = Lifecycle::new(Fake {
        fail_create: true,
        ..Fake::default()
    });
    assert!(lifecycle.create(spec()).is_err());
    assert!(lifecycle.create(spec()).is_err());
    let views = lifecycle.poll();
    assert_eq!(views.len(), 1);
    assert!(views[0].snapshot.is_none());
    assert!(views[0].error.is_some());
    assert_eq!(lifecycle.control.creates.len(), 1);
}

#[test]
fn invalid_names_are_rejected_before_reserving_native_ownership() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    for name in [
        "".into(),
        "line\nbreak".into(),
        "x".repeat(64),
        "\u{1f3b5}".repeat(32),
    ] {
        let mut invalid = spec();
        invalid.name = name;
        assert!(lifecycle.create(invalid).is_err());
        assert!(lifecycle.endpoints.is_empty());
        assert!(lifecycle.control.creates.is_empty());
    }
    let mut valid = spec();
    valid.name = "x".repeat(63);
    lifecycle.create(valid).unwrap();
    assert_eq!(lifecycle.control.creates.len(), 1);
}

#[test]
fn worker_activation_never_opens_and_revoked_gate_close_resets_carrier() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    let mut control = Fake::default();
    let previous = control.current;
    let (current, changed) =
        super::super::transport::poll(&mut control, previous, &session).unwrap();
    assert!(!changed);
    assert!(!current.gate_open());
    assert!(control.gates.is_empty());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);

    control.current = snapshot(PdoState::Present, 3, 5, true);
    let (current, changed) =
        super::super::transport::poll(&mut control, current, &session).unwrap();
    assert!(changed);
    assert!(!current.gate_open());
    assert_eq!(control.gates, vec![false]);
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 1);
    assert_eq!(session.producer_quiesced_epoch.load(Ordering::Acquire), 0);
    assert_eq!(session.consumer_quiesced_epoch.load(Ordering::Acquire), 0);
    // Observation requests a reset; neither a control query nor gate closure
    // proves that a concurrent PCM publication and consumer discard finished.
}

#[test]
fn first_gate_admission_does_not_count_as_transport_recovery() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    let ticket = admission.set(Some(9));
    session.admitted_ticket.store(ticket, Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    let mut control = Fake {
        current: snapshot(PdoState::Present, 3, 5, false),
        ..Default::default()
    };
    let previous = control.current;
    control.current = snapshot(PdoState::Present, 3, 6, true);

    let (current, changed) =
        super::super::transport::poll(&mut control, previous, &session).unwrap();

    assert!(!changed, "initial admission is not recovery");
    assert!(current.gate_open());
    assert!(session.admitted());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);
    assert_eq!(session.telemetry.snapshot().recovery_events, 0);
}

#[test]
fn first_stream_start_after_gate_admission_does_not_count_as_recovery() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    let ticket = admission.set(Some(9));
    session.admitted_ticket.store(ticket, Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    let mut control = Fake {
        current: snapshot_native(
            PdoState::Present,
            4,
            8,
            false,
            11,
            (StreamState::Paused, true, true),
        ),
        ..Default::default()
    };
    let previous = control.current;
    control.current = snapshot_native(
        PdoState::Present,
        5,
        9,
        true,
        11,
        (StreamState::Running, true, true),
    );

    let (current, changed) = super::super::transport::poll_with_started_transport(
        &mut control,
        previous,
        &session,
        false,
    )
    .unwrap();

    assert!(!changed, "first stream start is not recovery");
    assert!(current.gate_open());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);
    assert_eq!(session.telemetry.snapshot().recovery_events, 0);
}

#[test]
fn admitted_worker_keeps_gate_but_stream_restart_revokes_and_resets() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    session
        .admitted_ticket
        .store(admission.set(Some(9)), Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    let mut control = Fake {
        current: snapshot(PdoState::Present, 3, 5, true),
        ..Default::default()
    };
    let previous = control.current;
    let (current, changed) =
        super::super::transport::poll(&mut control, previous, &session).unwrap();
    assert!(!changed);
    assert!(current.gate_open());
    assert!(control.gates.is_empty());

    control.current = snapshot(PdoState::Present, 4, 6, true);
    let (current, changed) =
        super::super::transport::poll(&mut control, current, &session).unwrap();
    assert!(changed);
    assert!(!current.gate_open());
    assert!(!session.admitted());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 1);
}

#[test]
fn native_carriers_cover_resampled_blocks_and_reject_invalid_preparation() {
    let request = VirtualBoundaryRequest {
        endpoint_id: id(),
        channels: 2,
        engine_sample_rate: 44_100,
        max_block_frames: 4096,
        max_demand_frames: 4096,
        buffer: Default::default(),
        audio_byte_budget: 1 << 20,
        owner_generation: 1,
    };
    for rate in [44_100, 48_000, 96_000] {
        let request = VirtualBoundaryRequest {
            engine_sample_rate: rate,
            ..request
        };
        let capacity = carrier_capacity(request).unwrap();
        let required = (4096_u64 * 48_000).div_ceil(u64::from(rate)) as usize + 32;
        assert!(capacity >= required);
        assert!(capacity >= crate::vad::MAX_PACKET_FRAMES as usize);
    }
    for invalid in [
        VirtualBoundaryRequest {
            channels: 0,
            ..request
        },
        VirtualBoundaryRequest {
            channels: 3,
            ..request
        },
        VirtualBoundaryRequest {
            engine_sample_rate: 0,
            ..request
        },
        VirtualBoundaryRequest {
            engine_sample_rate: 96_001,
            ..request
        },
        VirtualBoundaryRequest {
            max_block_frames: 0,
            ..request
        },
        VirtualBoundaryRequest {
            max_block_frames: usize::MAX,
            ..request
        },
        VirtualBoundaryRequest {
            audio_byte_budget: 0,
            ..request
        },
    ] {
        assert!(carrier_capacity(invalid).is_err());
    }
}

#[test]
fn accepted_create_must_be_observed_present_and_old_keys_cannot_open_replacement() {
    let mut lifecycle = Lifecycle::new(Fake {
        creating: true,
        ..Default::default()
    });
    lifecycle.create(spec()).unwrap();
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Creating
    );
    let admission = Arc::new(Admission::new());
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .is_err()
    );
    assert!(lifecycle.control.gates.is_empty());
    lifecycle.control.current = snapshot(PdoState::Present, 3, 5, false);
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Present
    );
    session.drained.store(true, Ordering::Release);
    lifecycle.retire(id(), [11, 7, 11, 3, 5]).unwrap();
    lifecycle.control.current = snapshot(PdoState::Removed, 3, 5, false);
    lifecycle.poll();
    lifecycle.control.creating = false;
    lifecycle.create(spec()).unwrap();
    prepared(&mut lifecycle, admission.clone());
    assert!(
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .is_err()
    );
    assert!(lifecycle.control.gates.is_empty());
}

#[test]
fn failed_observation_revokes_session_and_removal_retries_only_with_drain() {
    let admission = Arc::new(Admission::new());
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    lifecycle
        .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
        .unwrap();
    lifecycle.control.fail_query = true;
    let views = lifecycle.poll();
    assert!(views[0].error.is_some());
    assert!(!session.admitted());
    lifecycle.control.fail_query = false;
    lifecycle.control.current = snapshot(PdoState::Present, 3, 6, false);
    lifecycle.control.fail_remove = true;
    session.drained.store(true, Ordering::Release);
    lifecycle.poll();
    assert!(lifecycle.retire(id(), [11, 7, 11, 3, 6]).is_err());
    let calls = lifecycle.control.remove_calls;
    lifecycle.control.fail_remove = false;
    assert!(lifecycle.poll()[0].removing);
    assert_eq!(lifecycle.control.remove_calls, calls + 1);
    assert_eq!(
        lifecycle.poll()[0].snapshot.unwrap().state(),
        PdoState::Removing
    );
}

#[test]
fn query_failure_marks_cached_diagnostics_without_relabeling_preparation_refusal() {
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let admission = Arc::new(Admission::new());
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    lifecycle
        .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
        .unwrap();
    session.telemetry.submission_attempt(128, 64);
    session.telemetry.submission_accepted();
    let previous = lifecycle.poll().remove(0);
    assert_eq!(
        previous.diagnostic_provenance,
        VadExportDiagnosticProvenance::Fresh
    );
    assert_eq!(previous.telemetry.unwrap().submission_attempts, 1);
    assert!(session.admitted());

    // A drained worker leaves exactly the summary retained by the real poll.
    session.drained.store(true, Ordering::Release);
    lifecycle.control.fail_query = true;
    let failed = lifecycle.poll().remove(0);
    assert_eq!(
        failed.diagnostic_provenance,
        VadExportDiagnosticProvenance::CachedAfterQueryFailure
    );
    assert_eq!(failed.snapshot, previous.snapshot);
    assert_eq!(failed.telemetry, previous.telemetry);
    assert!(failed.error.is_some());
    assert!(!session.admitted());
    let observation = VadExportObservation::from(failed);
    assert_eq!(
        observation.diagnostic_provenance,
        VadExportDiagnosticProvenance::CachedAfterQueryFailure
    );
    assert_eq!(observation.telemetry, previous.telemetry);

    lifecycle.control.fail_query = false;
    lifecycle.control.current = snapshot_native_with_status(
        PdoState::Present,
        3,
        5,
        false,
        11,
        (StreamState::Closed, true, false),
        0xc000_000d,
    );
    let fresh = lifecycle.poll().remove(0);
    assert_eq!(
        fresh.diagnostic_provenance,
        VadExportDiagnosticProvenance::Fresh
    );
    assert_eq!(fresh.snapshot.unwrap().last_status(), 0xc000_000d);
    assert!(fresh.error.is_some());
    assert_eq!(fresh.telemetry, previous.telemetry);
    assert_eq!(
        VadExportObservation::from(fresh).diagnostic_provenance,
        VadExportDiagnosticProvenance::Fresh
    );
}

#[test]
fn pcm_cancellation_rebuilds_on_epoch_change_but_same_epoch_fault_is_fatal() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    session
        .admitted_ticket
        .store(admission.set(Some(9)), Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    let previous = snapshot(PdoState::Present, 3, 5, true);
    let mut control = Fake {
        current: previous,
        ..Default::default()
    };
    assert!(
        super::super::transport::after_io_error(
            &mut control,
            previous,
            &session,
            unavailable("native PCM fault"),
        )
        .is_err()
    );
    control.current = snapshot(PdoState::Present, 4, 6, false);
    let current = super::super::transport::after_io_error(
        &mut control,
        previous,
        &session,
        BackendError::Cancelled,
    )
    .unwrap();
    assert_eq!(current.stream_generation(), 4);
    assert!(!current.gate_open());
    assert_eq!(session.reset_epoch.load(Ordering::Acquire), 1);
}

#[test]
fn pcm_cancel_active_same_stamp_pause_closed_and_running_abort_stay_fatal() {
    for stream_state in [
        StreamState::Paused,
        StreamState::Closed,
        StreamState::Running,
    ] {
        let admission = Arc::new(Admission::new());
        let session = SessionState::new(admission.clone());
        session.active.store(true, Ordering::Release);
        session
            .admitted_ticket
            .store(admission.set(Some(9)), Ordering::Release);
        session.admitted_revision.store(9, Ordering::Release);
        let previous = snapshot(PdoState::Present, 3, 5, true);
        let current = snapshot_native(
            PdoState::Present,
            3,
            5,
            true,
            11,
            (stream_state, true, true),
        );
        let mut control = Fake {
            current,
            ..Default::default()
        };
        let result = super::super::transport::after_io_error(
            &mut control,
            previous,
            &session,
            BackendError::Cancelled,
        );
        assert!(
            matches!(result, Err(BackendError::Cancelled)),
            "native state alone does not prove stop intent: {stream_state:?}"
        );
        assert!(
            !session.drained.load(Ordering::Acquire),
            "retirement still has to drain requests"
        );
        assert_eq!(
            session.reset_epoch.load(Ordering::Acquire),
            0,
            "Pause is not a fabricated epoch"
        );
    }
}

#[test]
fn pcm_cancel_shutdown_race_needs_no_error_recovery_query_but_still_needs_drain() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission);
    session.active.store(true, Ordering::Release);
    session.cancel.store(true, Ordering::Release);
    let previous = snapshot(PdoState::Present, 3, 5, true);
    let mut control = Fake {
        fail_query: true,
        ..Default::default()
    };
    assert_eq!(
        super::super::transport::after_io_error(
            &mut control,
            previous,
            &session,
            BackendError::Cancelled,
        )
        .unwrap(),
        previous,
    );
    assert!(!session.drained.load(Ordering::Acquire));
    assert!(
        control.gates.is_empty(),
        "worker-exit gate close remains mandatory"
    );
}

#[test]
fn pcm_cancel_revoked_admission_closes_gate_without_claiming_a_drained_session() {
    let admission = Arc::new(Admission::new());
    let session = SessionState::new(admission.clone());
    session.active.store(true, Ordering::Release);
    session
        .admitted_ticket
        .store(admission.set(Some(9)), Ordering::Release);
    session.admitted_revision.store(9, Ordering::Release);
    admission.set(None);
    let previous = snapshot(PdoState::Present, 3, 5, true);
    let mut control = Fake {
        current: previous,
        ..Default::default()
    };
    let current = super::super::transport::after_io_error(
        &mut control,
        previous,
        &session,
        BackendError::Cancelled,
    )
    .unwrap();
    assert!(!current.gate_open());
    assert_eq!(control.gates, [false]);
    assert!(!session.drained.load(Ordering::Acquire));
}

#[test]
fn pcm_cancel_active_query_failure_and_prior_worker_failure_stay_visible_on_join() {
    let admission = Arc::new(Admission::new());
    let state = Arc::new(SessionState::new(admission.clone()));
    state.active.store(true, Ordering::Release);
    state
        .admitted_ticket
        .store(admission.set(Some(9)), Ordering::Release);
    state.admitted_revision.store(9, Ordering::Release);
    let previous = snapshot(PdoState::Present, 3, 5, true);
    let mut control = Fake {
        current: previous,
        fail_query: true,
        ..Default::default()
    };
    let error = super::super::transport::after_io_error(
        &mut control,
        previous,
        &state,
        BackendError::Cancelled,
    )
    .unwrap_err();
    assert_eq!(error, unavailable("QUERY timeout"));
    *state.failure.lock().unwrap() = Some(error.clone());
    state.status.store(2, Ordering::Release);
    let session = Box::new(VadSession {
        state: state.clone(),
        worker: None,
    });
    assert_eq!(session.stop_and_join(), Err(error));
    assert!(state.cancel.load(Ordering::Acquire));
    assert!(!state.drained.load(Ordering::Acquire));
}

#[test]
fn pcm_cancel_does_not_hide_distinct_io_fault_when_stopped_revoked_or_epoch_changes() {
    for situation in 0..4 {
        let admission = Arc::new(Admission::new());
        let session = SessionState::new(admission.clone());
        session.active.store(true, Ordering::Release);
        session
            .admitted_ticket
            .store(admission.set(Some(9)), Ordering::Release);
        session.admitted_revision.store(9, Ordering::Release);
        let previous = snapshot(PdoState::Present, 3, 5, true);
        let mut current = previous;
        match situation {
            0 => session.cancel.store(true, Ordering::Release),
            1 => session.revoke(),
            2 => {
                current = snapshot_native(
                    PdoState::Present,
                    3,
                    5,
                    true,
                    11,
                    (StreamState::Paused, true, true),
                )
            }
            _ => current = snapshot(PdoState::Present, 4, 6, false),
        }
        let mut control = Fake {
            current,
            ..Default::default()
        };
        let error = unavailable("distinct PCM device failure");
        assert_eq!(
            super::super::transport::after_io_error(
                &mut control,
                previous,
                &session,
                error.clone(),
            ),
            Err(error),
        );
        assert!(!session.drained.load(Ordering::Acquire));
    }
}

#[test]
fn stopped_and_drained_sessions_leave_native_endpoints_present_but_closed() {
    let admission = Arc::new(Admission::new());
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    lifecycle
        .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
        .unwrap();
    admission.set(None);
    let view = lifecycle.poll().remove(0);
    assert_eq!(view.snapshot.unwrap().state(), PdoState::Present);
    assert!(!view.snapshot.unwrap().gate_open());
    assert_eq!(lifecycle.control.remove_calls, 0);
    session.drained.store(true, Ordering::Release);
    // Even a last-known open gate after worker failure must be revoked.
    lifecycle.control.current = snapshot(PdoState::Present, 3, 7, true);
    assert!(!lifecycle.poll()[0].snapshot.unwrap().gate_open());
    assert_eq!(lifecycle.control.remove_calls, 0);
}

#[test]
fn dropping_prepared_session_before_attachment_cancels_and_joins_worker() {
    let state = Arc::new(SessionState::new(Arc::new(Admission::new())));
    let worker_state = state.clone();
    let worker = thread::spawn(move || {
        while !worker_state.cancel.load(Ordering::Acquire) {
            thread::park_timeout(Duration::from_millis(1));
        }
        worker_state.drained.store(true, Ordering::Release);
    });
    drop(VadSession {
        state: state.clone(),
        worker: Some(worker),
    });
    assert!(state.cancel.load(Ordering::Acquire));
    assert!(state.drained.load(Ordering::Acquire));
}
