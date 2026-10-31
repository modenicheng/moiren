//! Unit tests for the deferred-job intake, folded in from the integration
//! suite so the test-only `try_defer`/`pending_jobs` surface can stay
//! `#[cfg(test)]` (the `settings_tests` split is the earlier precedent).

use super::*;

struct DropProbe(mpsc::Sender<thread::ThreadId>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.send(thread::current().id()).unwrap();
    }
}

#[test]
fn deferred_jobs_have_fixed_capacity_and_preserve_refusals() {
    let mut c = ServiceContext::default();
    for _ in 0..MAX_PENDING_JOBS {
        c.try_defer(JobPriority::Cleanup, Box::new(|| {}))
            .unwrap_or_else(|_| panic!("space"));
    }
    let (tx, rx) = mpsc::channel();
    let probe = DropProbe(tx);
    let (_, job) = c
        .try_defer(JobPriority::Cleanup, Box::new(move || drop(probe)))
        .err()
        .unwrap();
    assert!(rx.try_recv().is_err());
    thread::spawn(job).join().unwrap();
    assert_ne!(rx.recv().unwrap(), thread::current().id());
}

#[test]
fn reserved_cleanup_survives_full_service_storage() {
    let mut c = ServiceContext::default();
    let reserved = c.try_reserve_cleanup().unwrap();
    for _ in 0..15 {
        c.try_defer(JobPriority::Prepare, Box::new(|| {}))
            .unwrap_or_else(|_| panic!("space"));
    }
    assert!(matches!(c.try_reserve_cleanup(), Err(DispatchError::Busy)));
    c.defer_reserved_cleanup(reserved, Box::new(|| {}))
        .unwrap_or_else(|_| panic!("reserved space"));
    assert_eq!(c.pending_jobs(), 16);
}
