use moiren_app::{
    service::*,
    ui::{DetailSelection, SelectionState, WindowPresentation},
};

#[test]
fn pure_presentation_hide_and_recreate_leaves_idle_service_untouched() {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let mut presentation = WindowPresentation::default();
    presentation.show();
    presentation.page = 2;
    presentation.hide_to_tray();
    assert!(!presentation.visible);
    let mut replacement = WindowPresentation::default();
    replacement.show();
    assert!(replacement.visible && !replacement.minimized);
    assert_eq!(handle.try_snapshot().unwrap().phase, SessionPhase::Idle);
    handle.request_exit();
    runtime.join().unwrap();
}

fn catalog(ids: &[&str], default: &str) -> DeviceCatalog {
    DeviceCatalog {
        outputs: ids
            .iter()
            .map(|id| DeviceRow {
                endpoint_id: id.to_string(),
                name: id.to_string(),
            })
            .collect(),
        default_output_endpoint_id: Some(default.into()),
        ..Default::default()
    }
}

#[test]
fn defaults_and_refresh_preserve_identity_and_reject_vanished_selection() {
    let mut selections = SelectionState::default();
    selections.refresh(&catalog(&["a", "b"], "b"));
    assert_eq!(selections.output_id.as_deref(), Some("b"));
    selections.refresh(&catalog(&["b", "a"], "a"));
    assert_eq!(selections.output_id.as_deref(), Some("b"));
    selections.refresh(&catalog(&["a"], "a"));
    assert_eq!(selections.output_index(&catalog(&["a"], "a")), -1);
    selections.refresh(&DeviceCatalog::default());
    assert_eq!(selections.output_index(&DeviceCatalog::default()), -1);
}

#[test]
fn exact_process_identity_survives_reorder_but_not_pid_reuse() {
    let old = ProcessRow {
        pid: 42,
        creation_time_100ns: u64::MAX - 1,
        name: "old".into(),
    };
    let mut selection = SelectionState::default();
    let mut catalog = DeviceCatalog {
        processes: vec![old.clone()],
        ..Default::default()
    };
    selection.select_process(&catalog, 0);
    assert_eq!(
        selection.process_source(&catalog),
        Some(InputSelection::Process {
            pid: 42,
            creation_time_100ns: u64::MAX - 1
        })
    );
    catalog.processes.insert(
        0,
        ProcessRow {
            pid: 7,
            creation_time_100ns: 8,
            name: "other".into(),
        },
    );
    assert_eq!(selection.process_index(&catalog), 1);
    catalog.processes[1].creation_time_100ns = u64::MAX;
    assert_eq!(selection.process_source(&catalog), None);
}

#[test]
fn full_ordinary_mailbox_cannot_hide_exit() {
    let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
    while handle.try_request(AppRequest::RefreshCatalog).is_ok() {}
    assert_eq!(
        handle.try_request(AppRequest::Stop),
        Err(DispatchError::Busy)
    );
    handle.request_exit();
    assert!(mailbox.exit_requested());
    assert_eq!(
        handle.try_request(AppRequest::Stop),
        Err(DispatchError::Exiting)
    );
}

#[test]
fn inspector_follows_last_clicked_category_and_identity_without_mixing_endpoints() {
    let mut catalog = catalog(&["out-a", "out-b"], "out-a");
    catalog.inputs = vec![DeviceRow {
        endpoint_id: "in-a".into(),
        name: "Microphone".into(),
    }];
    let mut detail = DetailSelection::default();
    detail.select(false, "out-b".into());
    assert_eq!(detail.endpoint(&catalog).unwrap().endpoint_id, "out-b");
    detail.select(true, "in-a".into());
    assert_eq!(detail.endpoint(&catalog).unwrap().name, "Microphone");
    catalog.outputs.swap(0, 1);
    assert_eq!(detail.endpoint(&catalog).unwrap().endpoint_id, "in-a");
    catalog.inputs.clear();
    assert_eq!(detail.endpoint(&catalog), None);
    detail.select(false, "out-a".into());
    assert_eq!(detail.endpoint(&catalog).unwrap().endpoint_id, "out-a");
}
