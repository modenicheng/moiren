use super::support::*;
use moiren_engine::host::*;
use std::{sync::atomic::Ordering, time::Duration};

#[test]
fn lifecycle_open_runs_and_drop_joins_with_handles_alive() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let (project, _, _, _) = project();
    let host = EngineHost::<f32>::open(HostConfig::default(), project, backend).unwrap();
    let control = host.control();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running && snapshot.active_revision == Some(1)
    });
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    assert_eq!(control.edit(|_| Ok(())).err(), Some(HostError::Closed));
    assert_eq!(&*state.drops.lock().unwrap(), &["moiren-host-cleanup"]);
}

#[test]
fn lifecycle_no_output_is_dormant_without_preparation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), EngineProject::default(), backend).unwrap();
    wait(&host.observer(), |snapshot| {
        snapshot.host == HostState::Open
    });
    assert_eq!(state.prepares.load(Ordering::SeqCst), 0);
    assert_eq!(
        host.observer().snapshot().execution,
        ExecutionState::Dormant
    );
}

#[test]
fn lifecycle_prepare_panic_is_fault_and_close_still_completes() {
    let backend = FakeBackend::default();
    backend.0.panic_prepare.store(true, Ordering::SeqCst);
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project().0, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| snapshot.fault.is_some());
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(state.prepares.load(Ordering::SeqCst), 1);
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
}

#[test]
fn lifecycle_stale_initial_completion_never_starts_audio() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    state.hold_attach.store(true, Ordering::SeqCst);
    let (project, _, gain, _) = project();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    wait_count(&state.attaches, 1);
    let edit = host
        .control()
        .edit(move |project| {
            project
                .nodes
                .insert(gain, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })
        .unwrap();
    wait(&host.observer(), |snapshot| snapshot.desired_revision == 2);
    assert_eq!(state.starts.load(Ordering::SeqCst), 0);
    state.hold_attach.store(false, Ordering::SeqCst);
    assert!(matches!(
        edit.wait(TIMEOUT),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    assert_eq!(state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(state.attaches.load(Ordering::SeqCst), 2);
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_drop_during_initial_attach_revokes_activation() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    state.hold_attach.store(true, Ordering::SeqCst);
    let host = EngineHost::<f64>::open(HostConfig::default(), project().0, backend).unwrap();
    let observer = host.observer();
    wait_count(&state.attaches, 1);
    let closing = std::thread::spawn(move || drop(host));
    wait(&observer, |snapshot| snapshot.host == HostState::Closing);
    state.hold_attach.store(false, Ordering::SeqCst);
    closing.join().unwrap();
    assert_eq!(state.starts.load(Ordering::SeqCst), 0);
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    assert_eq!(observer.snapshot().host, HostState::Closed);
}

#[test]
fn lifecycle_cleanup_failure_remains_observable_after_closed() {
    let backend = FakeBackend::default();
    backend.0.fail_stop.store(true, Ordering::SeqCst);
    let host = EngineHost::<f64>::open(HostConfig::default(), project().0, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert_eq!(
        observer.snapshot().cleanup_error,
        Some(BackendError::Failed("injected stop failure".into()))
    );
}

#[test]
fn lifecycle_owner_released_inside_edit_closes_without_self_join() {
    let host = EngineHost::<f64>::open(
        HostConfig::default(),
        EngineProject::default(),
        FakeBackend::default(),
    )
    .unwrap();
    let observer = host.observer();
    let control = host.control();
    let receipt = control
        .edit(move |_| {
            drop(host);
            Ok(())
        })
        .unwrap();
    assert!(receipt.wait(TIMEOUT).is_terminal());
    let closed = wait(&observer, |snapshot| snapshot.host == HostState::Closed);
    assert!(closed.fault.is_none());
}

#[test]
fn lifecycle_cancellation_hook_panic_does_not_skip_remaining_wakes() {
    let cancellation = CancellationToken::default();
    let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    cancellation
        .on_cancel(|| panic!("injected hook panic"))
        .unwrap();
    let notified = wakes.clone();
    cancellation
        .on_cancel(move || {
            notified.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    cancellation.cancel();
    assert!(cancellation.is_cancelled());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(
        cancellation.on_cancel(|| panic!("late hook panic")),
        Err(BackendError::WorkerPanicked)
    );
}
