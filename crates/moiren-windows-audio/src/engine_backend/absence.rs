//! Control-side watch over endpoints that prepared as absent null streams.
//! A throttled probe re-resolves each absent device; the first one found
//! again publishes a `ResourceEvent`, and the supervisor's stop-and-reprepare
//! cycle replaces the null stream with the real device.
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use moiren_core::graph::NodeId;
use moiren_engine::audio_config::AudioBackend;
use moiren_engine::host::{NodeConfig, ResourceEvent};
use moiren_engine::node::EndpointSelector;

/// Endpoints whose devices are missing prepare as silent null streams; this
/// bounds how often the return probe hits the enumerator or the registry.
const ABSENCE_PROBE_INTERVAL: Duration = Duration::from_secs(1);

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

#[derive(Default)]
pub(super) struct AbsenceMonitor {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    owner_generation: u64,
    entries: Vec<AbsentEntry>,
    event_generation: u64,
    /// `None` probes immediately; a probe arms the interval.
    next_probe: Option<Instant>,
}

impl AbsenceMonitor {
    /// Registers the absent set of one preparation. A newer or equal owner
    /// generation replaces the watch wholesale — including with an empty
    /// set, which retires probing for devices a later owner no longer
    /// references. Stale generations are ignored.
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
    }

    /// Supervisor polling never waits. `probe` reports whether one absent
    /// device has come back; a returned event releases its entry, so a
    /// failed re-prepare (the device flapped away again) re-registers and
    /// retries on a later probe.
    pub(super) fn poll(&self, probe: impl Fn(&AbsentDevice) -> bool) -> Option<ResourceEvent> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        if state.entries.is_empty() || state.next_probe.is_some_and(|next| Instant::now() < next) {
            return None;
        }
        state.next_probe = Some(Instant::now() + ABSENCE_PROBE_INTERVAL);
        let index = state
            .entries
            .iter()
            .position(|entry| probe(&entry.device))?;
        let entry = state.entries.remove(index);
        let generation = state.event_generation.checked_add(1)?;
        state.event_generation = generation;
        Some(ResourceEvent {
            node: entry.node,
            configuration: entry.configuration,
            generation,
            available: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};

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

    #[test]
    fn an_empty_watch_never_probes_or_emits() {
        let monitor = AbsenceMonitor::default();
        monitor.register(1, Vec::new());
        assert!(monitor.poll(never).is_none());
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

        assert!(monitor.poll(never).is_none(), "nothing has returned yet");
        monitor.state.lock().unwrap().next_probe = None;
        let event = monitor
            .poll(|device| device.selector == returned)
            .expect("the returning device emits an event");
        assert_eq!(event.node, expected_node);
        assert_eq!(event.configuration, expected_configuration);
        assert!(event.available);
        assert_eq!(event.generation, 1);

        // The released entry is gone; the remaining one still watches.
        monitor.state.lock().unwrap().next_probe = None;
        assert!(
            monitor.poll(|device| device.selector == returned).is_none(),
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
        assert!(monitor.poll(|_| true).is_none());
    }

    #[test]
    fn events_increase_monotonically_across_registrations() {
        let monitor = AbsenceMonitor::default();
        for generation in 1..=3u64 {
            monitor.register(generation, vec![entry(node_id(0), "{absent}")]);
            let event = monitor.poll(|_| true).unwrap();
            assert_eq!(event.generation, generation);
        }
    }

    #[test]
    fn probing_is_throttled_to_the_interval() {
        use std::cell::Cell;
        let monitor = AbsenceMonitor::default();
        monitor.register(1, vec![entry(node_id(0), "{absent}")]);
        let probes = Cell::new(0usize);
        let counting = |device: &AbsentDevice| {
            probes.set(probes.get() + 1);
            let _ = device;
            false
        };
        // The first poll probes; the throttle holds the second back.
        assert!(monitor.poll(counting).is_none());
        assert!(monitor.poll(counting).is_none());
        assert_eq!(probes.get(), 1);
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
