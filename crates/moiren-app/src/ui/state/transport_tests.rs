//! Transport control tests: Start is start-by-graph (design 23 §4.2). The
//! UI keeps no validation of its own — the idle/failed phases dispatch
//! Apply, a running session has no stop action, and the service's verdict
//! is the only one.

use super::*;
use crate::service::ServiceMailbox;
use moiren_engine::node::VirtualEndpointId;

/// Publishes a phase change the way the service would and polls it in.
fn publish_phase(state: &mut UiState, mailbox: &ServiceMailbox, phase: SessionPhase) {
    let mut snapshot = (*state.snapshot).clone();
    snapshot.phase = phase;
    mailbox.publish(snapshot);
    state.poll();
}

fn ui_with_phase(phase: SessionPhase) -> (UiState, ServiceMailbox) {
    let context = crate::service::ServiceContext::default();
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let mut initial = (*handle.try_snapshot().unwrap()).clone();
    initial.phase = phase;
    (UiState::new(handle, Arc::new(initial)), mailbox)
}

#[test]
fn transport_toggles_dispatch_apply_and_running_is_left_alone() {
    for phase in [SessionPhase::Idle, SessionPhase::Failed] {
        let (mut state, mailbox) = ui_with_phase(phase);
        state.toggle_transport();
        assert_eq!(
            mailbox.try_request(),
            Some(AppRequest::ApplyGraph),
            "{phase:?} starts by graph"
        );

        publish_phase(&mut state, &mailbox, SessionPhase::Running);
        state.toggle_transport();
        assert_eq!(mailbox.try_request(), None, "always-on: no stop request");

        publish_phase(&mut state, &mailbox, SessionPhase::Starting);
        state.toggle_transport();
        assert_eq!(mailbox.try_request(), None);
    }
}

/// The old ledger turned Start into a no-op whenever the selected device was
/// missing from the catalog (audit fact F12's mute grey). Start-by-graph
/// always sends the request: an absent device is the service's judgment to
/// make, and its answer lands as node faults.
#[test]
fn start_is_dispatched_even_when_the_catalog_has_no_devices() {
    let mut context = crate::service::ServiceContext::default();
    context.catalog = Arc::new(DeviceCatalog::default());
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let initial = handle.try_snapshot().unwrap();
    let mut state = UiState::new(handle, initial);
    state.toggle_transport();
    assert_eq!(mailbox.try_request(), Some(AppRequest::ApplyGraph));
    assert!(
        state.dispatch_error.is_none(),
        "no UI-side verdict precedes the service's"
    );
}

#[test]
fn transport_ignores_transitional_phases() {
    let (mut state, mailbox) = ui_with_phase(SessionPhase::Stopping);
    state.toggle_transport();
    assert_eq!(mailbox.try_request(), None);
}

#[test]
fn physical_device_rows_hide_only_exact_vad_mmdevice_ids() {
    let row = |id: &str| DeviceRow {
        endpoint_id: id.into(),
        name: id.into(),
    };
    let catalog = DeviceCatalog {
        inputs: vec![row("physical-input"), row("vad-mmdevice")],
        outputs: vec![row("physical-output"), row("vad-mmdevice")],
        vad_endpoints: [(
            VirtualEndpointId::from_bytes([0x42; 16]),
            "vad-mmdevice".into(),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    assert_eq!(
        super::input_rows(&catalog, AudioBackend::Wasapi)
            .into_iter()
            .map(|row| row.endpoint_id)
            .collect::<Vec<_>>(),
        vec!["physical-input"]
    );
    assert_eq!(
        super::output_rows(&catalog, AudioBackend::Wasapi)
            .into_iter()
            .map(|row| row.endpoint_id)
            .collect::<Vec<_>>(),
        vec!["physical-output"]
    );
}
