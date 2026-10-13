use std::{collections::BTreeMap, sync::Mutex};

use moiren_core::graph::NodeId;
use moiren_signal::{Bus, HostControl, LatestValue, PreparedPublisher, RtHost, Subscriber, Topic};

use super::{BackendError, EngineProject, NodeConfig};

pub struct ManagedSignalSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<LatestValue<f32>>,
}

struct Directory {
    bus: Bus,
    generation: u64,
    topics: BTreeMap<NodeId, Topic<LatestValue<f32>>>,
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
        directory
            .bus
            .create_host(capacity)
            .map_err(|error| BackendError::Failed(error.to_string()))
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
            topic_checks: 128,
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
    }
}
