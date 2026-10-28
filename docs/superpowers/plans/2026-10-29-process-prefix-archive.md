# Process Prefix Archive Implementation Plan

**Goal:** Retain a bounded, exact sequence-one Process boundary history across
multiple control polls, without enlarging realtime histories or audio buffers.

**Architecture:** A control-only accumulator merges the actual 256-event ring
snapshots. Sequence gaps, conflicting overlap, re-opened writers and exhausted
archive limits latch an explicit refusal without stopping audio collection.
Version 2 diagnostic archives retain metadata and both complete prefixes;
version 1 imports remain supported. Replay validates every event before using
the real bridge, never invents initial state, and remains qualified=false.

## Constraints

- No native audio execution, VM, deployment, driver build/install or new defaults.
- Keep realtime storage at 256 events/writer; archival work stays control-side.
- Archive limit: 8192 events/writer; replay work: 16777216 frames; 65536/event.
- Import/trace export limit: 16 MiB; replay output remains independently 1 MiB.
- Exact sequence-one prefix only; gaps/overlaps/caps cannot become safe admission.
- Closure is unique-writer Drop, not cleanup. JSON remains caller-supplied.
- Preserve gate.log; scoped atomic commits, prompt push and remote SHA verification.

## Interfaces

- `ProcessTraceCollector::new(ProcessIdentity, generation, input_rate,
  consumer_rate, prepared_demand) -> Self` allocates only on the control side.
- `poll(&mut self, &BridgeObserver)` performs one snapshot attempt per writer;
  contention does not substitute stale data. Faults latch and polls stay bounded.
- `artifact(&self) -> ProcessTraceArchive` copies bounded retained diagnostics,
  including final closure and collection refusal. It never fabricates completion.
- `ProcessTraceArchive::trace(&ProcessIdentity, generation)` requires complete,
  closed prefixes and validates canonical writers across all retained events.
- `ProcessTraceDocument` decodes schema 1 or 2 and exposes `trace`, `process`,
  `owner_generation`; imported data never gains retained-native provenance.

## Tasks

- [ ] Test-first accumulator: repeated/overlapping polls, >256 exact events,
  final closure, missed poll, modified overlap, metadata/counter regressions,
  event cap and permanent refusal. Strict bounded archive deserialization.
- [ ] Extend canonical replay validation with bounded complete prefixes while
  preserving version 1's 256-event shape. Verify early edits are rejected even
  when no longer in the realtime suffix; explicit work cap and caller origin.
- [ ] Wire both actual collectors before activation, poll during control loops
  and after joins, export schema 2. Ordinary finite runs must poll even without
  journal/load. Archive refusal records diagnostics, not an early audio stop.
- [ ] Support both input schemas in the actual offline replay CLI; independent
  byte/output bounds and tests, actual executable offline smoke validation.
- [ ] Run affected full software/RT suites, strict lint/fmt, independent review,
  document precise results/remaining physical gates; commit/push atomic slices.

This is finite diagnostic/replay infrastructure, not a 10-minute native result,
sample-age evidence, a stable sub-10-ms result or completion of all S1-S3.
