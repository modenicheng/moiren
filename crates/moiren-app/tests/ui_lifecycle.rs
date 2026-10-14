use moiren_app::{
    service::*,
    ui::{Page, SelectionState, WindowPresentation},
};

#[test]
fn pure_presentation_hide_and_recreate_leaves_idle_service_untouched() {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let mut presentation = WindowPresentation::default();
    presentation.show();
    presentation.page = Page::Graph;
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
    assert_eq!(
        selections.output_index(&catalog(&["a"], "a")),
        None,
        "vanished identity stays selected but invalid"
    );
    assert_eq!(selections.output_index(&DeviceCatalog::default()), None);
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
    selection.select_process(&catalog, Some(0));
    assert_eq!(
        selection.input_selection(&catalog),
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
    assert_eq!(selection.process_index(&catalog), Some(1));
    catalog.processes[1].creation_time_100ns = u64::MAX;
    assert_eq!(selection.input_selection(&catalog), None);
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
