//! Single-flight metadata reads, independent of the driver control session.

use super::{
    CacheEntry, EndpointNameReader, NEGATIVE_RETRY_INTERVAL, NameKey, NameObserver, ObserverClock,
    VadEndpointNameObservation, VadEndpointNameTarget, read_names,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
    thread,
    time::Duration,
};

// An in-flight COM call can outlive its provider. Keep the process-wide slot
// until the call returns, including across service replacement.
#[cfg(windows)]
pub(in crate::vad) static NAME_READ_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct ReadPermit(&'static AtomicBool);

impl ReadPermit {
    fn acquire(slot: &'static AtomicBool) -> Option<Self> {
        slot.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self(slot))
    }
}

impl Drop for ReadPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Default)]
pub(in crate::vad) struct NameRefresh {
    observer: NameObserver,
    result: Option<Receiver<Vec<CacheEntry>>>,
    retry_at: Duration,
}

impl NameRefresh {
    pub(in crate::vad) fn poll<R, C>(
        &mut self,
        exports: &[VadEndpointNameTarget],
        now: Duration,
        slot: &'static AtomicBool,
        source: impl FnOnce() -> (R, C),
    ) -> Vec<VadEndpointNameObservation>
    where
        R: EndpointNameReader + Send + 'static,
        C: ObserverClock + Send + 'static,
    {
        if let Some(result) = &self.result {
            match result.try_recv() {
                Ok(entries) => {
                    self.observer.complete(entries);
                    self.result = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.result = None;
                    self.retry_at = now + NEGATIVE_RETRY_INTERVAL;
                }
            }
        }

        let keys = NameKey::eligible(exports);
        self.observer.retain(&keys);
        if self.result.is_none() && now >= self.retry_at {
            let pending = self.observer.due(&keys, now);
            if !pending.is_empty()
                && let Some(permit) = ReadPermit::acquire(slot)
            {
                // Only the detached inputs and a result sender reach the
                // thread. Dropping this refresh never joins blocked COM.
                let (mut reader, mut clock) = source();
                let (sender, result) = sync_channel(1);
                let started = thread::Builder::new()
                    .name("vad-endpoint-names".into())
                    .spawn(move || {
                        let entries = read_names(pending, &mut reader, &mut clock);
                        drop(permit);
                        let _ = sender.send(entries);
                    });
                match started {
                    Ok(worker) => {
                        drop(worker);
                        self.result = Some(result);
                    }
                    Err(error) => {
                        self.retry_at = now + NEGATIVE_RETRY_INTERVAL;
                        log::debug!(
                            target: "moiren::audio::vad::name",
                            "could not start endpoint name read: {error}"
                        );
                    }
                }
            }
        }
        self.observer.observations(&keys, now)
    }
}
