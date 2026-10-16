use std::{collections::BTreeMap, sync::Mutex};

use crate::meter::signal::{AudioWindow, ChannelLevel, SignalLevelMeter};
use moiren_core::graph::NodeId;
use moiren_signal::{
    BlockInit, Bus, HostControl, LatestBlock, LatestValue, PreparedPublisher, RtHost, Subscriber,
    Topic,
};
pub type ManagedLevelTopic = LatestBlock<AudioWindow, ChannelLevel>;
pub struct ManagedLevelSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<ManagedLevelTopic>,
}

use super::{BackendError, EngineProject, NodeConfig};

pub struct ManagedSignalSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<LatestValue<f32>>,
}

struct Directory {
    bus: Bus,
    generation: u64,
    topics: BTreeMap<NodeId, Topic<LatestValue<f32>>>,
    levels: BTreeMap<NodeId, Topic<ManagedLevelTopic>>,
}
pub(crate) struct SignalHub {
    directory: Mutex<Directory>,
}
impl SignalHub {
    pub(crate) fn new(config: moiren_signal::BusConfig) -> Result<Self, String> {
        Ok(Self {
            directory: Mutex::new(Directory {
                bus: Bus::new(config).map_err(|error| error.to_string())?,
                generation: 0,
                topics: BTreeMap::new(),
                levels: BTreeMap::new(),
            }),
        })
    }
    pub(crate) fn create_host(
        &self,
        generation: u64,
        capacity: usize,
    ) -> Result<(HostControl, RtHost), BackendError> {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let topics = std::mem::take(&mut directory.topics);
        for topic in topics.into_values() {
            directory
                .bus
                .unregister(&topic)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
        }
        directory.generation = generation;
        for topic in std::mem::take(&mut directory.levels).into_values() {
            directory
                .bus
                .unregister(&topic)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
        }
        directory
            .bus
            .create_host(capacity)
            .map_err(|error| BackendError::Failed(error.to_string()))
    }
    pub(crate) fn prepare_levels<S: crate::sample::ProcessingSample>(
        &self,
        project: &EngineProject,
        host: &HostControl,
        bindings: &mut crate::compiler::NodeBindings<S>,
    ) -> Result<(), BackendError> {
        let mut directory = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        for node in project.graph.nodes() {
            let port = match node.kind() {
                moiren_core::graph::NodeKind::Sink => node.inputs().first(),
                _ => node.outputs().first(),
            }
            .expect("validated IO port");
            let topic = if let Some(topic) = directory.levels.get(&node.id()) {
                topic.clone()
            } else {
                let name = format!("owner/{}/node/{:?}/level", directory.generation, node.id());
                let topic = directory
                    .bus
                    .register_latest_block(
                        &name,
                        BlockInit {
                            header: AudioWindow {
                                timeline_epoch: 0,
                                frame_start: 0,
                                frame_end: 0,
                                sample_rate: 0.0,
                            },
                            element: ChannelLevel {
                                peak: 0.0,
                                rms: 0.0,
                            },
                            capacity: node.channels(),
                        },
                    )
                    .map_err(|e| BackendError::Failed(e.to_string()))?;
                directory.levels.insert(node.id(), topic.clone());
                topic
            };
            let publisher = host
                .prepare(
                    &topic,
                    host.new_binding()
                        .map_err(|e| BackendError::Failed(e.to_string()))?,
                )
                .map_err(|e| BackendError::Failed(e.to_string()))?;
            let meter = SignalLevelMeter::new(node.channels(), 1536, publisher)
                .map_err(|e| BackendError::Failed(e.to_string()))?;
            match node.kind() {
                moiren_core::graph::NodeKind::Sink => bindings.observe_input(port.id(), meter),
                _ => bindings.observe_output(port.id(), meter),
            }
            .map_err(|e| BackendError::Failed(e.to_string()))?;
        }
        Ok(())
    }
    pub(crate) fn subscribe_level(&self, node: NodeId) -> Result<ManagedLevelSubscription, String> {
        let mut directory = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        let topic = directory
            .levels
            .get(&node)
            .cloned()
            .ok_or("node level signal is not prepared")?;
        Ok(ManagedLevelSubscription {
            owner_generation: directory.generation,
            subscriber: directory
                .bus
                .subscribe(&topic, Default::default())
                .map_err(|e| e.to_string())?,
        })
    }
    pub(crate) fn prepare(
        &self,
        node: NodeId,
        host: &HostControl,
    ) -> Result<PreparedPublisher<LatestValue<f32>>, BackendError> {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let topic = if let Some(topic) = directory.topics.get(&node) {
            topic.clone()
        } else {
            let name = format!(
                "owner/{}/node/{node:?}/compressor/reduction",
                directory.generation
            );
            let topic = directory
                .bus
                .register_latest_value(&name, 0.0_f32)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
            directory.topics.insert(node, topic.clone());
            topic
        };
        host.prepare(
            &topic,
            host.new_binding()
                .map_err(|error| BackendError::Failed(error.to_string()))?,
        )
        .map_err(|error| BackendError::Failed(error.to_string()))
    }
    pub(crate) fn subscribe(&self, node: NodeId) -> Result<ManagedSignalSubscription, String> {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let topic = directory
            .topics
            .get(&node)
            .cloned()
            .ok_or_else(|| "compressor signal is not prepared".to_string())?;
        Ok(ManagedSignalSubscription {
            owner_generation: directory.generation,
            subscriber: directory
                .bus
                .subscribe(&topic, Default::default())
                .map_err(|error| error.to_string())?,
        })
    }
    pub(crate) fn pump(&self) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        directory.bus.pump(moiren_signal::PumpBudget {
            topic_checks: 1024,
            deliveries: 256,
            copy_bytes: 1024 * 1024,
        });
    }
    pub(crate) fn prune(&self, project: &EngineProject) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let removed: Vec<_> = directory
            .topics
            .iter()
            .filter(|(node, _)| !matches!(project.nodes.get(node), Some(NodeConfig::Compressor(_))))
            .map(|(&node, topic)| (node, topic.clone()))
            .collect();
        for (node, topic) in removed {
            if directory.bus.unregister(&topic).is_ok() {
                directory.topics.remove(&node);
            }
        }
        let removed: Vec<_> = directory
            .levels
            .iter()
            .filter(|(node, _)| !project.nodes.contains_key(node))
            .map(|(&node, topic)| (node, topic.clone()))
            .collect();
        for (node, topic) in removed {
            if directory.bus.unregister(&topic).is_ok() {
                directory.levels.remove(&node);
            }
        }
    }
}
