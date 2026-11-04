use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Fixture epoch source for the in-crate graph compile tests. This crate's
/// production paths never build a `CompileConfig` themselves — the managed
/// engine host owns real epochs — so nothing here has a production caller.
pub(crate) fn next_epoch() -> Option<u64> {
    allocate_epoch(&NEXT_EPOCH)
}

fn allocate_epoch(next: &AtomicU64) -> Option<u64> {
    next.try_update(Ordering::Relaxed, Ordering::Relaxed, |epoch| {
        epoch.checked_add(1)
    })
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustion_never_wraps_or_reuses_an_epoch() {
        let next = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_epoch(&next), Some(u64::MAX - 1));
        assert_eq!(allocate_epoch(&next), None);
        assert_eq!(allocate_epoch(&next), None);
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn concurrent_preparations_receive_distinct_epochs() {
        let next = AtomicU64::new(1);
        let mut epochs = std::thread::scope(|scope| {
            let preparations: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..64)
                            .map(|_| allocate_epoch(&next).unwrap())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            preparations
                .into_iter()
                .flat_map(|preparation| preparation.join().unwrap())
                .collect::<Vec<_>>()
        });
        epochs.sort_unstable();
        assert_eq!(epochs, (1..=512).collect::<Vec<_>>());
    }
}
