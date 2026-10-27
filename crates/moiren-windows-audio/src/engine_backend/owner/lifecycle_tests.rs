//! Control-side ownership checks shared with the isolated native RT harness.
use super::*;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Probe {
    running: AtomicBool,
    cancelled: AtomicUsize,
    joined: AtomicUsize,
    dropped: AtomicUsize,
}

struct Auxiliary {
    state: Arc<RunState>,
    probe: Arc<Probe>,
    cancel_on_activate: bool,
}

impl AuxiliaryOwner for Auxiliary {
    fn activate(&self) -> Result<(), BackendError> {
        self.probe.running.store(true, Ordering::Release);
        if self.cancel_on_activate {
            self.state.cancel();
        }
        Ok(())
    }

    fn cancel(&self) {
        self.probe.running.store(false, Ordering::Release);
        self.probe.cancelled.fetch_add(1, Ordering::AcqRel);
    }

    fn running(&self) -> bool {
        self.probe.running.load(Ordering::Acquire)
    }

    fn finished(&self) -> bool {
        false
    }

    fn join(&mut self) -> Result<(), BackendError> {
        self.probe.joined.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

impl Drop for Auxiliary {
    fn drop(&mut self) {
        self.probe.dropped.fetch_add(1, Ordering::AcqRel);
    }
}

fn transport(cancel_on_activate: bool) -> (Box<Transport<f32>>, Arc<Probe>) {
    let mut transport = Box::new(Transport::new(1, ProfessionalObservations::default()));
    let probe = Arc::new(Probe::default());
    transport.auxiliaries.push(Box::new(Auxiliary {
        state: transport.state.clone(),
        probe: probe.clone(),
        cancel_on_activate,
    }));
    (transport, probe)
}

#[test]
fn cancellation_during_auxiliary_activation_rolls_back_started_resources() {
    let (transport, probe) = transport(true);
    assert_eq!(transport.activate(), Err(BackendError::Cancelled));
    assert!(
        probe.cancelled.load(Ordering::Acquire) > 0,
        "failed native activation must revoke auxiliary admission before returning"
    );
    assert!(!probe.running.load(Ordering::Acquire));
    drop(transport);
    assert_eq!(probe.joined.load(Ordering::Acquire), 1);
    assert_eq!(probe.dropped.load(Ordering::Acquire), 1);
}

#[test]
fn capture_follower_activates_and_retires_without_an_engine_handoff() {
    let (transport, probe) = transport(false);
    let slot = transport.slot.clone();
    let cancel = CancellationToken::default();
    let follower = transport.into_follower(cancel).unwrap();
    assert_eq!(follower.status(), OwnerStatus::Starting);
    follower.activate().unwrap();
    assert_eq!(follower.status(), OwnerStatus::Running);
    assert!(slot.take().is_none(), "only the graph owner holds Engine");
    follower.cancel();
    follower.stop_and_join().unwrap();
    assert_eq!(probe.joined.load(Ordering::Acquire), 1);
    assert_eq!(probe.dropped.load(Ordering::Acquire), 1);
    assert!(!probe.running.load(Ordering::Acquire));
}

#[test]
fn a_render_transport_cannot_become_a_virtual_clock_follower() {
    let (mut transport, probe) = transport(false);
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let sink = graph
        .create_node(moiren_core::graph::NodeKind::Sink, 2)
        .unwrap();
    transport.sinks.push(sink);
    assert!(matches!(
        transport.into_follower(CancellationToken::default()),
        Err(BackendError::Unsupported(_))
    ));
    assert_eq!(probe.joined.load(Ordering::Acquire), 1);
    assert_eq!(probe.dropped.load(Ordering::Acquire), 1);
    assert!(!probe.running.load(Ordering::Acquire));
}
