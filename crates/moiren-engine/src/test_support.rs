//! Support shared by `#[cfg(test)]` modules inside the library.
//!
//! Integration tests keep a parallel copy of `pump` in
//! `tests/common/mod.rs`: they compile against the library build, where
//! `#[cfg(test)]` items are not visible.

use moiren_signal::{Bus, PumpBudget};

/// One bounded pump pass with the canonical unit-test budget (the
/// spectrum fixture keeps a larger `copy_bytes` for its wide blocks).
pub(crate) fn pump(bus: &mut Bus) {
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 128,
        copy_bytes: 65_536,
    });
}
