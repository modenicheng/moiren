//! Demand-gated, bounded copies from confirmed engine spectra. All storage and
//! subscription changes happen on the service thread, never on audio callbacks.
use super::*;
use moiren_engine::host::{HostId, ManagedSpectrumSubscription};
use moiren_signal::{OpSignalBindings, PreparedPublisher, RtHost};

const COPY_BUDGET: usize = super::super::signals::MAX_SPECTRUM_COPY_BYTES;
const COPIES_PER_POLL: usize = 4;

struct Slot {
    node: NodeId,
    binding: SpectrumBinding,
    host: RtHost,
    operation: OpSignalBindings,
    publisher: PreparedPublisher<SpectrumTopic>,
    subscription: Option<ManagedSpectrumSubscription>,
    demand_generation: u64,
    after: Option<host::OwnerTimeline>,
}

impl Slot {
    fn new(c: &mut ServiceContext, node: NodeId, binding: SpectrumBinding) -> Result<Self, String> {
        let (control, mut host) = c.signals.create_spectrum_host()?;
        let key = control.new_binding().map_err(|e| e.to_string())?;
        let publisher = control
            .prepare(&binding.topic, key)
            .map_err(|e| e.to_string())?;
        let update = control
            .prepare_update(&[publisher.erase()])
            .map_err(|e| e.to_string())?;
        let operation = update.op_bindings(key).map_err(|e| e.to_string())?;
        drop(host.activate(update).map_err(|e| e.error.to_string())?);
        Ok(Self {
            node,
            binding,
            host,
            operation,
            publisher,
            subscription: None,
            demand_generation: 0,
            after: None,
        })
    }

    fn copy(&mut self, observer: &EngineObserver, snapshot: &host::ProjectSnapshot) -> bool {
        let Some(timeline) = snapshot.state.timeline else {
            return false;
        };
        if self.subscription.is_none() {
            self.subscription = observer
                .subscribe_spectrum(self.node)
                .ok()
                .filter(|s| s.owner_generation == self.binding.owner_generation);
            // Hop-only updates can reuse the engine topic and seed its cached
            // payload. Require a complete window captured after this subscription.
            self.after = Some(timeline);
        }
        let Some(subscription) = &mut self.subscription else {
            return false;
        };
        let Some(window) = subscription.subscriber.read_latest() else {
            return false;
        };
        let header = window.header();
        let Some(after) = self.after else {
            return false;
        };
        if header.channels != self.binding.channels
            || header.fft_size != self.binding.settings.fft_size
            || window.elements().len() != self.publisher.capacity()
            || header.timeline_epoch != timeline.epoch
            || header.timeline_epoch != after.epoch
            || header.frame_start < after.frame
            || header.frame_end.checked_sub(header.frame_start) != Some(header.fft_size as u64)
            || snapshot
                .active
                .as_ref()
                .is_none_or(|p| header.sample_rate != p.processing.engine.processing_sr)
        {
            return false;
        }
        let current = observer.read();
        if current.host_id != snapshot.host_id
            || current.state.owner_generation != snapshot.state.owner_generation
            || current.state.active_revision != snapshot.state.active_revision
            || current.state.execution != ExecutionState::Running
        {
            return false;
        }
        self.host.begin_block();
        let mut scope = self.host.scope(&self.operation);
        let Ok(mut write) = scope.try_write(&self.publisher.handle(), window.elements().len())
        else {
            return false;
        };
        *write.header_mut() = *header;
        write.elements_mut().copy_from_slice(window.elements());
        write.commit(window.elements().len()).is_ok()
    }
}

#[derive(Default)]
pub(super) struct SpectrumBridge {
    owner: Option<(HostId, u64)>,
    slots: Vec<Slot>,
    cursor: usize,
}

impl SpectrumBridge {
    pub fn clear(&mut self, c: &mut ServiceContext) {
        self.slots.clear();
        let nodes: Vec<_> = c.signals.topics.node_spectra.keys().copied().collect();
        for node in nodes {
            if let Err(error) = c.signals.unregister_spectrum(node) {
                c.graph
                    .record_error(format!("Node spectrum retirement: {error}"));
            }
        }
        self.owner = None;
        self.cursor = 0;
    }

    pub fn poll(&mut self, c: &mut ServiceContext, observer: &EngineObserver) {
        let snapshot = observer.read();
        let Some(active) = snapshot
            .active
            .as_ref()
            .filter(|_| snapshot.state.execution == ExecutionState::Running)
        else {
            self.clear(c);
            return;
        };
        let owner = (snapshot.host_id, snapshot.state.owner_generation);
        if self.owner != Some(owner) {
            self.clear(c);
            self.owner = Some(owner);
        }
        if let Err(error) = self.sync(c, active, owner.1) {
            c.graph.record_error(format!("Node spectra: {error}"));
        }
        // Always release every hidden upstream, even when large copies exhaust
        // this poll's budget. Detect hide/show between polls through demand epochs.
        for slot in &mut self.slots {
            let demand = slot.binding.topic.demand_snapshot();
            if !demand.active || demand.generation != slot.demand_generation {
                slot.subscription = None;
                slot.after = None;
                slot.demand_generation = demand.generation;
            }
        }
        let mut remaining = COPY_BUDGET;
        let mut copies = 0;
        for _ in 0..self.slots.len() {
            self.cursor %= self.slots.len();
            let slot = &mut self.slots[self.cursor];
            let bytes = slot.publisher.capacity() * size_of::<f32>() + 128;
            if slot.binding.topic.business_subscribers() != 0 {
                // Retry this slot first next poll; a continuously publishing
                // smaller predecessor cannot consume its allowance indefinitely.
                if bytes > remaining || copies == COPIES_PER_POLL {
                    break;
                }
                if slot.copy(observer, &snapshot) {
                    remaining -= bytes;
                    copies += 1;
                }
            }
            self.cursor = (self.cursor + 1) % self.slots.len();
        }
    }

    fn sync(
        &mut self,
        c: &mut ServiceContext,
        active: &host::EngineProject,
        generation: u64,
    ) -> Result<(), String> {
        let stale: Vec<_> = c.signals.topics.node_spectra.iter().filter_map(|(&node, binding)| {
            let matches = active.graph.get_node(node).is_ok_and(|n| n.channels() == binding.channels)
                && matches!(active.nodes.get(&node), Some(host::NodeConfig::SpectrumAnalyzer(settings)) if *settings == binding.settings)
                && generation == binding.owner_generation;
            (!matches).then_some(node)
        }).collect();
        self.slots.retain(|slot| !stale.contains(&slot.node));
        for node in stale {
            c.signals.unregister_spectrum(node)?;
        }
        for node in active.graph.nodes() {
            let Some(host::NodeConfig::SpectrumAnalyzer(settings)) = active.nodes.get(&node.id())
            else {
                continue;
            };
            if self.slots.iter().any(|slot| slot.node == node.id()) {
                continue;
            }
            c.signals.unregister_spectrum(node.id())?;
            let binding =
                c.signals
                    .register_spectrum(node.id(), *settings, node.channels(), generation)?;
            match Slot::new(c, node.id(), binding) {
                Ok(slot) => self.slots.push(slot),
                Err(error) => {
                    c.signals.unregister_spectrum(node.id())?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}
