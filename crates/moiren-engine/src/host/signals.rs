use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{
    meter::{
        LevelSnapshot, MeterReader,
        signal::{AudioWindow, ChannelLevel, SignalLevelMeter},
    },
    recorder::{RecorderReader, RecorderStats},
    spectrum::{SpectrumSettings, SpectrumWindow},
};
use moiren_core::graph::{NodeId, PortId};
use moiren_signal::{
    BlockInit, Bus, HostControl, LatestBlock, LatestValue, PreparedPublisher, RtHost, Subscriber,
    Topic,
};

pub type ManagedLevelTopic = LatestBlock<AudioWindow, ChannelLevel>;
pub type ManagedSpectrumTopic = LatestBlock<SpectrumWindow, f32>;

pub struct ManagedLevelSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<ManagedLevelTopic>,
}

use super::{BackendError, EngineProject, NodeConfig};

pub struct ManagedSignalSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<LatestValue<f32>>,
}

pub struct ManagedSpectrumSubscription {
    pub owner_generation: u64,
    pub subscriber: Subscriber<ManagedSpectrumTopic>,
}

type ReaderSlot<T> = Arc<Mutex<Option<(u64, T)>>>;

/// Control-side handle to a meter reader. It becomes stale when its owner is
/// replaced, and never exposes the reader to the realtime thread.
pub struct ManagedMeterReader {
    owner_generation: u64,
    slot: ReaderSlot<MeterReader>,
}

impl ManagedMeterReader {
    pub fn owner_generation(&self) -> u64 {
        self.owner_generation
    }

    pub fn latest(&mut self) -> Option<LevelSnapshot> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (generation, reader) = slot.as_mut()?;
        (*generation == self.owner_generation)
            .then(|| reader.latest())
            .flatten()
    }
}

/// Control-side handle to a recorder endpoint, tied to the owner generation
/// that prepared its SPSC queue.
pub struct ManagedRecorderReader {
    owner_generation: u64,
    slot: ReaderSlot<RecorderReader>,
}

impl ManagedRecorderReader {
    pub fn owner_generation(&self) -> u64 {
        self.owner_generation
    }

    pub fn channel_count(&self) -> Option<usize> {
        self.with_reader(|reader| reader.channel_count())
    }

    pub fn capacity_frames(&self) -> Option<usize> {
        self.with_reader(|reader| reader.capacity_frames())
    }

    pub fn start(&self) -> bool {
        self.with_reader(|reader| reader.start()).is_some()
    }

    pub fn stop(&self) -> bool {
        self.with_reader(|reader| reader.stop()).is_some()
    }

    pub fn is_recording(&self) -> Option<bool> {
        self.with_reader(|reader| reader.is_recording())
    }

    pub fn stats(&self) -> Option<RecorderStats> {
        self.with_reader(|reader| reader.stats())
    }

    pub fn queued_frames(&self) -> Option<usize> {
        self.with_reader(|reader| reader.queued_frames())
    }

    pub fn read_interleaved(&mut self, samples: &mut [f32]) -> Option<usize> {
        self.with_reader(|reader| reader.read_interleaved(samples))
    }

    fn with_reader<T>(&self, f: impl FnOnce(&mut RecorderReader) -> T) -> Option<T> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (generation, reader) = slot.as_mut()?;
        (*generation == self.owner_generation).then(|| f(reader))
    }
}

#[derive(Default)]
pub(crate) struct PreparedReaders {
    // None explicitly retains an existing endpoint after validated observer
    // reuse; absence removes it. Never replace a retained recording queue.
    pub meters: BTreeMap<NodeId, Option<MeterReader>>,
    pub recorders: BTreeMap<NodeId, Option<RecorderReader>>,
}

struct Directory {
    bus: Bus,
    generation: u64,
    topics: BTreeMap<NodeId, Topic<LatestValue<f32>>>,
    // Prepared/retired shapes coexist with the active shape until all plan
    // leases retire. Preparation never changes the public subscription target.
    levels: BTreeMap<(NodeId, PortId, usize), Topic<ManagedLevelTopic>>,
    active_levels: BTreeMap<NodeId, Topic<ManagedLevelTopic>>,
    spectra: BTreeMap<(NodeId, usize, usize), Topic<ManagedSpectrumTopic>>,
    active_spectra: BTreeMap<NodeId, Topic<ManagedSpectrumTopic>>,
    meters: BTreeMap<NodeId, ReaderSlot<MeterReader>>,
    recorders: BTreeMap<NodeId, ReaderSlot<RecorderReader>>,
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
                active_levels: BTreeMap::new(),
                spectra: BTreeMap::new(),
                active_spectra: BTreeMap::new(),
                meters: BTreeMap::new(),
                recorders: BTreeMap::new(),
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
        directory.active_levels.clear();
        directory.active_spectra.clear();
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
        for topic in std::mem::take(&mut directory.spectra).into_values() {
            directory
                .bus
                .unregister(&topic)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
        }
        clear_slots(&mut directory.meters);
        clear_slots(&mut directory.recorders);
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
            let shape = (node.id(), port.id(), port.channels());
            let topic = if let Some(topic) = directory.levels.get(&shape) {
                topic.clone()
            } else {
                let name = format!(
                    "owner/{}/node/{:?}/level/{:?}/{}",
                    directory.generation,
                    node.id(),
                    port.id(),
                    port.channels()
                );
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
                            capacity: port.channels(),
                        },
                    )
                    .map_err(|e| BackendError::Failed(e.to_string()))?;
                directory.levels.insert(shape, topic.clone());
                topic
            };
            let publisher = host
                .prepare(
                    &topic,
                    host.new_binding()
                        .map_err(|e| BackendError::Failed(e.to_string()))?,
                )
                .map_err(|e| BackendError::Failed(e.to_string()))?;
            let window_frames = match project.nodes.get(&node.id()) {
                Some(NodeConfig::Meter(settings)) => settings.window_frames,
                _ => 1536,
            };
            let meter = SignalLevelMeter::new(port.channels(), window_frames, publisher)
                .map_err(|e| BackendError::Failed(e.to_string()))?;
            match node.kind() {
                moiren_core::graph::NodeKind::Sink => bindings.observe_input(port.id(), meter),
                _ => bindings.observe_output(port.id(), meter),
            }
            .map_err(|e| BackendError::Failed(e.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn prepare_spectrum(
        &self,
        node: NodeId,
        channels: usize,
        settings: SpectrumSettings,
        host: &HostControl,
    ) -> Result<PreparedPublisher<ManagedSpectrumTopic>, BackendError> {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let capacity = channels
            .checked_mul(settings.bins_per_channel())
            .ok_or_else(|| BackendError::Failed("spectrum topic capacity overflow".into()))?;
        let shape = (node, channels, settings.fft_size);
        let topic = if let Some(topic) = directory.spectra.get(&shape) {
            topic.clone()
        } else {
            let name = format!(
                "owner/{}/node/{node:?}/spectrum/{channels}/{}",
                directory.generation, settings.fft_size
            );
            let topic = directory
                .bus
                .register_latest_block(
                    &name,
                    BlockInit {
                        header: SpectrumWindow {
                            timeline_epoch: 0,
                            frame_start: 0,
                            frame_end: 0,
                            sample_rate: 0.0,
                            fft_size: settings.fft_size,
                            channels,
                        },
                        element: 0.0_f32,
                        capacity,
                    },
                )
                .map_err(|error| BackendError::Failed(error.to_string()))?;
            directory.spectra.insert(shape, topic.clone());
            topic
        };
        host.prepare(
            &topic,
            host.new_binding()
                .map_err(|error| BackendError::Failed(error.to_string()))?,
        )
        .map_err(|error| BackendError::Failed(error.to_string()))
    }

    pub(crate) fn subscribe_spectrum(
        &self,
        node: NodeId,
    ) -> Result<ManagedSpectrumSubscription, String> {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let topic = directory
            .active_spectra
            .get(&node)
            .cloned()
            .ok_or_else(|| "spectrum signal is not prepared".to_string())?;
        Ok(ManagedSpectrumSubscription {
            owner_generation: directory.generation,
            subscriber: directory
                .bus
                .subscribe(&topic, Default::default())
                .map_err(|error| error.to_string())?,
        })
    }

    pub(crate) fn install_readers(&self, generation: u64, mut readers: PreparedReaders) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        install_slots(&mut directory.meters, generation, &mut readers.meters);
        install_slots(&mut directory.recorders, generation, &mut readers.recorders);
    }

    pub(crate) fn activate(&self, generation: u64, project: &EngineProject) {
        let mut directory = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        if directory.generation != generation {
            return;
        }
        let mut levels = BTreeMap::new();
        let mut spectra = BTreeMap::new();
        for node in project.graph.nodes() {
            let port = match node.kind() {
                moiren_core::graph::NodeKind::Sink => node.inputs().first(),
                _ => node.outputs().first(),
            }
            .expect("validated IO port");
            let topic = &directory.levels[&(node.id(), port.id(), port.channels())];
            levels.insert(node.id(), topic.clone());
            if let Some(NodeConfig::SpectrumAnalyzer(settings)) = project.nodes.get(&node.id()) {
                let topic = &directory.spectra[&(node.id(), node.channels(), settings.fft_size)];
                spectra.insert(node.id(), topic.clone());
            }
        }
        directory.active_levels = levels;
        directory.active_spectra = spectra;
    }

    pub(crate) fn clear_readers(&self, generation: u64) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_generation(&mut directory.meters, generation);
        clear_generation(&mut directory.recorders, generation);
        if directory.generation == generation {
            directory.active_levels.clear();
            directory.active_spectra.clear();
        }
    }

    pub(crate) fn meter_reader(&self, node: NodeId) -> Result<ManagedMeterReader, String> {
        let directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let slot = directory
            .meters
            .get(&node)
            .cloned()
            .ok_or_else(|| "meter reader is not prepared".to_string())?;
        let owner_generation = slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .map(|(generation, _)| *generation)
            .ok_or_else(|| "meter reader is not active".to_string())?;
        Ok(ManagedMeterReader {
            owner_generation,
            slot,
        })
    }

    pub(crate) fn recorder_reader(&self, node: NodeId) -> Result<ManagedRecorderReader, String> {
        let directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let slot = directory
            .recorders
            .get(&node)
            .cloned()
            .ok_or_else(|| "recorder reader is not prepared".to_string())?;
        let owner_generation = slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .map(|(generation, _)| *generation)
            .ok_or_else(|| "recorder reader is not active".to_string())?;
        Ok(ManagedRecorderReader {
            owner_generation,
            slot,
        })
    }
    pub(crate) fn subscribe_level(&self, node: NodeId) -> Result<ManagedLevelSubscription, String> {
        let mut directory = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        let topic = directory
            .active_levels
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
    pub(crate) fn prune(&self, project: Option<&EngineProject>) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let removed: Vec<_> = directory
            .topics
            .iter()
            .filter(|(node, _)| {
                !matches!(
                    project.and_then(|p| p.nodes.get(node)),
                    Some(NodeConfig::Compressor(_))
                )
            })
            .map(|(&node, topic)| (node, topic.clone()))
            .collect();
        for (node, topic) in removed {
            if directory.bus.unregister(&topic).is_ok() {
                directory.topics.remove(&node);
            }
        }
        let removed: Vec<_> = directory
            .spectra
            .iter()
            .filter(|((node, _, _), topic)| directory.active_spectra.get(node) != Some(*topic))
            .map(|(&shape, topic)| (shape, topic.clone()))
            .collect();
        for (shape, topic) in removed {
            if directory.bus.unregister(&topic).is_ok() {
                directory.spectra.remove(&shape);
            }
        }
        let removed: Vec<_> = directory
            .levels
            .iter()
            .filter(|((node, _, _), topic)| directory.active_levels.get(node) != Some(*topic))
            .map(|(&shape, topic)| (shape, topic.clone()))
            .collect();
        for (shape, topic) in removed {
            if directory.bus.unregister(&topic).is_ok() {
                directory.levels.remove(&shape);
            }
        }
        if let Some(project) = project {
            prune_slots(&mut directory.meters, &project.nodes, |config| {
                matches!(config, NodeConfig::Meter(_))
            });
            prune_slots(&mut directory.recorders, &project.nodes, |config| {
                matches!(config, NodeConfig::Recorder(_))
            });
        }
    }
}

fn install_slots<T>(
    slots: &mut BTreeMap<NodeId, ReaderSlot<T>>,
    generation: u64,
    readers: &mut BTreeMap<NodeId, Option<T>>,
) {
    let active_nodes: std::collections::BTreeSet<_> = readers.keys().copied().collect();
    for (node, reader) in std::mem::take(readers) {
        let Some(reader) = reader else {
            debug_assert!(slots.get(&node).is_some_and(|slot| {
                slot.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == generation)
            }));
            continue;
        };
        let slot = slots
            .entry(node)
            .or_insert_with(|| Arc::new(Mutex::new(None)));
        *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = Some((generation, reader));
    }
    slots.retain(|node, slot| {
        if active_nodes.contains(node) {
            return true;
        }
        *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        false
    });
}

fn clear_slots<T>(slots: &mut BTreeMap<NodeId, ReaderSlot<T>>) {
    for slot in slots.values() {
        *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    }
    slots.clear();
}

fn clear_generation<T>(slots: &mut BTreeMap<NodeId, ReaderSlot<T>>, generation: u64) {
    slots.retain(|_, slot| {
        let mut value = slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if value
            .as_ref()
            .is_some_and(|(owner, _)| *owner == generation)
        {
            *value = None;
        }
        value.is_some()
    });
}

fn prune_slots<T>(
    slots: &mut BTreeMap<NodeId, ReaderSlot<T>>,
    nodes: &BTreeMap<NodeId, NodeConfig>,
    keep: impl Fn(&NodeConfig) -> bool,
) {
    slots.retain(|node, slot| {
        if nodes.get(node).is_some_and(&keep) {
            return true;
        }
        *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        false
    });
}
