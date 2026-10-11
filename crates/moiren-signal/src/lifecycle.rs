use std::sync::atomic::{AtomicU64, Ordering};

use crate::RegisterError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DirectoryId(u64);

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

impl DirectoryId {
    pub(crate) fn allocate() -> Result<Self, RegisterError> {
        checked_identity(&NEXT_DIRECTORY).map(Self)
    }
}

fn checked_identity(counter: &AtomicU64) -> Result<u64, RegisterError> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .ok_or(RegisterError::GenerationExhausted)?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current),
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_identity_never_wraps() {
        let counter = AtomicU64::new(u64::MAX);
        assert_eq!(
            checked_identity(&counter),
            Err(RegisterError::GenerationExhausted)
        );
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
