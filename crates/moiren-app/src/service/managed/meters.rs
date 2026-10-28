//! Copy bounded latest windows between the lifecycle-owned host directory and
//! the stable application topics. Demand still decides whether RT meters run.
use super::*;
use moiren_core::graph::NodeKind;
use moiren_engine::host::ManagedLevelSubscription;
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
pub(super) struct MeterBridge {
    control: HostControl,
    host: RtHost,
    slots: Vec<Slot>,
}
impl MeterBridge {
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
        })
    }
    pub fn poll(&mut self, c: &mut ServiceContext, observer: &EngineObserver) {
        let snapshot = observer.read();
        let Some(active) = snapshot.active else {
            return;
        };
        if let Err(error) = self.sync_nodes(c, &active.graph) {
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
    }

    fn sync_nodes(
        &mut self,
        c: &mut ServiceContext,
        graph: &moiren_core::graph::LogicalGraph,
    ) -> Result<(), String> {
        if self.slots.len() == graph.nodes().len() + 3
            && self
                .slots
                .iter()
                .skip(3)
                .all(|slot| slot.node.is_some_and(|id| graph.get_node(id).is_ok()))
        {
            return Ok(());
        }
        c.signals.prepare_node_levels(graph)?;
        self.slots
            .retain(|slot| slot.node.is_none_or(|id| graph.get_node(id).is_ok()));
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
        let bindings: Vec<_> = self
            .slots
            .iter()
            .map(|slot| slot.publisher.erase())
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
        drop(
            self.host
                .activate(update)
                .map_err(|error| error.error.to_string())?,
        );
        Ok(())
    }
}
