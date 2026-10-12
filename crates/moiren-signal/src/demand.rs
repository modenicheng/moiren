use crate::SubscribeError;
use std::sync::atomic::{AtomicU64, Ordering};

const COUNT_MASK: u64 = 0x7f;
const CLOSED_MASK: u64 = 0x80;
const MAX_GENERATION: u64 = u64::MAX >> 8;

/// One atomic observation of business demand and its activation epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemandSnapshot {
    pub active: bool,
    pub generation: u64,
}

pub(crate) struct Demand(AtomicU64);

impl Demand {
    pub(crate) fn new() -> Self {
        Self(AtomicU64::new(0))
    }
    pub(crate) fn snapshot(&self) -> DemandSnapshot {
        let word = self.0.load(Ordering::Acquire);
        DemandSnapshot {
            active: word & COUNT_MASK != 0 && word & CLOSED_MASK == 0,
            generation: word >> 8,
        }
    }
    pub(crate) fn subscriber_count(&self) -> usize {
        (self.0.load(Ordering::Acquire) & COUNT_MASK) as usize
    }
    pub(crate) fn check_add(&self, limit: usize) -> Result<(), SubscribeError> {
        next_subscription_word(self.0.load(Ordering::Acquire), limit).map(|_| ())
    }
    pub(crate) fn add(&self, limit: usize) -> Result<DemandSnapshot, SubscribeError> {
        let mut word = self.0.load(Ordering::Acquire);
        loop {
            let next = next_subscription_word(word, limit)?;
            match self
                .0
                .compare_exchange_weak(word, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Ok(DemandSnapshot {
                        active: true,
                        generation: next >> 8,
                    });
                }
                Err(current) => word = current,
            }
        }
    }
    pub(crate) fn remove(&self) {
        let mut word = self.0.load(Ordering::Acquire);
        loop {
            assert_ne!(
                word & COUNT_MASK,
                0,
                "live subscription owns a demand count"
            );
            match self
                .0
                .compare_exchange_weak(word, word - 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(current) => word = current,
            }
        }
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.0.load(Ordering::Acquire) & CLOSED_MASK != 0
    }
    // Closure preserves the real count so existing lease drops cannot underflow.
    pub(crate) fn close(&self) {
        self.0.fetch_or(CLOSED_MASK, Ordering::AcqRel);
    }
}

fn next_subscription_word(word: u64, limit: usize) -> Result<u64, SubscribeError> {
    if word & CLOSED_MASK != 0 {
        return Err(SubscribeError::Closed);
    }
    let count = word & COUNT_MASK;
    if count >= limit.min(64) as u64 {
        return Err(SubscribeError::SubscriberLimit);
    }
    let generation = if count == 0 {
        (word >> 8)
            .checked_add(1)
            .filter(|generation| *generation <= MAX_GENERATION)
            .ok_or(SubscribeError::DemandExhausted)?
    } else {
        word >> 8
    };
    Ok((generation << 8) | (count + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_to_positive_advances_generation_even_between_rt_observations() {
        let demand = Demand::new();
        assert_eq!(
            demand.snapshot(),
            DemandSnapshot {
                active: false,
                generation: 0
            }
        );
        assert_eq!(
            demand.add(64).unwrap(),
            DemandSnapshot {
                active: true,
                generation: 1
            }
        );
        demand.remove();
        assert_eq!(
            demand.add(64).unwrap(),
            DemandSnapshot {
                active: true,
                generation: 2
            }
        );
    }
    #[test]
    fn continuous_demand_limits_and_closed_drop_preserve_the_epoch() {
        let demand = Demand::new();
        demand.add(2).unwrap();
        assert_eq!(demand.add(2).unwrap().generation, 1);
        assert_eq!(demand.add(2), Err(SubscribeError::SubscriberLimit));
        demand.remove();
        assert_eq!(demand.add(2).unwrap().generation, 1);
        demand.close();
        assert!(!demand.snapshot().active);
        assert_eq!(demand.add(2), Err(SubscribeError::Closed));
        demand.remove();
        demand.remove();
        assert_eq!(demand.0.load(Ordering::Acquire), (1 << 8) | CLOSED_MASK);
    }
    #[test]
    fn exhausted_generation_does_not_change_the_word() {
        let demand = Demand(AtomicU64::new(MAX_GENERATION << 8));
        assert_eq!(demand.add(64), Err(SubscribeError::DemandExhausted));
        assert_eq!(demand.0.load(Ordering::Acquire), MAX_GENERATION << 8);
    }

    #[test]
    fn encoded_count_never_exceeds_sixty_four() {
        let demand = Demand::new();
        for _ in 0..64 {
            assert_eq!(demand.add(100).unwrap().generation, 1);
        }
        assert_eq!(demand.add(100), Err(SubscribeError::SubscriberLimit));
        assert_eq!(demand.0.load(Ordering::Acquire) & COUNT_MASK, 64);
        for _ in 0..64 {
            demand.remove();
        }
        assert_eq!(
            demand.snapshot(),
            DemandSnapshot {
                active: false,
                generation: 1
            }
        );
    }

    #[test]
    fn concurrent_subscription_drops_preserve_one_atomic_epoch() {
        let demand = Demand::new();
        let added = std::sync::Barrier::new(9);
        let release = std::sync::Barrier::new(9);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    assert_eq!(demand.add(64).unwrap().generation, 1);
                    added.wait();
                    release.wait();
                    demand.remove();
                });
            }
            added.wait();
            assert_eq!(demand.0.load(Ordering::Acquire), (1 << 8) | 8);
            release.wait();
        });
        assert_eq!(
            demand.snapshot(),
            DemandSnapshot {
                active: false,
                generation: 1
            }
        );
        assert_eq!(demand.add(64).unwrap().generation, 2);
        demand.remove();
    }
}
