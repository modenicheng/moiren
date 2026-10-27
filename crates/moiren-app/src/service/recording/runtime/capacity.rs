//! Optional capacity IO must never own or delay a recording session's stop path.
use std::{sync::Weak, time::Instant};

use super::*;

#[derive(Default)]
pub(super) struct Snapshot {
    observation: Option<disk::Observation>,
    checked_at: Option<Instant>,
}

impl Snapshot {
    pub(super) fn apply(&self, snapshot: &mut RecordingSnapshot) {
        snapshot.estimate.free_disk_bytes = self.observation.as_ref().map(|disk| disk.available);
        snapshot.estimate.disk_volume = self
            .observation
            .as_ref()
            .and_then(|disk| disk.volume.clone());
        // Terminal tasks are reaped before aggregate_estimates, so recalculate
        // the single-file budget here too; live tasks are aggregated afterward.
        snapshot
            .estimate
            .apply_disk_budget(snapshot.estimate.bytes_per_second.unwrap_or(0));
    }
}

struct Worker {
    node: NodeId,
    owner: Weak<Shared>,
    thread: JoinHandle<Option<disk::Observation>>,
}

#[cfg(test)]
type Probe = dyn Fn(&Path) -> Option<disk::Observation> + Send + Sync;

/// Keep one live query, including across stop/load/restart. A stuck filesystem
/// call delays estimates only; dropping its JoinHandle never waits for it.
#[derive(Default)]
pub(super) struct Capacity {
    worker: Option<Worker>,
    #[cfg(test)]
    probe: Option<Arc<Probe>>,
}

impl Capacity {
    #[cfg(test)]
    pub(super) fn set_probe(
        &mut self,
        probe: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static,
    ) {
        self.probe = Some(Arc::new(move |path| {
            probe(path).map(|available| disk::Observation {
                available,
                volume: None,
            })
        }));
    }

    pub(super) fn poll(&mut self, tasks: &mut BTreeMap<NodeId, Task>) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            let worker = self.worker.take().expect("finished capacity probe");
            let observation = worker.thread.join().ok().flatten();
            // NodeId and destination can both be reused after load. Only the
            // exact original task may receive this read-only observation.
            if let Some(task) = tasks.get_mut(&worker.node)
                && let Some(owner) = worker.owner.upgrade()
                && Arc::ptr_eq(&task.shared, &owner)
                && !task.shared.stop.load(Ordering::Acquire)
            {
                task.disk.observation = observation;
                task.disk.checked_at = Some(Instant::now());
            }
        }
        if self.worker.is_some() {
            return;
        }
        // Oldest observation first prevents a fast first node starving peers.
        let Some((&node, task)) = tasks
            .iter_mut()
            .filter(|(_, task)| {
                task.last_snapshot.phase == RecordingPhase::Recording
                    && !task.shared.stop.load(Ordering::Acquire)
                    && !task.thread.is_finished()
                    && task
                        .disk
                        .checked_at
                        .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            })
            .min_by_key(|(_, task)| task.disk.checked_at)
        else {
            return;
        };
        let Some(path) = task.last_snapshot.path.clone() else {
            return;
        };
        task.disk.checked_at = Some(Instant::now());
        let owner = Arc::downgrade(&task.shared);
        let wake = thread::current();
        #[cfg(test)]
        let probe = self.probe.clone();
        let thread = thread::Builder::new()
            .name("app-recording-capacity".into())
            .spawn(move || {
                #[cfg(test)]
                let observation = match probe {
                    Some(probe) => probe(&path),
                    None => disk::observe(&path),
                };
                #[cfg(not(test))]
                let observation = disk::observe(&path);
                wake.unpark();
                observation
            });
        if let Ok(thread) = thread {
            self.worker = Some(Worker {
                node,
                owner,
                thread,
            });
        }
    }
}
