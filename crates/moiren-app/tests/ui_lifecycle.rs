mod common;

use moiren_app::{
    service::*,
    ui::{Page, WindowPresentation},
};

#[test]
fn pure_presentation_hide_and_recreate_leaves_idle_service_untouched() {
    let runtime = common::service();
    let handle = runtime.handle();
    let mut presentation = WindowPresentation::default();
    presentation.show();
    presentation.page = Page::Graph;
    presentation.hide_to_tray();
    assert!(!presentation.visible);
    let mut replacement = WindowPresentation::default();
    replacement.show();
    assert!(replacement.visible && !replacement.minimized);
    // Reads are intentionally nonblocking and may race the service publisher.
    let snapshot = common::wait_snapshot(&handle);
    assert_eq!(snapshot.phase, SessionPhase::Idle);
    common::finish(runtime);
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
