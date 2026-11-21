//! Control-side watch over endpoints that prepared as absent null streams.
//! A throttled probe re-resolves each absent device; the first one found
//! again publishes a `ResourceEvent`, and the supervisor's stop-and-reprepare
//! cycle replaces the null stream with the real device. Probe sweeps run on
//! short-lived threads with a shared deadline (audit X9): the host control
//! thread that polls this watch never runs an enumerator call itself, and a
//! wedged sweep is abandoned as unknown rather than blocking or faking a
//! verdict.
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use moiren_core::graph::NodeId;
use moiren_engine::audio_config::AudioBackend;
use moiren_engine::host::{NodeConfig, ResourceEvent};
use moiren_engine::node::EndpointSelector;

/// Endpoints whose devices are missing prepare as silent null streams; this
/// bounds how often the return probe hits the enumerator or the registry.
const ABSENCE_PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// Deadline for one off-thread probe sweep (audit X9): a wedged MMDevice
/// enumeration or COM call must never block the host control thread, so each
/// sweep runs on a short-lived probe thread and the poller abandons it at
/// this deadline. An abandoned sweep is unknown — it reports neither
/// presence nor absence, so no verdict is cleared and no event is emitted —
/// and the throttle holds the retry one interval back.
const ABSENCE_PROBE_DEADLINE: Duration = Duration::from_secs(2);

/// What the probe re-resolves for one absent IO node.
#[derive(Clone)]
pub(crate) struct AbsentDevice {
    pub selector: EndpointSelector,
    pub backend: AudioBackend,
    pub input: bool,
    /// The native id this endpoint last resolved to. `None` for endpoints
    /// that were missing at prepare time; `Some` for endpoints whose open
    /// hung past its deadline — the ["stuck"](#105) flavor of absence, which
    /// enumerates as present and must not be re-opened until the device is
    /// observed missing (a re-plug) or the session is re-applied — and for
    /// endpoints a running stream lost to device removal (the runtime-unplug
    /// flavor), which suppress re-opens the same way.
    pub native_id: Option<String>,
}

/// Session-scoped record of endpoints whose native open was abandoned after
/// its deadline (issue #105, decision D3). A stuck endpoint prepares as a
/// null stream and the absence watch refuses to report it back while it
/// still enumerates; observing it missing clears the record so the normal
/// absence recovery re-opens the device after a re-plug. A fresh backend
/// instance (every user apply builds one) starts with an empty record —
/// that is the manual retry.
#[derive(Default)]
pub(crate) struct StuckOpens {
    endpoints: std::sync::Mutex<std::collections::HashSet<(AudioBackend, String)>>,
}

impl StuckOpens {
    pub(crate) fn mark(&self, backend: AudioBackend, native_id: &str) {
        self.endpoints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((backend, native_id.to_owned()));
    }
    pub(crate) fn is_stuck(&self, backend: AudioBackend, native_id: &str) -> bool {
        self.endpoints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&(backend, native_id.to_owned()))
    }
    /// Forget one stuck verdict once the endpoint has been observed missing
    /// from enumeration — the re-plug signal that the device instance was
    /// re-created and may open promptly again.
    pub(crate) fn clear(&self, backend: AudioBackend, native_id: Option<&str>) {
        if let Some(id) = native_id {
            self.endpoints
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&(backend, id.to_owned()));
        }
    }
}

#[derive(Clone)]
pub(super) struct AbsentEntry {
    pub node: NodeId,
    pub configuration: NodeConfig,
    pub device: AbsentDevice,
}

/// One endpoint identity a worker reports when its running stream dies of
/// device removal: the selector the graph bound plus the native id the group
/// had opened (a duplex group reports one mark per direction).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct RuntimeAbsentMark {
    pub backend: AudioBackend,
    pub selector: EndpointSelector,
    pub native_id: String,
}

/// Session-scoped record of endpoints whose running native stream failed with
/// an absence-class error (issue #105 runtime unplug). Like [`StuckOpens`], a
/// marked endpoint prepares as a null stream and the absence watch refuses to
/// report it back while it still enumerates — the suppress verdict holds until
/// the endpoint is observed missing once (the re-plug signal that the device
/// instance was re-created) or the session is re-applied with a fresh backend.
/// Unlike a stuck open, the mark is keyed by the bound selector so a resolve
/// that observes the device already missing can consume it and pass its last
/// native id to the absence watch.
#[derive(Default)]
pub(crate) struct RuntimeAbsent {
    marks: std::sync::Mutex<Vec<RuntimeAbsentMark>>,
}

impl RuntimeAbsent {
    pub(crate) fn mark(&self, mark: RuntimeAbsentMark) {
        let mut marks = self.marks.lock().unwrap_or_else(|p| p.into_inner());
        match marks.iter().position(|existing| {
            existing.backend == mark.backend && existing.selector == mark.selector
        }) {
            Some(index) => marks[index] = mark,
            None => marks.push(mark),
        }
    }
    /// The marked native id for this selector, if the endpoint is suppressed.
    pub(crate) fn marked_id(
        &self,
        backend: AudioBackend,
        selector: &EndpointSelector,
    ) -> Option<String> {
        self.marks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|existing| existing.backend == backend && &existing.selector == selector)
            .map(|existing| existing.native_id.clone())
    }
    /// Is this exact (backend, selector, id) endpoint suppressed?
    pub(crate) fn is_marked(
        &self,
        backend: AudioBackend,
        selector: &EndpointSelector,
        native_id: &str,
    ) -> bool {
        self.marked_id(backend, selector)
            .is_some_and(|marked| marked == native_id)
    }
    /// Consume one selector's mark — the endpoint was just observed missing
    /// from enumeration, which is itself the re-plug-clearing signal. Returns
    /// the last native id so the absence watch can clear stale verdicts.
    pub(crate) fn take(
        &self,
        backend: AudioBackend,
        selector: &EndpointSelector,
    ) -> Option<String> {
        let mut marks = self.marks.lock().unwrap_or_else(|p| p.into_inner());
        marks
            .iter()
            .position(|existing| existing.backend == backend && &existing.selector == selector)
            .map(|index| marks.remove(index).native_id)
    }
    /// Forget every mark on one native id: the endpoint was observed missing,
    /// so a later sighting may open for real again. A duplex group's two
    /// selector marks share one id and clear together.
    pub(crate) fn clear_id(&self, backend: AudioBackend, native_id: Option<&str>) {
        let Some(native_id) = native_id else {
            return;
        };
        self.marks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|existing| !(existing.backend == backend && existing.native_id == native_id));
    }
}

/// What one native worker reports when its stream dies of device removal: the
/// shared registry plus its group's endpoint identities. The worker marks
/// before recording the failure, so the supervisor can never re-prepare
/// against an unmarked endpoint.
pub(crate) struct RuntimeAbsenceReport {
    registry: Arc<RuntimeAbsent>,
    marks: Vec<RuntimeAbsentMark>,
}

impl RuntimeAbsenceReport {
    pub(crate) fn new(registry: Arc<RuntimeAbsent>, marks: Vec<RuntimeAbsentMark>) -> Self {
        Self { registry, marks }
    }
    /// Mark every endpoint of this worker's group as runtime-absent.
    pub(crate) fn report(&self) {
        for mark in &self.marks {
            self.registry.mark(mark.clone());
        }
    }
}

/// What one probe sweep reports back from its thread: the index (into the
/// sweep's own entry snapshot) of the first entry whose device returned, or
/// that nothing returned this sweep.
enum SweepOutcome {
    Returned(usize),
    NothingReturned,
}

/// One sweep handed to a probe thread: the poller harvests or abandons it,
/// never waits for it. Dropping the receiver abandons the sweep; the probe
/// thread unwinds when its OS call finally returns and its send fails.
struct InFlight {
    /// The owner generation the sweep's indices refer to; a watch replaced
    /// by a newer (or equal) registration discards the sweep wholesale.
    owner_generation: u64,
    receiver: Receiver<SweepOutcome>,
    deadline: Instant,
}

/// The caller's per-device check, run on the probe thread. Returning `true`
/// reports the device back; the closure may also record the missing
/// sighting (its verdict clears) while it runs.
pub(super) type AbsenceCheck = Arc<dyn Fn(&AbsentDevice) -> bool + Send + Sync>;

pub(super) struct AbsenceMonitor {
    state: Arc<Mutex<State>>,
    /// Per-sweep abandonment deadline; production uses
    /// [`ABSENCE_PROBE_DEADLINE`], tests shorten it.
    probe_deadline: Duration,
}

impl Default for AbsenceMonitor {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            probe_deadline: ABSENCE_PROBE_DEADLINE,
        }
    }
}

#[derive(Default)]
struct State {
    owner_generation: u64,
    entries: Vec<AbsentEntry>,
    event_generation: u64,
    /// `None` probes immediately; a probe arms the interval.
    next_probe: Option<Instant>,
    /// The sweep currently running on its probe thread, if any.
    in_flight: Option<InFlight>,
}

impl AbsenceMonitor {
    /// Registers the absent set of one preparation. A newer or equal owner
    /// generation replaces the watch wholesale — including with an empty
    /// set, which retires probing for devices a later owner no longer
    /// references — and discards any in-flight sweep, whose indices refer
    /// to the replaced entries. Stale generations are ignored.
    pub(super) fn register(&self, owner_generation: u64, entries: Vec<AbsentEntry>) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if owner_generation < state.owner_generation {
            return;
        }
        state.owner_generation = owner_generation;
        state.entries = entries;
        // The registration itself just confirmed absence; a fresh watch may
        // probe right away.
        state.next_probe = None;
        state.in_flight = None;
    }

    /// Supervisor polling never waits (audit X9): the device probes run on
    /// short-lived probe threads with a shared deadline, exactly like the
    /// catalog's endpoint probes. Each poll either harvests a settled
    /// sweep — `probe` reported one absent device back, which releases its
    /// entry, so a failed re-prepare (the device flapped away again)
    /// re-registers and retries on a later sweep — or returns `None`
    /// without a verdict while a sweep is in flight, past its deadline
    /// (abandoned as unknown), or throttled.
    pub(super) fn poll(&self, probe: AbsenceCheck) -> Option<ResourceEvent> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        // Harvest the in-flight sweep first; the poller never waits on it.
        if let Some(flight) = state.in_flight.take() {
            if flight.owner_generation != state.owner_generation {
                // The watch was replaced; the sweep's verdicts are stale.
                return None;
            }
            match flight.receiver.try_recv() {
                Ok(SweepOutcome::Returned(index)) => {
                    let entry = state.entries.get(index)?.clone();
                    state.entries.remove(index);
                    let generation = state.event_generation.checked_add(1)?;
                    state.event_generation = generation;
                    return Some(ResourceEvent {
                        node: entry.node,
                        configuration: entry.configuration,
                        generation,
                        available: true,
                    });
                }
                Ok(SweepOutcome::NothingReturned) => return None,
                Err(TryRecvError::Empty) => {
                    if Instant::now() < flight.deadline {
                        // Still probing on its thread: no verdict this poll.
                        state.in_flight = Some(flight);
                    } else {
                        // Deadline passed: abandon the sweep (dropping the
                        // receiver). The probe thread unwinds when its OS
                        // call returns; the outcome is unknown — never
                        // dressed up as present or absent — and the armed
                        // throttle spaces the retry out.
                        log::warn!(
                            target: "moiren::audio::professional",
                            "absence probe sweep did not settle within {:?}; abandoning it without a verdict",
                            self.probe_deadline
                        );
                    }
                    return None;
                }
                Err(TryRecvError::Disconnected) => {
                    // The probe thread died before reporting; the sweep is
                    // inconclusive and the throttle holds the retry back.
                    return None;
                }
            }
        }
        if state.entries.is_empty() || state.next_probe.is_some_and(|next| Instant::now() < next) {
            return None;
        }
        state.next_probe = Some(Instant::now() + ABSENCE_PROBE_INTERVAL);
        let owner_generation = state.owner_generation;
        let entries = state.entries.clone();
        let (sender, receiver) = channel();
        let deadline = Instant::now() + self.probe_deadline;
        let thread_guard = moiren_engine::threads::register_thread(
            "moiren-absence-probe",
            moiren_engine::threads::ThreadClass::Control,
        );
        let spawned = std::thread::Builder::new()
            .name("moiren-absence-probe".to_owned())
            .spawn(move || {
                thread_guard.enter();
                // First returning entry wins, in registration order — the
                // same order the synchronous probe kept. The check (and its
                // missing-sighting verdict clears) runs on this thread, off
                // the host control thread.
                let outcome = entries
                    .iter()
                    .position(|entry| probe(&entry.device))
                    .map(SweepOutcome::Returned)
                    .unwrap_or(SweepOutcome::NothingReturned);
                let _ = sender.send(outcome);
            })
            .is_ok();
        if spawned {
            state.in_flight = Some(InFlight {
                owner_generation,
                receiver,
                deadline,
            });
        }
        // A spawn failure is inconclusive like an abandoned sweep: no
        // verdict, and the armed throttle spaces the retry out.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn node_id(index: usize) -> NodeId {
        let mut graph = LogicalGraph::new();
        let mut id = graph.create_node(NodeKind::Gain, 2).unwrap();
        for _ in 0..index {
            id = graph.create_node(NodeKind::Gain, 2).unwrap();
        }
        id
    }

    fn entry(node: NodeId, endpoint: &str) -> AbsentEntry {
        AbsentEntry {
            node,
            configuration: NodeConfig::Gain { initial: 0.5 },
            device: AbsentDevice {
                selector: EndpointSelector::PinnedEndpoint {
                    endpoint_id: endpoint.into(),
                    stable_id: None,
                },
                backend: AudioBackend::Wasapi,
                input: true,
                native_id: None,
            },
        }
    }

    fn never(_: &AbsentDevice) -> bool {
        false
    }

    /// Poll until the current sweep settles (an event is harvested, or the
    /// sweep finished/aborted without one) or the timeout passes. Fast test
    /// probes settle in milliseconds; the timeout only guards deadlock.
    fn poll_settled(
        monitor: &AbsenceMonitor,
        probe: AbsenceCheck,
        timeout: Duration,
    ) -> Option<ResourceEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = monitor.poll(probe.clone()) {
                return Some(event);
            }
            if monitor.state.lock().unwrap().in_flight.is_none() || Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn settle_timeout() -> Duration {
        Duration::from_secs(3)
    }

    #[test]
    fn an_empty_watch_never_probes_or_emits() {
        let monitor = AbsenceMonitor::default();
        monitor.register(1, Vec::new());
        assert!(monitor.poll(Arc::new(never)).is_none());
    }

    #[test]
    fn a_returning_device_emits_one_event_and_releases_its_entry() {
        let monitor = AbsenceMonitor::default();
        let first = entry(node_id(0), "{absent-a}");
        let second = entry(node_id(1), "{absent-b}");
        let expected_node = first.node;
        let expected_configuration = first.configuration.clone();
        let returned = first.device.selector.clone();
        let second_node = second.node;
        monitor.register(1, vec![first, second]);

        let nothing = Arc::new(never as fn(&AbsentDevice) -> bool);
        assert!(
            poll_settled(&monitor, nothing.clone(), settle_timeout()).is_none(),
            "nothing has returned yet"
        );
        monitor.state.lock().unwrap().next_probe = None;
        let matching: AbsenceCheck = Arc::new(move |device| device.selector == returned);
        let event = poll_settled(&monitor, matching.clone(), settle_timeout())
            .expect("the returning device emits an event");
        assert_eq!(event.node, expected_node);
        assert_eq!(event.configuration, expected_configuration);
        assert!(event.available);
        assert_eq!(event.generation, 1);

        // The released entry is gone; the remaining one still watches.
        monitor.state.lock().unwrap().next_probe = None;
        assert!(
            poll_settled(&monitor, matching.clone(), settle_timeout()).is_none(),
            "only the still-absent entry remains, and it has not returned"
        );
        let state = monitor.state.lock().unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].node, second_node);
    }

    #[test]
    fn a_stale_registration_cannot_replace_the_current_watch() {
        let monitor = AbsenceMonitor::default();
        monitor.register(2, vec![entry(node_id(0), "{absent-a}")]);
        monitor.register(1, vec![entry(node_id(1), "{absent-b}")]);
        let state = monitor.state.lock().unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].device.selector, {
            let expected = entry(node_id(0), "{absent-a}");
            expected.device.selector
        });
    }

    #[test]
    fn an_empty_newer_registration_retires_probing() {
        let monitor = AbsenceMonitor::default();
        monitor.register(2, vec![entry(node_id(0), "{absent-a}")]);
        monitor.register(3, Vec::new());
        assert!(monitor.state.lock().unwrap().entries.is_empty());
        assert!(monitor.poll(Arc::new(|_| true)).is_none());
    }

    #[test]
    fn events_increase_monotonically_across_registrations() {
        let monitor = AbsenceMonitor::default();
        let always: AbsenceCheck = Arc::new(|_| true);
        for generation in 1..=3u64 {
            monitor.register(generation, vec![entry(node_id(0), "{absent}")]);
            let event = poll_settled(&monitor, always.clone(), settle_timeout()).unwrap();
            assert_eq!(event.generation, generation);
        }
    }

    #[test]
    fn a_replaced_registration_discards_its_in_flight_sweep() {
        // A sweep for a replaced watch must never emit against the new
        // entries: registering a newer watch discards it wholesale, and
        // even its late "returned" verdict (sent to a dropped receiver)
        // changes nothing.
        let monitor = AbsenceMonitor::default();
        monitor.register(1, vec![entry(node_id(0), "{absent-a}")]);
        let (release, parked) = std::sync::mpsc::channel::<()>();
        let parked = Arc::new(std::sync::Mutex::new(parked));
        let blocking: AbsenceCheck = Arc::new(move |device| {
            let _ = parked.lock().unwrap().recv(); // parks while the test holds the sender
            let _ = device;
            true
        });
        assert!(monitor.poll(blocking).is_none(), "the wedged sweep starts");
        assert!(monitor.state.lock().unwrap().in_flight.is_some());
        // Replace the watch while the sweep is parked.
        monitor.register(2, vec![entry(node_id(1), "{absent-b}")]);
        assert!(
            monitor.state.lock().unwrap().in_flight.is_none(),
            "registering a new watch discards the in-flight sweep"
        );
        // Unpark the stale probe thread; its verdict goes nowhere and the
        // new watch probes normally.
        drop(release);
        let event = poll_settled(&monitor, Arc::new(|_| true), settle_timeout())
            .expect("the new watch probes its own entry");
        assert_eq!(event.generation, 1);
    }

    #[test]
    fn probing_is_throttled_to_the_interval() {
        let monitor = AbsenceMonitor::default();
        monitor.register(1, vec![entry(node_id(0), "{absent}")]);
        let probes = Arc::new(AtomicUsize::new(0));
        let counting: AbsenceCheck = {
            let probes = probes.clone();
            Arc::new(move |device| {
                probes.fetch_add(1, Ordering::SeqCst);
                let _ = device;
                false
            })
        };
        // The sweep runs once off-thread; the throttle holds the next one
        // back even though the watch is polled again right away.
        assert!(poll_settled(&monitor, counting.clone(), settle_timeout()).is_none());
        assert!(monitor.poll(counting.clone()).is_none());
        assert!(monitor.poll(counting.clone()).is_none());
        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_probe_runs_off_the_calling_thread() {
        // Audit X9: the enumerator call must never run on the host control
        // thread that polls the watch.
        let monitor = AbsenceMonitor::default();
        monitor.register(1, vec![entry(node_id(0), "{absent}")]);
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder: AbsenceCheck = {
            let seen = seen.clone();
            Arc::new(move |device| {
                *seen.lock().unwrap() = Some(std::thread::current().id());
                let _ = device;
                false
            })
        };
        assert!(poll_settled(&monitor, recorder.clone(), settle_timeout()).is_none());
        let probed_on = seen.lock().unwrap().expect("the probe ran");
        assert_ne!(
            probed_on,
            std::thread::current().id(),
            "the probe sweep must run on its own thread"
        );
    }

    #[test]
    fn a_wedged_sweep_is_abandoned_as_unknown_and_the_watch_retries() {
        // A stuck MMDevice call: the sweep misses its deadline, is
        // abandoned without a verdict (no event, no entry released), and
        // the next sweep after the throttle may report normally.
        let monitor = AbsenceMonitor {
            state: Arc::default(),
            probe_deadline: Duration::from_millis(100),
        };
        monitor.register(1, vec![entry(node_id(0), "{absent}")]);
        let (release, wedged) = std::sync::mpsc::channel::<()>();
        let wedged = Arc::new(std::sync::Mutex::new(wedged));
        let blocking: AbsenceCheck = Arc::new(move |device| {
            let _ = wedged.lock().unwrap().recv(); // parks like a wedged COM call
            let _ = device;
            true
        });
        // Start the wedged sweep, then wait out the deadline.
        assert!(monitor.poll(blocking.clone()).is_none());
        assert!(monitor.state.lock().unwrap().in_flight.is_some());
        std::thread::sleep(Duration::from_millis(200));
        // Past the deadline the sweep is abandoned as unknown: no event,
        // no verdict, nothing released.
        assert!(monitor.poll(blocking.clone()).is_none());
        assert!(
            monitor.state.lock().unwrap().in_flight.is_none(),
            "the wedged sweep is abandoned at its deadline"
        );
        // Release the wedged probe so its thread can unwind.
        drop(release);
        // The entry survives the unknown outcome; the next sweep (throttle
        // bypassed like earlier tests) reports it normally.
        monitor.state.lock().unwrap().next_probe = None;
        let event = poll_settled(&monitor, Arc::new(|_| true), settle_timeout())
            .expect("the watch retries after an abandoned sweep");
        assert!(event.available);
    }

    #[test]
    fn stuck_records_survive_until_observed_missing() {
        let stuck = StuckOpens::default();
        stuck.mark(AudioBackend::Wasapi, "{stuck-device}");
        assert!(stuck.is_stuck(AudioBackend::Wasapi, "{stuck-device}"));
        // A different endpoint or backend is unaffected.
        assert!(!stuck.is_stuck(AudioBackend::Wasapi, "{other}"));
        assert!(!stuck.is_stuck(AudioBackend::Asio, "{stuck-device}"));
        // Clearing without a native id is a no-op (plain absent entries).
        stuck.clear(AudioBackend::Wasapi, None);
        assert!(stuck.is_stuck(AudioBackend::Wasapi, "{stuck-device}"));
        // Observing the endpoint missing forgets the verdict; a fresh mark
        // can be recorded again after another timed-out open.
        stuck.clear(AudioBackend::Wasapi, Some("{stuck-device}"));
        assert!(!stuck.is_stuck(AudioBackend::Wasapi, "{stuck-device}"));
        stuck.mark(AudioBackend::Wasapi, "{stuck-device}");
        assert!(stuck.is_stuck(AudioBackend::Wasapi, "{stuck-device}"));
    }

    fn runtime_mark(endpoint: &str, native_id: &str) -> RuntimeAbsentMark {
        RuntimeAbsentMark {
            backend: AudioBackend::Wasapi,
            selector: EndpointSelector::PinnedEndpoint {
                endpoint_id: endpoint.into(),
                stable_id: None,
            },
            native_id: native_id.into(),
        }
    }

    #[test]
    fn runtime_marks_suppress_reopens_until_observed_missing_or_moved() {
        let runtime = RuntimeAbsent::default();
        let mark = runtime_mark("{unplugged}", "{native-a}");
        runtime.mark(mark.clone());
        // The same endpoint and id is suppressed; another selector, another
        // id, or another backend is not.
        assert!(runtime.is_marked(AudioBackend::Wasapi, &mark.selector, "{native-a}"));
        assert!(!runtime.is_marked(AudioBackend::Wasapi, &mark.selector, "{native-b}"));
        assert!(!runtime.is_marked(AudioBackend::Asio, &mark.selector, "{native-a}"));
        // A new failure of the same selector replaces the id (a followed
        // default that resolved to a different device before dying).
        runtime.mark(runtime_mark("{unplugged}", "{native-b}"));
        assert!(!runtime.is_marked(AudioBackend::Wasapi, &mark.selector, "{native-a}"));
        assert!(runtime.is_marked(AudioBackend::Wasapi, &mark.selector, "{native-b}"));
        // Observing the endpoint missing consumes the mark and returns its
        // last native id for the stale-verdict clears.
        assert_eq!(
            runtime.take(AudioBackend::Wasapi, &mark.selector),
            Some("{native-b}".into())
        );
        assert_eq!(runtime.take(AudioBackend::Wasapi, &mark.selector), None);
        // clear_id forgets every selector marked on one native id — a duplex
        // group's two directions clear together.
        runtime.mark(runtime_mark("{input}", "{native-duplex}"));
        runtime.mark(RuntimeAbsentMark {
            backend: AudioBackend::Wasapi,
            selector: EndpointSelector::PinnedEndpoint {
                endpoint_id: "{output}".into(),
                stable_id: None,
            },
            native_id: "{native-duplex}".into(),
        });
        runtime.clear_id(AudioBackend::Wasapi, Some("{native-duplex}"));
        assert_eq!(
            runtime.marked_id(AudioBackend::Wasapi, &mark.selector),
            None
        );
        assert_eq!(
            runtime.marked_id(
                AudioBackend::Wasapi,
                &EndpointSelector::PinnedEndpoint {
                    endpoint_id: "{input}".into(),
                    stable_id: None,
                }
            ),
            None
        );
        // Without a native id there is nothing to clear.
        runtime.mark(runtime_mark("{input}", "{native-duplex}"));
        runtime.clear_id(AudioBackend::Wasapi, None);
        assert!(runtime.is_marked(
            AudioBackend::Wasapi,
            &runtime_mark("{input}", "{native-duplex}").selector,
            "{native-duplex}"
        ));
    }
}
