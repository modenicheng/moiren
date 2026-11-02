//! Copy bounded latest windows between the lifecycle-owned host directory and
//! the stable application topics. Demand still decides whether RT meters run.
use super::*;
use crate::service::signals::{SpectrumShape, SpectrumTopic};
use moiren_core::graph::NodeKind;
use moiren_engine::host::{ManagedLevelSubscription, ManagedSpectrumSubscription};
use moiren_signal::{BindingKey, HostControl, OpSignalBindings, PreparedPublisher, RtHost, Topic};
struct Slot {
    node: Option<moiren_core::graph::NodeId>,
    key: BindingKey,
    topic: Topic<LevelTopic>,
    publisher: PreparedPublisher<LevelTopic>,
    operation: OpSignalBindings,
    subscription: Option<(moiren_core::graph::NodeId, ManagedLevelSubscription)>,
    sequence: u64,
}
/// One spectrum analyzer's relay: the engine's latest FFT window copied onto
/// the application topic registered at the same shape. Demand-gated exactly
/// like the level slots, so an analyzer nobody looks at computes nothing.
struct SpectrumSlot {
    node: NodeId,
    shape: SpectrumShape,
    key: BindingKey,
    topic: Topic<SpectrumTopic>,
    publisher: PreparedPublisher<SpectrumTopic>,
    operation: OpSignalBindings,
    subscription: Option<ManagedSpectrumSubscription>,
    sequence: u64,
}
pub(super) struct MeterBridge {
    control: HostControl,
    host: RtHost,
    slots: Vec<Slot>,
    spectra: Vec<SpectrumSlot>,
}
impl MeterBridge {
    /// How many analyzers the bridge currently holds an engine subscription
    /// for — the demand the realtime FFT actually sees.
    #[cfg(test)]
    pub(super) fn demanded_spectra(&self) -> usize {
        self.spectra
            .iter()
            .filter(|slot| slot.subscription.is_some())
            .count()
    }
    pub fn new(c: &mut ServiceContext) -> Result<Self, String> {
        let (control, mut host) = c.signals.create_bridge_host().map_err(|e| e.to_string())?;
        let topics = &c.signals.topics;
        let mut slots = Vec::new();
        for topic in [
            &topics.monitor_input,
            &topics.monitor_output,
            &topics.tone_output,
        ] {
            let key = control.new_binding().map_err(|e| e.to_string())?;
            let publisher = control.prepare(topic, key).map_err(|e| e.to_string())?;
            slots.push((topic.clone(), key, publisher));
        }
        let bindings: Vec<_> = slots.iter().map(|(_, _, p)| p.erase()).collect();
        let update = control
            .prepare_update(&bindings)
            .map_err(|e| e.to_string())?;
        let mut prepared = Vec::new();
        for (topic, key, publisher) in slots {
            prepared.push(Slot {
                node: None,
                key,
                topic,
                publisher,
                operation: update.op_bindings(key).map_err(|e| e.to_string())?,
                subscription: None,
                sequence: 0,
            });
        }
        drop(host.activate(update).map_err(|e| e.error.to_string())?);
        Ok(Self {
            control,
            host,
            slots: prepared,
            spectra: Vec::new(),
        })
    }
    pub fn poll(&mut self, c: &mut ServiceContext, observer: &EngineObserver) {
        let snapshot = observer.read();
        let Some(active) = snapshot.active else {
            return;
        };
        if let Err(error) = self.sync_nodes(c, &active) {
            c.graph.record_error(format!("Node meters: {error}"));
            return;
        }
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
        // The resolved wiring is the fact: consumers read `tone_session` from
        // the snapshot instead of re-deriving it from their own input.
        c.tone_session = tone;
        let nodes = [
            source.filter(|_| !tone).map(|n| n.id()),
            sink.filter(|_| !tone).map(|n| n.id()),
            sink.filter(|_| tone).map(|n| n.id()),
        ];
        self.host.begin_block();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let node = slot.node.or_else(|| nodes.get(index).copied().flatten());
            if slot.topic.business_subscribers() == 0 || node.is_none() {
                slot.subscription = None;
                continue;
            }
            let node = node.expect("checked");
            if slot.subscription.as_ref().is_none_or(|(id, s)| {
                *id != node || s.owner_generation != snapshot.state.owner_generation
            }) {
                slot.subscription = observer.subscribe_level(node).ok().map(|s| (node, s));
                slot.sequence = 0;
            }
            let Some((_, subscription)) = &mut slot.subscription else {
                continue;
            };
            let Some(window) = subscription.subscriber.read_latest() else {
                continue;
            };
            if window.meta().sequence == slot.sequence {
                continue;
            }
            slot.sequence = window.meta().sequence;
            let mut scope = self.host.scope(&slot.operation);
            if let Ok(mut write) =
                scope.try_write(&slot.publisher.handle(), window.elements().len())
            {
                *write.header_mut() = *window.header();
                write.elements_mut().copy_from_slice(window.elements());
                let _ = write.commit(window.elements().len());
            }
        }
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
            if window.elements().len() != slot.shape.capacity() {
                continue;
            }
            let mut scope = self.host.scope(&slot.operation);
            if let Ok(mut write) =
                scope.try_write(&slot.publisher.handle(), window.elements().len())
            {
                *write.header_mut() = *window.header();
                write.elements_mut().copy_from_slice(window.elements());
                let _ = write.commit(window.elements().len());
            }
        }
    }

    fn sync_nodes(
        &mut self,
        c: &mut ServiceContext,
        project: &host::EngineProject,
    ) -> Result<(), String> {
        let graph = &project.graph;
        let shapes = spectrum_shapes(project);
        let levels_synced = self.slots.len() == graph.nodes().len() + 3
            && self
                .slots
                .iter()
                .skip(3)
                .all(|slot| slot.node.is_some_and(|id| graph.get_node(id).is_ok()));
        let spectra_synced = self.spectra.len() == shapes.len()
            && self
                .spectra
                .iter()
                .all(|slot| shapes.get(&slot.node) == Some(&slot.shape));
        if levels_synced && spectra_synced {
            return Ok(());
        }
        // A topic closes only once nothing leases it, and a slot leases its
        // topic through the publisher, its operation handle and the active
        // table alike. Release the departed slots — nodes that left, spectra
        // whose shape changed — and activate a table without them before
        // the topics are reconciled, so the bus can close theirs.
        let kept = (self.slots.len(), self.spectra.len());
        self.slots
            .retain(|slot| slot.node.is_none_or(|id| graph.get_node(id).is_ok()));
        self.spectra
            .retain(|slot| shapes.get(&slot.node) == Some(&slot.shape));
        if kept != (self.slots.len(), self.spectra.len()) {
            self.activate_table()?;
        }
        c.signals.prepare_node_levels(graph)?;
        c.signals.prepare_node_spectra(&shapes)?;
        for (&node, topic) in &c.signals.topics.node_levels {
            if self.slots.iter().any(|slot| slot.node == Some(node)) {
                continue;
            }
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
            self.slots.push(Slot {
                node: Some(node),
                key,
                topic: topic.clone(),
                publisher,
                operation: update.op_bindings(key).map_err(|error| error.to_string())?,
                subscription: None,
                sequence: 0,
            });
        }
        for (&node, topic) in &c.signals.topics.node_spectra {
            if self.spectra.iter().any(|slot| slot.node == node) {
                continue;
            }
            let Some(&shape) = shapes.get(&node) else {
                continue;
            };
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
            });
        }
        self.activate_table()
    }

    /// Levels and spectra share the bridge host, so they share its one
    /// active binding table: every publisher goes into the same update, and
    /// the retired table is dropped here, off the audio path, which is what
    /// releases the leases of the publishers it no longer carries.
    fn activate_table(&mut self) -> Result<(), String> {
        let bindings: Vec<_> = self
            .slots
            .iter()
            .map(|slot| slot.publisher.erase())
            .chain(self.spectra.iter().map(|slot| slot.publisher.erase()))
            .collect();
        let update = self
            .control
            .prepare_update(&bindings)
            .map_err(|error| error.to_string())?;
        for slot in &mut self.slots {
            slot.operation = update
                .op_bindings(slot.key)
                .map_err(|error| error.to_string())?;
        }
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
