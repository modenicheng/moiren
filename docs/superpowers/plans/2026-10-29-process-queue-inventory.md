# Consumer-owned capture queue inventory implementation plan

Continues the authorized S1-S3 objective. No native audio, VM timing run,
deployment, driver installation or production-default change in this slice.
Use test-first implementation and independent read-only review.

## Evidence boundary and design

Existing producer events are completed drains, not packet publication times;
existing consumer events are Engine demand, not native input removal. Neither
establishes entry inventory or alternate-target queue feasibility. Do not
reinterpret these as an actual FIFO replay or reject configurations from a
synthetic initial fill.

Instead record a dedicated consumer-owned history at every nonzero ClockSource
read. Its boundary contains cumulative successful ring removals D, acquired ring
occupancy q and interpolation-held frames h. The causal publication frontier is
P=D+q; retired frontier is R=D-h; pending inventory is P-R=q+h. Post-read is a
later boundary, not a second sample at the pre-read timestamp. Same consumer
owns D/h; acquired ring occupancy does not combine independently sampled
producer counters. Track priming/backlog/age discards, held frames discarded by
reset, reset count and underrun separately. A retained next frame is not lost.

Keep 256 single-writer scalar atomic events, one-attempt coherent control reads,
explicit unknown clocks/overwrites and sticky checked-arithmetic exhaustion.
No new lock, retry loop, clock read per sample, allocation or destruction on RT.
Account for history storage in capture preparation and total backend budgeting.
Expose observations through BridgeObserver and generation-pinned Process/native
snapshots. Do not require diagnostic history for the existing measured per-wake
API or claim target replay, source age, sustainability or physical latency.

## Tasks

- [x] Add pure queue boundary/history API with failing conservation, overwrite,
  clock/overflow/coherence tests; implement and verify with fixed storage.
- [x] Add real bridge regressions for prime, normal interpolation, both rate
  directions, overflow/discontinuity/reset and all recovery/discard paths;
  count all ring removals and publish pre/post read evidence without changing
  audio behavior. Include no allocation/deallocation assertions.
- [x] Expose history under immutable native source/owner identity; test final
  retained handle updates after registry clear and new generation isolation.
  Include queue storage in actual preparation budgets and test refusal bounds.
- [x] Preserve exact measured desired/active desktop identity through settings
  display, save rebasing, transport clearing and restart confirmation. Include
  test-first regressions and independent read-only review.
- [x] Verify affected audio, integration, probe, RT and application surfaces;
  strict Clippy/fmt, review, record exact failures/results and open gates,
  atomic commits, push and verify remote SHA. Preserve unrelated gate.log.

Implementation commits `7c2f3a0` and `343c8d5` were independently pushed and
remote-hash verified. Full results, first failing runs and remaining gates are
in [`2026-10-29-process-queue-inventory.md`](../../validation/2026-10-29-process-queue-inventory.md).
All software checks are hardware-free; Linux checking is cross-compilation,
not test execution or CI. The docs commit records this completed bounded slice,
not completion of the full S1-S3 objective.

The full objective stays active. Matched physical E2E, actual lower native
periods, sample-age proof and alternate-target finite-window feasibility remain
unproven; this ledger supplies the missing real inventory evidence first.
