use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use moiren_engine::{
    host::ManagedRecorderReader,
    recorder::{RecordingState, RecordingStatus},
};

use super::*;

mod capacity;

const MAX_RECORDINGS: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct Shared {
    stop: AtomicBool,
    snapshot: Mutex<RecordingSnapshot>,
}

struct Task {
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
    last_snapshot: RecordingSnapshot,
    disk: capacity::Snapshot,
}

impl Task {
    fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.thread.thread().unpark();
    }

    fn snapshot(&self) -> Option<RecordingSnapshot> {
        match self.shared.snapshot.try_lock() {
            Ok(snapshot) => Some(snapshot.clone()),
            Err(TryLockError::Poisoned(poison)) => Some(poison.into_inner().clone()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// Each task owns its RecordingSession on a dedicated non-RT thread from open
/// through finish/drop. Neither shared job lane ever joins a live recording:
/// cleanup can retire the audio host even if file IO or finalization stalls.
#[derive(Default)]
pub(crate) struct Runtime {
    tasks: BTreeMap<NodeId, Task>,
    retired: Vec<Task>,
    preflights: super::preflight::Preflights,
    capacity: capacity::Capacity,
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn set_capacity_probe_for_test(
        &mut self,
        probe: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static,
    ) {
        self.capacity.set_probe(probe);
    }

    pub(crate) fn is_drained(&self) -> bool {
        // Optional live capacity IO owns no session and cannot delay shutdown.
        self.tasks.is_empty() && self.retired.is_empty() && !self.preflights.has_pending()
    }

    pub(crate) fn preflight(
        &mut self,
        service: &mut RecordingService,
        node: NodeId,
        rate: u32,
        channels: usize,
    ) -> Result<(), String> {
        self.preflights.request(service, node, rate, channels)
    }

    pub(crate) fn refresh_preflights(
        &mut self,
        service: &mut RecordingService,
        graph: &GraphSnapshot,
        rate: u32,
    ) {
        self.preflights.refresh(service, graph, rate);
    }

    pub(crate) fn cancel_preflights(&mut self, service: &mut RecordingService) {
        self.preflights.cancel_all(service);
    }

    pub(crate) fn cancel_preflight(&mut self, service: &mut RecordingService, node: NodeId) {
        self.preflights.cancel(service, node);
    }

    pub(crate) fn is_busy(&self, node: NodeId) -> bool {
        self.tasks.contains_key(&node)
            || self
                .retired
                .iter()
                .any(|task| task.last_snapshot.node == node)
    }

    pub(crate) fn start(
        &mut self,
        service: &mut RecordingService,
        node: NodeId,
        reader: ManagedRecorderReader,
        sample_rate: u32,
        channels: usize,
    ) -> Result<(), String> {
        if self.is_busy(node) {
            return Err("Recording is already active or finalizing".into());
        }
        self.cancel_preflight(service, node);
        if self.tasks.len() + self.retired.len() >= MAX_RECORDINGS {
            return Err("At most eight recordings may be active or finalizing".into());
        }
        let path = service
            .entries
            .get(&node)
            .and_then(|entry| entry.path.clone())
            .ok_or("Choose a .wav destination before starting recording")?;
        let mut snapshot = RecordingSnapshot::idle(node);
        snapshot.format = service.entries[&node].format;
        snapshot.path = Some(path);
        snapshot.phase = RecordingPhase::Starting;
        snapshot.sample_rate = Some(sample_rate);
        snapshot.channels = Some(channels);
        snapshot.estimate = RecordingEstimate::wav(snapshot.format, sample_rate, channels, 0, None);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            snapshot: Mutex::new(snapshot.clone()),
        });
        let worker_shared = shared.clone();
        let wake = thread::current();
        let worker = thread::Builder::new()
            .name("app-recording".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run(reader, &worker_shared, sample_rate, channels)
                }))
                .unwrap_or_else(|_| Err(io::Error::other("recording service worker panicked")));
                if let Err(error) = result {
                    let mut snapshot = worker_shared
                        .snapshot
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    snapshot.phase = RecordingPhase::Failed;
                    snapshot.error = Some(error.to_string());
                }
                wake.unpark();
            })
            .map_err(|error| error.to_string())?;
        service.entries.insert(node, snapshot.clone());
        self.tasks.insert(
            node,
            Task {
                shared,
                thread: worker,
                last_snapshot: snapshot,
                disk: capacity::Snapshot::default(),
            },
        );
        Ok(())
    }

    pub(crate) fn stop(&self, service: &mut RecordingService, node: NodeId) {
        if let Some(task) = self.tasks.get(&node) {
            task.stop();
            if let Some(snapshot) = service.entries.get_mut(&node) {
                snapshot.phase = RecordingPhase::Stopping;
            }
        }
    }

    pub(crate) fn stop_all(&self, service: &mut RecordingService) {
        for &node in self.tasks.keys() {
            self.stop(service, node);
        }
        for task in &self.retired {
            task.stop();
        }
    }

    pub(crate) fn prepare_apply(
        &self,
        service: &mut RecordingService,
        project: &moiren_engine::host::EngineProject,
        active: Option<&moiren_engine::host::EngineProject>,
    ) -> bool {
        let mut ready = self.retired.is_empty();
        for (&node, task) in &self.tasks {
            let snapshot = &task.last_snapshot;
            let rate_changed = snapshot.sample_rate.map(f64::from)
                != Some(project.processing.engine.processing_sr);
            let channels_changed = project
                .graph
                .get_node(node)
                .ok()
                .map(|node| node.channels())
                != snapshot.channels;
            let config_changed =
                active.and_then(|active| active.nodes.get(&node)) != project.nodes.get(&node);
            if rate_changed || channels_changed || config_changed {
                self.stop(service, node);
                ready = false;
            }
        }
        ready
    }

    pub(crate) fn reset(&mut self, service: &mut RecordingService) {
        self.stop_all(service);
        self.preflights.cancel_all(service);
        // Retired tasks keep their original managed reader and cannot publish
        // into the replacement document's NodeId-indexed snapshot map. Apply
        // waits for this list to drain before another host can reuse those IDs.
        self.retired
            .extend(std::mem::take(&mut self.tasks).into_values());
        service.clear();
    }

    pub(crate) fn reconcile(&mut self, service: &mut RecordingService, graph: &GraphSnapshot) {
        let removed: Vec<_> = self
            .tasks
            .keys()
            .copied()
            .filter(|node| graph.kind_of(*node) != Some(NodeKind::Recorder))
            .collect();
        for node in removed {
            let task = self.tasks.remove(&node).expect("collected task");
            task.stop();
            self.retired.push(task);
        }
        service
            .entries
            .retain(|node, _| graph.kind_of(*node) == Some(NodeKind::Recorder));
    }

    pub(crate) fn poll(&mut self, service: &mut RecordingService) -> Vec<String> {
        self.preflights.poll(service);
        self.capacity.poll(&mut self.tasks);
        let mut errors = Vec::new();
        let mut finished = Vec::new();
        for (&node, task) in &mut self.tasks {
            let done = task.thread.is_finished();
            // Never queue behind file-worker publication on the service/UI
            // path. A terminal status is exposed only after its owner finishes.
            if let Some(mut snapshot) = task.snapshot() {
                task.disk.apply(&mut snapshot);
                if !done
                    && (matches!(
                        snapshot.phase,
                        RecordingPhase::Canceled
                            | RecordingPhase::Finished
                            | RecordingPhase::Failed
                    ) || task.shared.stop.load(Ordering::Acquire))
                {
                    snapshot.phase = RecordingPhase::Stopping;
                }
                task.last_snapshot = snapshot.clone();
                service.entries.insert(node, snapshot);
                if done {
                    finished.push(node);
                }
            }
        }
        for node in finished {
            let task = self.tasks.remove(&node).expect("finished task");
            // is_finished above means this join cannot wait on file IO, audio
            // quiescence, engine locks or a queued cleanup job.
            if task.thread.join().is_err() {
                service.fail(node, "recording service worker panicked");
            }
        }
        let mut index = 0;
        while index < self.retired.len() {
            if let Some(mut snapshot) = self.retired[index].snapshot() {
                self.retired[index].disk.apply(&mut snapshot);
                self.retired[index].last_snapshot = snapshot;
            }
            if !self.retired[index].thread.is_finished() {
                index += 1;
                continue;
            }
            let task = self.retired.swap_remove(index);
            if let Some(snapshot) = task.snapshot()
                && let Some(error) = &snapshot.error
            {
                errors.push(format!("Recording {:?}: {error}", snapshot.path));
            }
            if task.thread.join().is_err() {
                errors.push("Retired recording worker panicked".into());
            }
        }
        let writers: Vec<_> = self
            .tasks
            .values()
            .chain(self.retired.iter())
            .map(|task| &task.last_snapshot)
            .collect();
        aggregate_estimates(service, &writers);
        errors
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // The orderly service exit polls until drained. An unwind still signals
        // every worker, but never drops a blocking RecordingSession on this thread.
        for task in self.tasks.values().chain(self.retired.iter()) {
            task.stop();
        }
    }
}

fn publish(shared: &Shared, status: RecordingStatus, rate: u32, channels: usize) {
    let mut snapshot = shared.snapshot.lock().unwrap_or_else(|p| p.into_inner());
    snapshot.phase = match status.state {
        RecordingState::Recording => RecordingPhase::Recording,
        RecordingState::Stopping => RecordingPhase::Stopping,
        RecordingState::Finished => RecordingPhase::Finished,
        RecordingState::Failed => RecordingPhase::Failed,
    };
    snapshot.written_frames = status.written_frames;
    snapshot.stats = Some(status.stats);
    snapshot.error = status.error.map(|error| error.message);
    snapshot.estimate =
        RecordingEstimate::wav(snapshot.format, rate, channels, status.written_frames, None);
}

fn aggregate_estimates(service: &mut RecordingService, writers: &[&RecordingSnapshot]) {
    for snapshot in service
        .entries
        .values_mut()
        .filter(|entry| entry.phase.is_busy() || entry.estimate.proposed_sample_rate.is_some())
    {
        let estimate = &mut snapshot.estimate;
        // A prospective file adds its own rate; idle destinations are not
        // counted as simultaneous writers in other nodes' estimates.
        let mut aggregate = if estimate.proposed_sample_rate.is_some() {
            estimate.bytes_per_second.unwrap_or(0)
        } else {
            0
        };
        for writer in writers {
            let same_or_unknown = match (&estimate.disk_volume, &writer.estimate.disk_volume) {
                (Some(volume), Some(other)) => volume == other,
                _ => true,
            };
            if same_or_unknown {
                aggregate = aggregate.saturating_add(writer.estimate.bytes_per_second.unwrap_or(0));
            }
            if estimate.disk_volume.is_some()
                && estimate.disk_volume == writer.estimate.disk_volume
                && let (Some(free), Some(other)) =
                    (estimate.free_disk_bytes, writer.estimate.free_disk_bytes)
            {
                estimate.free_disk_bytes = Some(free.min(other));
            }
        }
        estimate.apply_disk_budget(aggregate);
    }
}

fn open_destination(shared: &Shared) -> io::Result<Option<File>> {
    if shared.stop.load(Ordering::Acquire) {
        shared
            .snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .phase = RecordingPhase::Canceled;
        return Ok(None);
    }
    let path = shared
        .snapshot
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .path
        .clone()
        .expect("configured path");
    // Do not use record_wav: its File::create truncates. Atomic create_new also
    // refuses symlinks/existing files without a racy exists-then-open check.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(Some)
}

fn run(
    reader: ManagedRecorderReader,
    shared: &Shared,
    sample_rate: u32,
    channels: usize,
) -> io::Result<()> {
    let Some(file) = open_destination(shared)? else {
        return Ok(());
    };
    let format = shared
        .snapshot
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .format
        .wav_format();
    // Claim/drain the original endpoint, never look up a node on a newer host.
    // The discard session proves quiescence, including an in-flight RT block;
    // until the actual file session starts, the returned reader stays stopped.
    super::discard::before_start(&reader, sample_rate)?;
    if shared.stop.load(Ordering::Acquire) {
        // Cancellation during open/discard must not re-arm capture. Preserve the
        // exclusively-created path as a valid empty WAV rather than a partial file.
        moiren_engine::recorder::WavRecording::from_writer_with_format(
            BufWriter::new(file),
            channels,
            sample_rate,
            format,
        )?
        .finish()?;
        shared
            .snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .phase = RecordingPhase::Finished;
        return Ok(());
    }
    let session = reader.record_with_writer_format(BufWriter::new(file), sample_rate, format)?;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            session.stop();
        }
        let status = session.status();
        let done = matches!(
            status.state,
            RecordingState::Finished | RecordingState::Failed
        );
        // Capacity probes own no session and never run on this control thread:
        // even a stuck network filesystem query cannot delay session.stop().
        publish(shared, status, sample_rate, channels);
        if done {
            // finish joins only here; no service locks are held and host cleanup
            // has an independent lane, so neither can wait for the other.
            session.finish()?;
            return Ok(());
        }
        thread::park_timeout(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests;
