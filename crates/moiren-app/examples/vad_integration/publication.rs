//! Observe service publication while the native coordinator is inside WASAPI.
use anyhow::{Context, ensure};
use serde::Serialize;
use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(super) const FRESHNESS_LIMIT: Duration = Duration::from_secs(2);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(20);

type Snapshot<T> = (Arc<T>, Instant);

#[derive(Debug, Clone, Serialize)]
pub(super) struct PublicationEvidence {
    samples: u64,
    readable_samples: u64,
    advances: u64,
    longest_sample_gap_ms: u128,
    longest_without_advance_ms: u128,
    last_advance_ms: Option<u128>,
}

pub(super) struct PublicationWatch<T> {
    started: Instant,
    last_sample: Instant,
    latest: Option<Snapshot<T>>,
    samples: u64,
    readable_samples: u64,
    advances: u64,
    longest_sample_gap: Duration,
    longest_without_advance: Duration,
}

impl<T> PublicationWatch<T> {
    fn new(started: Instant, latest: Option<Snapshot<T>>) -> Self {
        Self {
            started,
            last_sample: started,
            latest,
            samples: 0,
            readable_samples: 0,
            advances: 0,
            longest_sample_gap: Duration::ZERO,
            longest_without_advance: Duration::ZERO,
        }
    }

    fn sample(&mut self, snapshot: Option<Arc<T>>, now: Instant) {
        self.samples += 1;
        self.longest_sample_gap = self.longest_sample_gap.max(now - self.last_sample);
        self.last_sample = now;
        if let Some(snapshot) = snapshot {
            self.readable_samples += 1;
            if self
                .latest
                .as_ref()
                .is_none_or(|(previous, _)| !Arc::ptr_eq(previous, &snapshot))
            {
                self.advances += 1;
                self.latest = Some((snapshot, now));
                return;
            }
        }
        let last_advance = self.latest.as_ref().map_or(self.started, |(_, at)| *at);
        self.longest_without_advance = self.longest_without_advance.max(now - last_advance);
    }

    pub(super) fn require_fresh(&self) -> anyhow::Result<()> {
        ensure!(
            self.longest_sample_gap <= FRESHNESS_LIMIT,
            "native prepare: snapshot sampling gap of {} ms",
            self.longest_sample_gap.as_millis(),
        );
        ensure!(
            self.latest.is_some() && self.longest_without_advance <= FRESHNESS_LIMIT,
            "native prepare: no new readable snapshot for {} ms",
            self.longest_without_advance.as_millis(),
        );
        Ok(())
    }

    pub(super) fn evidence(&self, origin: Instant) -> PublicationEvidence {
        PublicationEvidence {
            samples: self.samples,
            readable_samples: self.readable_samples,
            advances: self.advances,
            longest_sample_gap_ms: self.longest_sample_gap.as_millis(),
            longest_without_advance_ms: self.longest_without_advance.as_millis(),
            last_advance_ms: self
                .latest
                .as_ref()
                .map(|(_, at)| (*at - origin).as_millis()),
        }
    }

    pub(super) fn into_latest(self) -> Option<Snapshot<T>> {
        self.latest
    }
}

pub(super) fn observe_during<T: Send + Sync, R>(
    initial: Option<Snapshot<T>>,
    mut snapshot: impl FnMut() -> Option<Arc<T>> + Send,
    prepare: impl FnOnce() -> R,
) -> anyhow::Result<(R, PublicationWatch<T>)> {
    thread::scope(|scope| {
        let (finished, stop) = mpsc::channel::<()>();
        let mut watch = PublicationWatch::new(Instant::now(), initial);
        // Keep COM objects and the mutable probe report on the coordinator.
        // This observer only reads the mailbox while native calls can block.
        let worker = thread::Builder::new()
            .name("vad-prepare-publication".into())
            .spawn_scoped(scope, move || {
                loop {
                    watch.sample(snapshot(), Instant::now());
                    if stop.recv_timeout(SAMPLE_INTERVAL)
                        == Err(mpsc::RecvTimeoutError::Disconnected)
                    {
                        break;
                    }
                }
                watch.sample(snapshot(), Instant::now());
                watch
            })
            .context("start native prepare publication observer")?;
        let result = prepare();
        drop(finished);
        let watch = worker
            .join()
            .map_err(|_| anyhow::anyhow!("native prepare publication observer panicked"))?;
        Ok((result, watch))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    #[test]
    fn publications_can_stay_fresh_across_a_long_native_prepare() {
        let start = Instant::now();
        let mut watch = PublicationWatch::new(start, Some((Arc::new(0), start)));
        for generation in 1..=4 {
            watch.sample(
                Some(Arc::new(generation)),
                start + Duration::from_millis(750 * generation),
            );
        }
        assert!(watch.require_fresh().is_ok());
        assert_eq!(watch.evidence(start).advances, 4);
        assert_eq!(watch.evidence(start).last_advance_ms, Some(3_000));
    }

    #[test]
    fn a_later_publication_does_not_erase_a_gap() {
        let start = Instant::now();
        let snapshot = Arc::new(0);
        let mut watch = PublicationWatch::new(start, Some((snapshot.clone(), start)));
        watch.sample(Some(snapshot), start + Duration::from_secs(1));
        watch.sample(None, start + Duration::from_millis(2_001));
        watch.sample(Some(Arc::new(1)), start + Duration::from_secs(3));
        let error = watch.require_fresh().unwrap_err().to_string();
        assert!(error.contains("no new readable snapshot"));
        assert_eq!(watch.evidence(start).longest_without_advance_ms, 2_001);
        assert_eq!(watch.evidence(start).readable_samples, 2);
    }

    #[test]
    fn a_gap_in_sampling_cannot_attest_publication_continuity() {
        let start = Instant::now();
        let mut watch = PublicationWatch::new(start, Some((Arc::new(0), start)));
        watch.sample(Some(Arc::new(1)), start + Duration::from_millis(2_001));
        let error = watch.require_fresh().unwrap_err().to_string();
        assert!(error.contains("sampling gap"));
        assert_eq!(watch.evidence(start).longest_sample_gap_ms, 2_001);
    }

    #[test]
    fn samples_the_mailbox_while_preparation_waits_on_its_own_thread() {
        let (observed, signal) = mpsc::channel();
        let snapshot = Arc::new(1);
        let expected = snapshot.clone();
        let (prepared, watch) = observe_during(
            None,
            move || {
                let _ = observed.send(());
                Some(snapshot.clone())
            },
            || signal.recv_timeout(Duration::from_secs(2)),
        )
        .unwrap();
        prepared.unwrap();
        assert!(Arc::ptr_eq(&watch.into_latest().unwrap().0, &expected));
    }
}
