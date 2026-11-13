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
}

#[derive(Clone)]
pub(super) struct AbsentEntry {
    pub node: NodeId,
    pub configuration: NodeConfig,
    pub device: AbsentDevice,
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
}
