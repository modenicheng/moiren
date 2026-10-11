use crate::PublishError;
use std::sync::atomic::{AtomicU64, Ordering};

/// Owned by the unique writer; sequence is never allocated by a reader.
#[derive(Default)]
pub(crate) struct Sequence(u64);

impl Sequence {
    pub(crate) fn allocate(&mut self, attempt: &AtomicU64) -> Result<u64, PublishError> {
        let next = self
            .0
            .checked_add(1)
            .ok_or(PublishError::SequenceExhausted)?;
        self.0 = next;
        // Subscription fences can observe the attempt before transport commit.
        attempt.store(next, Ordering::Release);
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_is_checked_and_release_attempt_never_wraps() {
        let mut sequence = Sequence(u64::MAX - 1);
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
