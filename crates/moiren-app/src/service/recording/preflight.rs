//! Bounded, read-only capacity probes. No file or engine endpoint is opened.
use std::thread::{self, JoinHandle};

use super::*;

#[derive(Clone, PartialEq, Eq)]
struct Request {
    ticket: u64,
    node: NodeId,
    path: PathBuf,
    rate: u32,
    channels: usize,
}

struct Worker {
    request: Request,
    thread: JoinHandle<Option<disk::Observation>>,
}

/// One OS query at a time; repeated edits replace that node's queued request.
/// Monotonic tickets survive reset so even identical NodeId/path/rate triples
/// in a loaded project cannot receive a retired document's observation.
#[derive(Default)]
pub(super) struct Preflights {
    next_ticket: u64,
    requests: BTreeMap<NodeId, Request>,
    worker: Option<Worker>,
}

impl Preflights {
    pub(super) fn has_pending(&self) -> bool {
        !self.requests.is_empty()
    }

    pub(super) fn request(
        &mut self,
        service: &mut RecordingService,
        node: NodeId,
        rate: u32,
        channels: usize,
    ) -> Result<(), String> {
        let entry = service
            .entries
            .get_mut(&node)
            .ok_or("Configure the recording destination first")?;
        let path = entry
            .path
            .clone()
            .ok_or("Configure the recording destination first")?;
        if entry.phase != RecordingPhase::Idle {
            return Err("Capacity preflight requires an idle recording destination".into());
        }
        let ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("Recording preflight identity exhausted")?;
        self.next_ticket = ticket;
        entry.estimate = RecordingEstimate::float_wav(rate, channels, 0, None);
        entry.estimate.proposed_sample_rate = Some(rate);
        entry.estimate.proposed_channels = Some(channels);
        entry.estimate.preflight_pending = true;
        self.requests.insert(
            node,
            Request {
                ticket,
                node,
                path,
                rate,
                channels,
            },
        );
        Ok(())
    }

    pub(super) fn cancel(&mut self, service: &mut RecordingService, node: NodeId) {
        self.requests.remove(&node);
        if let Some(entry) = service.entries.get_mut(&node) {
            entry.estimate.preflight_pending = false;
        }
    }

    pub(super) fn cancel_all(&mut self, service: &mut RecordingService) {
        self.requests.clear();
        for entry in service.entries.values_mut() {
            entry.estimate.preflight_pending = false;
        }
        // An in-flight read-only OS call cannot be interrupted. Its join handle
        // may safely detach on service exit: it owns no file/session or UI state.
    }

    pub(super) fn refresh(
        &mut self,
        service: &mut RecordingService,
        graph: &GraphSnapshot,
        rate: u32,
    ) {
        self.requests.retain(|node, request| {
            service.entries.get(node).is_some_and(|entry| {
                entry.phase == RecordingPhase::Idle
                    && entry.path.as_ref() == Some(&request.path)
                    && graph.kind_of(*node) == Some(NodeKind::Recorder)
            })
        });
        for entry in service
            .entries
            .values_mut()
            .filter(|entry| entry.phase != RecordingPhase::Idle)
        {
            entry.estimate.preflight_pending = false;
        }
        let changed: Vec<_> = service
            .entries
            .values()
            .filter_map(|entry| {
                if entry.phase != RecordingPhase::Idle || entry.path.is_none() {
                    return None;
                }
                let node = graph.graph.get_node(entry.node).ok()?;
                (node.kind() == NodeKind::Recorder
                    && (entry.estimate.proposed_sample_rate != Some(rate)
                        || entry.estimate.proposed_channels != Some(node.channels())))
                .then_some((entry.node, node.channels()))
            })
            .collect();
        for (node, channels) in changed {
            if let Err(error) = self.request(service, node, rate, channels) {
                log::warn!("Recording capacity preflight unavailable: {error}");
            }
        }
    }

    pub(super) fn poll(&mut self, service: &mut RecordingService) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            let worker = self.worker.take().expect("finished query");
            let observation = worker.thread.join().ok().flatten();
            self.complete(service, &worker.request, observation);
        }
        if self.worker.is_some() {
            return;
        }
        let Some(request) = self.requests.values().next().cloned() else {
            return;
        };
        let path = request.path.clone();
        let wake = thread::current();
        match thread::Builder::new()
            .name("app-recording-preflight".into())
            .spawn(move || {
                let result = disk::observe(&path);
                wake.unpark();
                result
            }) {
            Ok(thread) => self.worker = Some(Worker { request, thread }),
            Err(error) => {
                log::warn!("Recording capacity probe could not start: {error}");
                self.complete(service, &request, None);
            }
        }
    }

    fn complete(
        &mut self,
        service: &mut RecordingService,
        request: &Request,
        observation: Option<disk::Observation>,
    ) {
        if self.requests.get(&request.node) != Some(request) {
            return;
        }
        self.requests.remove(&request.node);
        let Some(entry) = service.entries.get_mut(&request.node) else {
            return;
        };
        if entry.phase != RecordingPhase::Idle
            || entry.path.as_ref() != Some(&request.path)
            || entry.estimate.proposed_sample_rate != Some(request.rate)
            || entry.estimate.proposed_channels != Some(request.channels)
        {
            return;
        }
        let mut estimate = RecordingEstimate::float_wav(
            request.rate,
            request.channels,
            0,
            observation.as_ref().map(|disk| disk.available),
        );
        estimate.disk_volume = observation.and_then(|disk| disk.volume);
        estimate.proposed_sample_rate = Some(request.rate);
        estimate.proposed_channels = Some(request.channels);
        entry.estimate = estimate;
    }
}

#[cfg(test)]
mod tests;
