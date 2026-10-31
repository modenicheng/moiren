# Pinned Process Prefix Collection Implementation Plan

**Goal:** Retain bounded Process histories from the actual managed session
through teardown, rather than losing the only authenticated trace at Stop.

**Architecture:** Continue the authorized S1-S3 work. Wrap the existing
control-side sequence-one archive in an opaque collector created only by an
exact retained native observation handle. Poll during the original session,
retain its observers through retirement, and read final closure after joins.
Public artifacts stay caller-supplied; only opaque retained collectors produce
native-origin traces. Collection never qualifies latency or stops audio.

**Alternatives:** Rebuilding from editable snapshot DTOs would invent trusted
provenance. Enlarging realtime history rings would add RT/storage scope and
still lose arbitrary histories. Reuse the fixed native rings and refuse
missed/capped prefixes instead. VM timing is not part of this work.

## Constraints

- Preserve no-allocation/no-blocking audio paths, defaults and whole-wake
  admission. Allocate/archive only on control or cleanup workers.
- Archive limit remains 8192 events per writer, native rings remain 256.
- Identity/source/rate/demand come from the retained source, not caller DTOs.
  Registry clear, native recovery or a new owner cannot redirect a collector.
- Writer closure is not cleanup success or future-safe admission. App evidence
  retains app generation, document, graph/settings and actual cleanup outcome.
- Bound app collection count/storage; no unbounded per-node archival or
  full archive cloning on every UI snapshot. Refusal is diagnostics only.
- No native audio, VM, driver build/install, deployment, new dependencies or
  production buffer enlargement. Preserve unrelated gate.log.

## Tasks

- [x] Add failing actual-bridge regressions for an opaque retained collector:
  more than 256 events, early poll, late post-clear writes, writer closure,
  identity/generation/node refusal, registry replacement and caller provenance.
- [x] Implement the narrow retained-handle collector API and bounded archive
  forwarding; verify missed poll/cap refusal, no replay while writers live.
- [x] Integrate bounded managed-session polling and post-join finalization;
  retain immutable evidence separately from active observation projections,
  fence settings/revisions/recovery and verify software-only lifecycle tests.
- [x] Run relevant app/audio/probe/RT tests, strict Clippy/fmt and Linux RT
  cross-check; review scope, commit/push exact paths and verify remote SHA.
- [x] Record verification and remaining finite-window/apply/hardware gates in
  the S1-S3 ledger; do not mark the full goal complete.

## File Responsibilities

- `crates/moiren-windows-audio/src/engine_backend/native_observations/process_trace.rs`:
  opaque pinned-source archive adapter; adjacent focused tests use actual bridge.
- `crates/moiren-windows-audio/src/engine_backend/native_observations.rs` and
  `engine_backend.rs`: narrow public construction/reexport, no observer escape.
- `crates/moiren-app/src/service/managed/process_traces.rs`: bounded per-app
  session archive ownership and finalization. Adjacent tests reuse managed rigs.
- `managed/lifecycle.rs`, `managed/driver.rs`, `managed.rs`: lifecycle hooks only.
- Service model/snapshot and diagnostics: compact receipts, not cloned archives.

Primary commands: focused collector tests, full `moiren-windows-audio --lib`,
`moiren-app --lib --features gpui-test`, probe tests with `vad-probe`, RT harness,
strict audio/app all-target Clippy, root/scoped fmt, Linux RT cross-check.

## Completion Receipt

Audio `c02a51b`, app `a4e3776` and test-hardening `975f3b3` are pushed and
remote-SHA verified. Final
host checks: audio547 passed (2 ignored), app484 (3 ignored), probe298,
RT146, UI acceptance13, CLI2/settings6; strict audio/app/RT Clippy, fmt,
diff and warning-free Linux RT cross-check pass. Independent final read-only
reviews find no remaining Important/Critical issue. Full evidence:
`docs/validation/2026-10-29-pinned-process-prefix.md`.

Bounds are explicit: one original collector for an exactly-one-Process host,
8192 events per writer, one Arc-backed predecessor. Managed test seams are
software CallerSupplied; actual native-origin tests are separate. Candidate
window evaluate/apply, source age, lower native periods and physical stable
sub-10-ms qualification remain incomplete. Full S1-S3 is not marked complete.
