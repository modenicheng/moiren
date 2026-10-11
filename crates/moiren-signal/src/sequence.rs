use crate::PublishError;
use std::sync::atomic::{AtomicU64, Ordering};

/// Owned by the unique writer; sequence is never allocated by a reader.
#[derive(Default)]
pub(crate) struct Sequence {
    last: u64,
    #[cfg(test)]
    after_allocate: Option<std::sync::Arc<CommitGate>>,
}

impl Sequence {
    pub(crate) fn allocate(&mut self, attempt: &AtomicU64) -> Result<u64, PublishError> {
        let next = self
            .last
            .checked_add(1)
            .ok_or(PublishError::SequenceExhausted)?;
        self.last = next;
        // Subscription fences can observe the attempt before transport commit.
        attempt.store(next, Ordering::Release);
        #[cfg(test)]
        if let Some(gate) = &self.after_allocate {
            gate.allocated.wait();
            gate.resume.wait();
        }
        Ok(next)
    }
    #[cfg(test)]
    pub(crate) fn at(last: u64) -> Self {
        Self {
            last,
            after_allocate: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn pause_after_allocate(&mut self, gate: std::sync::Arc<CommitGate>) {
        self.after_allocate = Some(gate);
    }
}

/// Test-only pause at the real commit sequence allocation boundary. Neither
/// these barriers nor their storage exist in a production publisher path.
#[cfg(test)]
pub(crate) struct CommitGate {
    pub(crate) allocated: std::sync::Barrier,
    pub(crate) resume: std::sync::Barrier,
}
#[cfg(test)]
impl CommitGate {
    pub(crate) fn new() -> Self {
        Self {
            allocated: std::sync::Barrier::new(2),
            resume: std::sync::Barrier::new(2),
        }
    }
}

/// Taken only after all subscriber resources are staged, before adding demand.
/// Release publication attempts precede enqueue, so a late subscriber cannot
/// replay an attempt whose payload has not yet reached the dispatcher.
pub(crate) fn subscription_fence(attempt: &AtomicU64) -> u64 {
    attempt.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_is_checked_and_release_attempt_never_wraps() {
        let mut sequence = Sequence::at(u64::MAX - 1);
        let attempt = AtomicU64::new(0);
        assert_eq!(sequence.allocate(&attempt), Ok(u64::MAX));
        assert_eq!(attempt.load(Ordering::Acquire), u64::MAX);
        assert_eq!(
            sequence.allocate(&attempt),
            Err(PublishError::SequenceExhausted)
        );
        assert_eq!(attempt.load(Ordering::Acquire), u64::MAX);
    }
}
