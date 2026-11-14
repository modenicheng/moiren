//! Meter signal delivery. The spectrum half is a relay: it reads the
//! engine's latest exact source windows, transforms them here on the
//! service thread, and publishes magnitudes onto the stable application
//! topics. Levels are not relayed at all (audit 2026-10-27 X-1 and X-3,
//! decision D-4 route B): a [`LevelSource`] holds the engine's
//! [`ManagedLevelSubscription`] directly and hands it to UI consumers as a
//! [`LevelLease`], so a level window is copied exactly once — by the
//! engine bus pump into the subscription — and the relay slot's
//! generation re-subscription rule runs here verbatim. Demand still
//! decides whether RT meters run: a source nobody subscribes to holds no
//! engine subscription, the engine bus idles, and the realtime meter
//! skips its capture.
use super::*;
use crate::service::signals::{SignalTopics, SpectrumShape};
use moiren_core::graph::NodeKind;
use moiren_engine::host::{ManagedLevelSubscription, ManagedSpectrumSubscription};
use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_engine::spectrum::SpectrumAnalysis;
use moiren_signal::{BindingKey, HostControl, OpSignalBindings, PreparedPublisher, RtHost, Topic};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// One stable level subscription point for one meter tap. Handles clone
/// into snapshots and live across sessions exactly as the application
/// topics of the retired relay did; the running registry binds the current
/// engine subscription underneath. Equality is handle identity: two
/// handles for the same tap are equal, and a rebuilt tap never is, which
/// is what makes the UI's subscription tables rebuild on a re-bind.
#[derive(Clone)]
pub struct LevelSource {
    shared: Arc<SourceShared>,
}

struct SourceShared {
    /// Live leases: the demand the registry reconciles against. This is
    /// the collapsed form of the relay's `topic.business_subscribers()`.
    consumers: AtomicU64,
    /// Whether the tap is wired: a resolved node inside a live session.
    /// False is the collapsed form of the app topic's closed/NoPublisher
    /// status pair; it only ever falls on owner retirement, which is when
    /// the app topics actually closed too.
    publisher: AtomicBool,
    /// Registered leases. Dead entries are dropped by the next
    /// reconciliation, so this never grows with hide/show flapping.
    leases: Mutex<Vec<Weak<LeaseShared>>>,
}

impl Default for LevelSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelSource {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SourceShared {
                consumers: AtomicU64::new(0),
                publisher: AtomicBool::new(false),
                leases: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Subscribe one consumer. The lease counts toward
    /// [`Self::business_subscribers`] until dropped, which is the whole
    /// demand chain: one lease → one engine subscription → engine bus
    /// demand → realtime capture.
    pub fn subscribe(&self) -> LevelLease {
        self.shared.consumers.fetch_add(1, Ordering::AcqRel);
        let shared = Arc::new(LeaseShared {
            epoch: AtomicU64::new(0),
            slot: Mutex::new(None),
        });
        self.shared
            .leases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(Arc::downgrade(&shared));
        LevelLease {
            shared,
            source: Arc::clone(&self.shared),
        }
    }

    /// How many leases read this tap: the demand the realtime meters see.
    pub fn business_subscribers(&self) -> usize {
        self.shared.consumers.load(Ordering::Acquire) as usize
    }

    /// Whether the tap is wired at all. Unwired (stopped session, owner
    /// retired) is the meter's NoPublisher state; wired-but-silent is
    /// Waiting, and the lease's own reads tell the two apart.
    pub fn publisher_present(&self) -> bool {
        self.shared.publisher.load(Ordering::Acquire)
    }

    /// The relay slot's subscription rule, moved over verbatim: a lease
    /// that holds no subscription, or one bound to another node or a
    /// replaced owner generation, gets a fresh engine subscription. The
    /// install bumps the lease's epoch — the consumer's signal that the
    /// engine topic changed and its sequence numbering restarted with it.
    fn reconcile(&self, node: NodeId, owner_generation: u64, observer: &EngineObserver) {
        self.shared.publisher.store(true, Ordering::Release);
        let mut leases = self
            .shared
            .leases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        leases.retain(|lease| {
            let Some(shared) = lease.upgrade() else {
                return false;
            };
            let mut slot = shared
                .slot
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if slot
                .as_ref()
                .is_none_or(|(id, s)| *id != node || s.owner_generation != owner_generation)
            {
                *slot = observer.subscribe_level(node).ok().map(|s| (node, s));
                shared.epoch.fetch_add(1, Ordering::AcqRel);
            }
            true
        });
    }

    /// The relay's release rule: demand gone or node unresolved, so the
    /// engine subscriptions drop. The publisher flag keeps the registry's
    /// last word — the relay's app topic also stayed open here while it
    /// merely stopped forwarding windows.
    fn release(&self) {
        let mut leases = self
            .shared
            .leases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        leases.retain(|lease| {
            let Some(shared) = lease.upgrade() else {
                return false;
            };
            let mut slot = shared
                .slot
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if slot.take().is_some() {
                shared.epoch.fetch_add(1, Ordering::AcqRel);
            }
            true
        });
    }

    /// Owner retirement or replacement: the tap unwinds completely.
    fn retire(&self) {
        self.shared.publisher.store(false, Ordering::Release);
        self.release();
    }

    /// How many live leases currently hold an engine subscription: the
    /// demand the realtime window tap actually sees.
    #[cfg(test)]
    pub(crate) fn held_subscriptions(&self) -> usize {
        self.shared
            .leases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter_map(|lease| lease.upgrade())
            .filter(|lease| {
                lease
                    .slot
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .is_some()
            })
            .count()
    }
}

impl PartialEq for LevelSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl std::fmt::Debug for LevelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelSource")
            .field("consumers", &self.business_subscribers())
            .field("publisher", &self.publisher_present())
            .finish_non_exhaustive()
    }
}

/// One consumer's slice of a [`LevelSource`]: its own engine subscription
/// (a subscriber is a single-reader cursor, so leases never share one) and
/// the epoch the registry bumps whenever it renews that subscription.
/// Dropping the lease releases the demand: the engine subscription dies
/// here, so the engine bus idles and the realtime meter stops capturing.
pub struct LevelLease {
    shared: Arc<LeaseShared>,
    source: Arc<SourceShared>,
}

struct LeaseShared {
    epoch: AtomicU64,
    /// The relay slot's subscription field as it was: the engine
    /// subscription this lease currently reads, with the node and owner
    /// generation it was prepared for.
    slot: Mutex<Option<(NodeId, ManagedLevelSubscription)>>,
}

impl Drop for LevelLease {
    fn drop(&mut self) {
        self.source.consumers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl LevelLease {
    /// The newest engine window, read in place under the slot lock. The
    /// epoch identifies the installed subscription: it changes exactly
    /// when the registry renewed the lease, and a renewed engine topic
    /// restarts sequence numbering, so a consumer resets its fences when
    /// the epoch moves.
    pub fn read_latest<R>(
        &self,
        reader: impl FnOnce(u64, &moiren_signal::BlockReadGuard<AudioWindow, ChannelLevel>) -> R,
    ) -> Option<R> {
        let mut slot = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_, subscription) = slot.as_mut()?;
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        subscription
            .subscriber
            .read_latest()
            .map(|window| reader(epoch, &window))
    }
}

/// One registry tap: the relay slot minus everything the relay copied.
/// `node` stays `None` for the fixed capture taps, whose node the active
/// graph resolves every poll.
struct LevelSlot {
    node: Option<NodeId>,
    source: LevelSource,
}

/// The meter subscription registry: the level half of the retired
/// MeterBridge, collapsed to wiring. It owns no copy machinery — no
/// publishers, no bus host, no sequence dedup — only which tap maps to
/// which node of the running project, and whether each demanded tap still
/// holds a current engine subscription.
pub struct LevelRegistry {
    /// The three fixed capture taps, in the relay's index order: input,
    /// output, tone output.
    monitors: [LevelSlot; 3],
    /// One tap per node of the running project, mirrored into
    /// `SignalTopics::node_levels` for the UI.
    nodes: BTreeMap<NodeId, LevelSource>,
}

impl LevelRegistry {
    pub(crate) fn new(topics: &SignalTopics) -> Self {
        Self {
            monitors: [
                LevelSlot {
                    node: None,
                    source: topics.monitor_input.clone(),
                },
                LevelSlot {
                    node: None,
                    source: topics.monitor_output.clone(),
                },
                LevelSlot {
                    node: None,
                    source: topics.tone_output.clone(),
                },
            ],
            nodes: Default::default(),
        }
    }

    /// One registry round. The relay's per-slot loop runs here against the
    /// leases the UI holds; the copy to the app bus is gone, because the
    /// subscription lives where the reader is. Returns the tone-session
    /// fact the relay published (None while no project is active, so the
    /// published fact stays as the last active round left it).
    pub(crate) fn poll(
        &mut self,
        observer: &EngineObserver,
        topics: &mut SignalTopics,
    ) -> Option<bool> {
        let snapshot = observer.read();
        let active = snapshot.active.as_ref()?;
        self.sync_nodes(topics, active);
        let source = primary_node_of(&active.graph, NodeKind::Source);
        let sink = primary_node_of(&active.graph, NodeKind::Sink);
        let tone = source.is_some_and(|node| {
            matches!(
                active.nodes.get(&node.id()),
                Some(host::NodeConfig::Input(moiren_engine::node::InputConfig {
                    source: moiren_engine::node::InputSource::Software { .. },
                    ..
                }))
            )
        });
        let nodes = [
            source.filter(|_| !tone).map(|n| n.id()),
            sink.filter(|_| !tone).map(|n| n.id()),
            sink.filter(|_| tone).map(|n| n.id()),
        ];
        for (index, slot) in self.monitors.iter_mut().enumerate() {
            let node = slot.node.or_else(|| nodes.get(index).copied().flatten());
            let Some(node) = node.filter(|_| slot.source.business_subscribers() != 0) else {
                slot.source.release();
                continue;
            };
            slot.source
                .reconcile(node, snapshot.state.owner_generation, observer);
        }
        for (&node, source) in &self.nodes {
            if source.business_subscribers() == 0 {
                source.release();
                continue;
            }
            source.reconcile(node, snapshot.state.owner_generation, observer);
        }
        Some(tone)
    }

    /// Mirror the running project's node set into stable per-node taps.
    /// The relay's bus registration/prune machine is gone: a tap is plain
    /// data, so a departing node needs no topic-close dance — the map
    /// entry (and with it the UI's resolve) simply goes away.
    fn sync_nodes(&mut self, topics: &mut SignalTopics, project: &host::EngineProject) {
        let graph = &project.graph;
        let synced = self.nodes.len() == graph.nodes().len()
            && self.nodes.keys().all(|id| graph.get_node(*id).is_ok());
        if synced {
            return;
        }
        self.nodes.retain(|id, _| graph.get_node(*id).is_ok());
        topics
            .node_levels
            .retain(|id, _| graph.get_node(*id).is_ok());
        for node in graph.nodes() {
            if self.nodes.contains_key(&node.id()) {
                continue;
            }
            let source = LevelSource::new();
            self.nodes.insert(node.id(), source.clone());
            topics.node_levels.insert(node.id(), source);
        }
    }

    /// Owner retirement or replacement: every engine subscription drops
    /// (the engine demand idles with it) and the per-node taps unwind.
    /// The fixed monitor taps keep their identities for the next session,
    /// exactly as the app-level topics did.
    pub(crate) fn retire(&mut self, topics: &mut SignalTopics) {
        for slot in &self.monitors {
            slot.source.retire();
        }
        for source in self.nodes.values() {
            source.retire();
        }
        self.nodes.clear();
        topics.node_levels.clear();
    }
}

/// One spectrum analyzer's relay: the engine's latest exact source window,
/// transformed here on the service thread and published as magnitudes onto
/// the application topic registered at the same shape. Demand-gated exactly
/// like the level registry's leases, so an analyzer nobody looks at
/// captures nothing.
pub(super) struct SpectrumBridge {
    control: HostControl,
    host: RtHost,
    spectra: Vec<SpectrumSlot>,
}
/// One spectrum analyzer's relay slot.
struct SpectrumSlot {
    node: NodeId,
    shape: SpectrumShape,
    key: BindingKey,
    topic: Topic<SpectrumTopic>,
    publisher: PreparedPublisher<SpectrumTopic>,
    operation: OpSignalBindings,
    subscription: Option<ManagedSpectrumSubscription>,
    sequence: u64,
    analysis: SpectrumAnalysis,
    magnitudes: Vec<f32>,
}
impl SpectrumBridge {
    /// How many analyzers the bridge currently holds an engine subscription
    /// for — the demand the realtime window tap actually sees.
    #[cfg(test)]
    pub(super) fn demanded_spectra(&self) -> usize {
        self.spectra
            .iter()
            .filter(|slot| slot.subscription.is_some())
            .count()
    }
    pub fn new(c: &mut ServiceContext) -> Result<Self, String> {
        let (control, host) = c.signals.create_bridge_host().map_err(|e| e.to_string())?;
        // No activation here: the spectrum relay's binding table only
        // carries analyzer publishers, and the first sync builds and
        // activates it (the level relay needed the eager activation, but
        // levels no longer publish through this host).
        Ok(Self {
            control,
            host,
            spectra: Vec::new(),
        })
    }
    pub fn poll(&mut self, c: &mut ServiceContext, observer: &EngineObserver) {
        let snapshot = observer.read();
        let Some(active) = snapshot.active else {
            return;
        };
        if let Err(error) = self.sync_nodes(c, &active) {
            c.graph.record_error(format!("Node spectra: {error}"));
            return;
        }
        // The publishers' demand snapshots ride the binding table; the
        // refresh is what lets a publication through as soon as a
        // subscriber arrives (the level relay's poll began with this too).
        self.host.begin_block();
        for slot in &mut self.spectra {
            if slot.topic.business_subscribers() == 0 {
                slot.subscription = None;
                continue;
            }
            if slot
                .subscription
                .as_ref()
                .is_none_or(|s| s.owner_generation != snapshot.state.owner_generation)
            {
                slot.subscription = observer.subscribe_spectrum(slot.node).ok();
                slot.sequence = 0;
            }
            let Some(subscription) = &mut slot.subscription else {
                continue;
            };
            let Some(window) = subscription.subscriber.read_latest() else {
                continue;
            };
            if window.meta().sequence == slot.sequence {
                continue;
            }
            slot.sequence = window.meta().sequence;
            // A window of another shape belongs to a plan this slot has not
            // been re-synced for yet; the next poll re-registers the topic.
            if window.elements().len() != slot.shape.window_capacity()
                || slot
                    .analysis
                    .analyze(window.elements(), &mut slot.magnitudes)
                    .is_err()
            {
                continue;
            }
            let mut scope = self.host.scope(&slot.operation);
            if let Ok(mut write) = scope.try_write(&slot.publisher.handle(), slot.magnitudes.len())
            {
                *write.header_mut() = *window.header();
                write.elements_mut().copy_from_slice(&slot.magnitudes);
                let _ = write.commit(slot.magnitudes.len());
            }
        }
    }

    fn sync_nodes(
        &mut self,
        c: &mut ServiceContext,
        project: &host::EngineProject,
    ) -> Result<(), String> {
        let shapes = spectrum_shapes(project);
        let spectra_synced = self.spectra.len() == shapes.len()
            && self
                .spectra
                .iter()
                .all(|slot| shapes.get(&slot.node) == Some(&slot.shape));
        if spectra_synced {
            return Ok(());
        }
        // A topic closes only once nothing leases it, and a slot leases its
        // topic through the publisher, its operation handle and the active
        // table alike. Release the departed slots — spectra whose shape
        // changed — and activate a table without them before the topics are
        // reconciled, so the bus can close theirs.
        let before = self.spectra.len();
        self.spectra
            .retain(|slot| shapes.get(&slot.node) == Some(&slot.shape));
        let departed = before != self.spectra.len();
        if departed {
            self.activate_table()?;
        }
        c.signals.prepare_node_spectra(&shapes)?;
        for (&node, topic) in &c.signals.topics.node_spectra {
            if self.spectra.iter().any(|slot| slot.node == node) {
                continue;
            }
            let Some(&shape) = shapes.get(&node) else {
                continue;
            };
            let analysis = SpectrumAnalysis::new(shape.channels, shape.fft_size)
                .map_err(|error| error.to_string())?;
            let key = self
                .control
                .new_binding()
                .map_err(|error| error.to_string())?;
            let publisher = self
                .control
                .prepare(topic, key)
                .map_err(|error| error.to_string())?;
            let update = self
                .control
                .prepare_update(&[publisher.erase()])
                .map_err(|error| error.to_string())?;
            self.spectra.push(SpectrumSlot {
                node,
                shape,
                key,
                topic: topic.clone(),
                publisher,
                operation: update.op_bindings(key).map_err(|error| error.to_string())?,
                subscription: None,
                sequence: 0,
                analysis,
                magnitudes: vec![0.; shape.capacity()],
            });
        }
        self.activate_table()
    }

    /// The spectrum relay's one active binding table: every publisher goes
    /// into the same update, and the retired table is dropped here, off the
    /// audio path, which is what releases the leases of the publishers it
    /// no longer carries.
    fn activate_table(&mut self) -> Result<(), String> {
        let bindings: Vec<_> = self
            .spectra
            .iter()
            .map(|slot| slot.publisher.erase())
            .collect();
        let update = self
            .control
            .prepare_update(&bindings)
            .map_err(|error| error.to_string())?;
        for slot in &mut self.spectra {
            slot.operation = update
                .op_bindings(slot.key)
                .map_err(|error| error.to_string())?;
        }
        drop(
            self.host
                .activate(update)
                .map_err(|error| error.error.to_string())?,
        );
        Ok(())
    }
}

/// The shape every spectrum analyzer of the running project publishes at,
/// read from the configuration the engine actually prepared rather than
/// from the editable graph, so a candidate edit never sizes a topic ahead
/// of the plan that will feed it.
fn spectrum_shapes(project: &host::EngineProject) -> BTreeMap<NodeId, SpectrumShape> {
    project
        .graph
        .nodes()
        .iter()
        .filter_map(|node| match project.nodes.get(&node.id()) {
            Some(host::NodeConfig::SpectrumAnalyzer(settings)) => Some((
                node.id(),
                SpectrumShape {
                    channels: node.channels(),
                    fft_size: settings.fft_size,
                },
            )),
            _ => None,
        })
        .collect()
}
