//! Shared integration-test scaffolding for moiren-signal.
//!
//! Every `tests/*.rs` binary compiles its own copy via `mod common;`.
//!
//! The unit-test side keeps a parallel copy of `pump_until_settled` in
//! `src/dispatcher/tests.rs`: integration tests cannot import `#[cfg(test)]`
//! items from the library, and each side needs exactly one shared definition.

use moiren_signal::{Bus, PumpBudget};

/// Maximum pump passes before a non-settling bus is reported as a defect.
const SETTLE_PASSES: usize = 1000;

/// Pumps `bus` with `budget` until a pass reports no pending work.
///
/// Panics after [`SETTLE_PASSES`] passes so a runaway dispatch loop fails the
/// test instead of hanging it. This is the settled semantics for all signal
/// tests: bounded iteration plus an over-limit panic (never a silent stop).
pub fn pump_until_settled(bus: &mut Bus, budget: PumpBudget) {
    for _ in 0..SETTLE_PASSES {
        if !bus.pump(budget).has_pending {
            return;
        }
    }
    panic!("pump did not settle within {SETTLE_PASSES} passes");
}
