use std::{
    collections::BTreeMap,
    io::{self, Seek, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    meter::{
        LevelSnapshot, MeterReader,
        signal::{AudioWindow, ChannelLevel, SignalLevelMeter},
    },
    recorder::{
        RecorderReader, RecorderStats, RecordingSession, RecordingSource, WavFormat, WavRecording,
        spawn_recording,
    },
    spectrum::{SpectrumSettings, SpectrumWindow},
};
use moiren_core::graph::{NodeId, PortId};
use moiren_signal::{
    BlockInit, Bus, HostControl, LatestBlock, LatestValue, PreparedPublisher, RtHost, Shape,
    Subscriber, Topic,
};

pub type ManagedLevelTopic = LatestBlock<AudioWindow, ChannelLevel>;
pub type ManagedSpectrumTopic = LatestBlock<SpectrumWindow, f32>;

/// Control-side subscription handle shared by every managed signal flavor:
/// the owning generation plus one bus subscriber. It becomes stale when its
/// owner generation is replaced; staleness against later owner changes is
/// the flavor's per-read concern.
pub struct ManagedSubscription<T: Shape> {
    pub owner_generation: u64,
    pub subscriber: Subscriber<T>,
}

pub type ManagedLevelSubscription = ManagedSubscription<ManagedLevelTopic>;
pub type ManagedSignalSubscription = ManagedSubscription<LatestValue<f32>>;
pub type ManagedSpectrumSubscription = ManagedSubscription<ManagedSpectrumTopic>;

use super::{BackendError, EngineProject, NodeConfig};

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
    slot: ReaderSlot<RecorderEndpoint>,
}

struct RecorderEndpoint {
    reader: Option<RecorderReader>,
    alive: Arc<AtomicBool>,
}

impl From<RecorderReader> for RecorderEndpoint {
    fn from(reader: RecorderReader) -> Self {
        Self {
            reader: Some(reader),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl Drop for RecorderEndpoint {
    fn drop(&mut self) {
        // A worker owns the consumer while leased. Revocation never waits for
        // file IO; the worker stops capture, drains and finalizes independently.
        self.alive.store(false, Ordering::Release);
    }
}

struct RecorderLease {
    owner_generation: u64,
    slot: ReaderSlot<RecorderEndpoint>,
    alive: Arc<AtomicBool>,
    reader: Option<RecorderReader>,
}

impl RecordingSource for RecorderLease {
    fn reader(&mut self) -> &mut RecorderReader {
        self.reader.as_mut().expect("lease owns the consumer")
    }

    fn is_active(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

impl Drop for RecorderLease {
    fn drop(&mut self) {
        let mut reader = self.reader.take();
        {
            let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((generation, endpoint)) = slot.as_mut()
                && *generation == self.owner_generation
                && Arc::ptr_eq(&endpoint.alive, &self.alive)
                && endpoint.alive.load(Ordering::Acquire)
                && endpoint.reader.is_none()
            {
                endpoint.reader = reader.take();
            }
        }
        // A replaced endpoint must never receive the retired SPSC consumer.
        // Reclaim it outside the slot lock; no lease owns the host or hub.
        drop(reader);
    }
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

    /// Record on a control-side worker with exclusive ownership of the queue.
    /// `sample_rate` is the graph's processing sample rate, not the hardware
    /// mix rate. Buffered complete frames are included. While leased, raw
    /// reader queries return None and start/stop return false.
    ///
    /// A busy/stale claim fails before opening or truncating the destination.
    /// WAV initialization errors return the reader without changing capture;
    /// after worker startup, completion or failure returns it stopped.
    pub fn record_wav(
        &self,
        path: impl AsRef<Path>,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        let mut source = self.claim()?;
        let wav = WavRecording::create(path, source.reader().channel_count(), sample_rate)?;
        spawn_recording(source, wav)
    }

    /// Like `record_wav`, with an empty caller-owned seekable destination.
    /// `sample_rate` must be the graph's processing rate, not a device mix rate.
    /// All writer IO runs without holding the hub or reader slot lock.
    pub fn record_with_writer<W: Write + Seek + Send + 'static>(
        &self,
        writer: W,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        self.record_with_writer_format(writer, sample_rate, WavFormat::Float32)
    }

    /// Select file encoding without changing the leased float32 transport.
    /// Claim validation and all file IO retain the same off-RT ownership rules.
    pub fn record_with_writer_format<W: Write + Seek + Send + 'static>(
        &self,
        writer: W,
        sample_rate: u32,
        format: WavFormat,
    ) -> io::Result<RecordingSession> {
        let mut source = self.claim()?;
        let wav = WavRecording::from_writer_with_format(
            writer,
            source.reader().channel_count(),
            sample_rate,
            format,
        )?;
        spawn_recording(source, wav)
    }

    fn claim(&self) -> io::Result<RecorderLease> {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        let (generation, endpoint) = slot
            .as_mut()
            .filter(|(generation, _)| *generation == self.owner_generation)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "recorder is not active"))?;
        let reader = endpoint.reader.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "recorder consumer is already leased",
            )
        })?;
        // Keep Some(generation, endpoint) installed: validated observer reuse
        // retains that marker even while the consumer is on the WAV worker.
        Ok(RecorderLease {
            owner_generation: *generation,
            slot: Arc::clone(&self.slot),
            alive: Arc::clone(&endpoint.alive),
            reader: Some(reader),
        })
    }

    fn with_reader<T>(&self, f: impl FnOnce(&mut RecorderReader) -> T) -> Option<T> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (generation, endpoint) = slot.as_mut()?;
        if *generation != self.owner_generation {
            return None;
        }
        endpoint.reader.as_mut().map(f)
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
    recorders: BTreeMap<NodeId, ReaderSlot<RecorderEndpoint>>,
}
pub(crate) struct SignalHub {
    directory: Mutex<Directory>,
    pump_topic_checks: usize,
}
impl SignalHub {
    pub(crate) fn new(config: moiren_signal::BusConfig) -> Result<Self, String> {
        Ok(Self {
            // The bus visits directory slots, including empty ones, and needs
            // separate visits to snapshot Latest and deliver each recipient.
            // A fixed small budget makes sparse, large-capacity hosts stale.
            pump_topic_checks: config.max_topics.saturating_mul(4),
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
        let mut guard = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = &mut *guard;
        directory.active_levels.clear();
        directory.active_spectra.clear();
        unregister_topics(&mut directory.bus, std::mem::take(&mut directory.topics))?;
        directory.generation = generation;
        unregister_topics(&mut directory.bus, std::mem::take(&mut directory.levels))?;
        unregister_topics(&mut directory.bus, std::mem::take(&mut directory.spectra))?;
        clear_slots(&mut directory.meters);
        clear_slots(&mut directory.recorders);
        directory.bus.create_host(capacity).map_err(failed)
    }
    pub(crate) fn prepare_levels<S: crate::sample::ProcessingSample>(
        &self,
        project: &EngineProject,
        host: &HostControl,
        bindings: &mut crate::compiler::NodeBindings<S>,
    ) -> Result<(), BackendError> {
        let mut guard = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        let directory = &mut *guard;
        for node in project.graph.nodes() {
            let port = match node.kind() {
                moiren_core::graph::NodeKind::Sink => node.inputs().first(),
                _ => node.outputs().first(),
            }
            .expect("validated IO port");
            let shape = (node.id(), port.id(), port.channels());
            let generation = directory.generation;
            let topic =
                topic_or_register(&mut directory.bus, &mut directory.levels, shape, |bus| {
                    bus.register_latest_block(
                        &format!(
                            "owner/{}/node/{:?}/level/{:?}/{}",
                            generation,
                            node.id(),
                            port.id(),
                            port.channels()
                        ),
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
                })?;
            let publisher = host
                .prepare(&topic, host.new_binding().map_err(failed)?)
                .map_err(failed)?;
            let window_frames = match project.nodes.get(&node.id()) {
                Some(NodeConfig::Meter(settings)) => settings.window_frames,
                _ => crate::meter::ui_window_frames(project.processing.engine.processing_sr),
            };
            let meter =
                SignalLevelMeter::new(port.channels(), window_frames, publisher).map_err(failed)?;
            match node.kind() {
                moiren_core::graph::NodeKind::Sink => bindings.observe_input(port.id(), meter),
                _ => bindings.observe_output(port.id(), meter),
            }
            .map_err(failed)?;
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
        let mut guard = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = &mut *guard;
        let capacity = channels
            .checked_mul(settings.bins_per_channel())
            .ok_or_else(|| failed("spectrum topic capacity overflow"))?;
        let shape = (node, channels, settings.fft_size);
        let generation = directory.generation;
        let topic = topic_or_register(&mut directory.bus, &mut directory.spectra, shape, |bus| {
            bus.register_latest_block(
                &format!(
                    "owner/{}/node/{node:?}/spectrum/{channels}/{}",
                    generation, settings.fft_size
                ),
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
        })?;
        host.prepare(&topic, host.new_binding().map_err(failed)?)
            .map_err(failed)
    }

    pub(crate) fn subscribe_spectrum(
        &self,
        node: NodeId,
    ) -> Result<ManagedSpectrumSubscription, String> {
        let mut guard = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = &mut *guard;
        let (owner_generation, subscriber) = subscribe_active(
            &mut directory.bus,
            directory.generation,
            directory.active_spectra.get(&node),
            "spectrum signal is not prepared",
        )?;
        Ok(ManagedSpectrumSubscription {
            owner_generation,
            subscriber,
        })
    }
    pub(crate) fn install_readers(&self, generation: u64, mut readers: PreparedReaders) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        install_slots(&mut directory.meters, generation, &mut readers.meters);
        let mut recorders = readers
            .recorders
            .into_iter()
            .map(|(node, reader)| (node, reader.map(RecorderEndpoint::from)))
            .collect();
        install_slots(&mut directory.recorders, generation, &mut recorders);
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
        let (owner_generation, slot) = reader_handle(
            &directory.meters,
            node,
            "meter reader is not prepared",
            "meter reader is not active",
        )?;
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
        let (owner_generation, slot) = reader_handle(
            &directory.recorders,
            node,
            "recorder reader is not prepared",
            "recorder reader is not active",
        )?;
        Ok(ManagedRecorderReader {
            owner_generation,
            slot,
        })
    }
    pub(crate) fn subscribe_level(&self, node: NodeId) -> Result<ManagedLevelSubscription, String> {
        let mut guard = self.directory.lock().unwrap_or_else(|p| p.into_inner());
        let directory = &mut *guard;
        let (owner_generation, subscriber) = subscribe_active(
            &mut directory.bus,
            directory.generation,
            directory.active_levels.get(&node),
            "node level signal is not prepared",
        )?;
        Ok(ManagedLevelSubscription {
            owner_generation,
            subscriber,
        })
    }
    pub(crate) fn prepare(
        &self,
        node: NodeId,
        host: &HostControl,
    ) -> Result<PreparedPublisher<LatestValue<f32>>, BackendError> {
        let mut guard = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = &mut *guard;
        let generation = directory.generation;
        let topic = topic_or_register(&mut directory.bus, &mut directory.topics, node, |bus| {
            bus.register_latest_value(
                &format!("owner/{generation}/node/{node:?}/compressor/reduction"),
                0.0_f32,
            )
        })?;
        host.prepare(&topic, host.new_binding().map_err(failed)?)
            .map_err(failed)
    }
    pub(crate) fn subscribe(&self, node: NodeId) -> Result<ManagedSignalSubscription, String> {
        let mut guard = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = &mut *guard;
        let (owner_generation, subscriber) = subscribe_active(
            &mut directory.bus,
            directory.generation,
            directory.topics.get(&node),
            "compressor signal is not prepared",
        )?;
        Ok(ManagedSignalSubscription {
            owner_generation,
            subscriber,
        })
    }

    pub(crate) fn pump(&self) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        directory.bus.pump(moiren_signal::PumpBudget {
            topic_checks: self.pump_topic_checks,
            deliveries: 256,
            copy_bytes: 1024 * 1024,
        });
    }
    pub(crate) fn poll_interval(&self, idle: std::time::Duration) -> std::time::Duration {
        if self.has_demand() {
            // Both the engine and compatibility bridge can queue a meter
            // window. One millisecond per stage leaves room for analysis
            // windows and scheduler jitter within a 120 Hz presentation frame.
            idle.min(std::time::Duration::from_millis(1))
        } else {
            idle
        }
    }
    pub(crate) fn has_demand(&self) -> bool {
        self.directory
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .bus
            .has_business_demand()
    }
    pub(crate) fn prune(&self, project: Option<&EngineProject>) {
        let mut directory = self
            .directory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // Split the directory borrow: each family prunes against its own
        // active-topic map while the bus mutates independently.
        let Directory {
            bus,
            topics,
            levels,
            active_levels,
            spectra,
            active_spectra,
            meters,
            recorders,
            ..
        } = &mut *directory;
        unregister_stale(bus, topics, |node, _| {
            !matches!(
                project.and_then(|p| p.nodes.get(node)),
                Some(NodeConfig::Compressor(_))
            )
        });
        unregister_stale(bus, spectra, |(node, ..), topic| {
            active_spectra.get(node) != Some(topic)
        });
        unregister_stale(bus, levels, |(node, ..), topic| {
            active_levels.get(node) != Some(topic)
        });
        if let Some(project) = project {
            prune_slots(meters, &project.nodes, |config| {
                matches!(config, NodeConfig::Meter(_))
            });
            prune_slots(recorders, &project.nodes, |config| {
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

/// Shared prune step of every topic family: collect the entries `stale`
/// reports, unregister them from the bus, and drop only the ones that
/// unregistered cleanly (a busy lease keeps its entry for a later prune).
fn unregister_stale<K: Ord + Copy, T: Shape>(
    bus: &mut Bus,
    cache: &mut BTreeMap<K, Topic<T>>,
    stale: impl Fn(&K, &Topic<T>) -> bool,
) {
    let removed: Vec<_> = cache
        .iter()
        .filter(|(key, topic)| stale(key, topic))
        .map(|(&key, topic)| (key, topic.clone()))
        .collect();
    for (key, topic) in removed {
        if bus.unregister(&topic).is_ok() {
            cache.remove(&key);
        }
    }
}

/// Shared error mapping of every bus/host failure this hub surfaces to the
/// backend: [`BackendError::Failed`] carrying the source message. A file-local
/// helper rather than a `From` impl so unrelated `?` sites keep their
/// inference and match exhaustiveness untouched.
fn failed(error: impl std::fmt::Display) -> BackendError {
    BackendError::Failed(error.to_string())
}

/// Get-or-register shared by every shape-keyed topic cache: a hit reuses the
/// topic an earlier (possibly retired) preparation registered, a miss
/// registers through `build` and caches the result. Caching by shape rather
/// than by active plan lets compatible successors share one topic, and the
/// name formatting runs only on the miss path.
fn topic_or_register<K: Ord + Copy, T: Shape>(
    bus: &mut Bus,
    cache: &mut BTreeMap<K, Topic<T>>,
    key: K,
    build: impl FnOnce(&mut Bus) -> Result<Topic<T>, moiren_signal::RegisterError>,
) -> Result<Topic<T>, BackendError> {
    if let Some(topic) = cache.get(&key) {
        return Ok(topic.clone());
    }
    let topic = build(bus).map_err(failed)?;
    cache.insert(key, topic.clone());
    Ok(topic)
}

/// Shared subscribe step of the three managed subscription flavors: resolve
/// the currently active topic, then bind one subscriber under the current
/// owner generation. Staleness against later owner changes is the caller's
/// per-read concern, not the subscription's.
fn subscribe_active<T: Shape>(
    bus: &mut Bus,
    generation: u64,
    topic: Option<&Topic<T>>,
    unprepared: &str,
) -> Result<(u64, Subscriber<T>), String> {
    let topic = topic.ok_or_else(|| unprepared.to_string())?;
    Ok((
        generation,
        bus.subscribe(topic, Default::default())
            .map_err(|error| error.to_string())?,
    ))
}

/// Shared half of the meter/recorder reader twins: resolve a node's reader
/// slot and the owner generation stamped inside it. A missing slot means the
/// observer was never prepared; an empty one means it is not active.
fn reader_handle<T>(
    slots: &BTreeMap<NodeId, ReaderSlot<T>>,
    node: NodeId,
    unprepared: &str,
    inactive: &str,
) -> Result<(u64, ReaderSlot<T>), String> {
    let slot = slots
        .get(&node)
        .cloned()
        .ok_or_else(|| unprepared.to_string())?;
    let owner_generation = slot
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
        .map(|(generation, _)| *generation)
        .ok_or_else(|| inactive.to_string())?;
    Ok((owner_generation, slot))
}

/// Retire one full topic family when a new owner takes over the bus. The map
/// has been taken up front, so a failed unregister drops its entries on the
/// returning error instead of leaving the directory pointing at dead topics.
fn unregister_topics<K, T: Shape>(
    bus: &mut Bus,
    topics: BTreeMap<K, Topic<T>>,
) -> Result<(), BackendError> {
    for topic in topics.into_values() {
        bus.unregister(&topic).map_err(failed)?;
    }
    Ok(())
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

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sparse_large_directory_delivers_a_level_in_one_pump() {
        let hub = SignalHub::new(moiren_signal::BusConfig {
            max_topics: 8192,
            ..Default::default()
        })
        .unwrap();
        let (control, mut host) = hub.create_host(1, 1).unwrap();
        let (topic, mut subscriber) = {
            let mut directory = hub.directory.lock().unwrap();
            let topic = directory
                .bus
                .register_latest_value("sparse/level", 0.0_f32)
                .unwrap();
            let subscriber = directory.bus.subscribe(&topic, Default::default()).unwrap();
            (topic, subscriber)
        };
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let operation = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        host.scope(&operation)
            .publish(&publisher.handle(), 0.75)
            .unwrap();
        hub.pump();
        assert_eq!(
            subscriber.read_latest().map(|value| value.value),
            Some(0.75)
        );
    }

    #[test]
    fn high_rate_polling_follows_demand_and_respects_a_faster_configured_poll() {
        let hub = SignalHub::new(Default::default()).unwrap();
        let idle = Duration::from_millis(20);
        assert_eq!(hub.poll_interval(idle), idle);
        let subscriber = {
            let mut directory = hub.directory.lock().unwrap();
            let topic = directory
                .bus
                .register_latest_value("scheduling-test", 0.0_f32)
                .unwrap();
            directory.bus.subscribe(&topic, Default::default()).unwrap()
        };
        assert_eq!(hub.poll_interval(idle), Duration::from_millis(1));
        assert_eq!(
            hub.poll_interval(Duration::from_millis(1)),
            Duration::from_millis(1)
        );
        drop(subscriber);
        assert_eq!(hub.poll_interval(idle), idle);
    }
}
