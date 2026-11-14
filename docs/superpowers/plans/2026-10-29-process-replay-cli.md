# Process Replay CLI Implementation Plan

**Goal:** Make bounded Process queue replay usable from the real probe CLI and
export the actual joined collectors' packet/request observations.

**Architecture:** A versioned serialized diagnostic artifact carries process
identity, per-owner generation, rates, prepared demand and both closed bounded
histories. Imported artifacts are always caller-supplied structure, never
native provenance. The offline probe validates explicit expected identity and
generation and evaluates a bounded explicit candidate list with per-candidate
refusals. Real collectors export their actual final histories after joins;
incomplete/truncated evidence remains exportable but is refused by replay.

**Tech Stack:** Existing Rust/serde, real capture bridge replay and probe CLI.

## Constraints

- No native audio run, VM, deployment, driver build/install or production-default
  changes. Preserve correctness versus latency-sensitive policy and gate.log.
- Imported JSON never promotes to retained-native origin or qualification.
- Strict finite limits: 256 events/writer, 16 candidates, 1 MiB/file, checked
  replay work limits and actual bridge storage admission. Never silent fallback.
- No invented starting inventory, times, successful cleanup or hardware result.
- Pure software and existing COM mocks only; scoped commits, prompt push/SHA check.

## Tasks

- [x] Test-first versioned `ProcessTraceArtifact` serialization and import
  validation in `process_loopback/replay/artifact.rs` with mandatory schema,
  bounded histories and explicit expected identity/generation. Add only required
  serde derives to scalar observation types, preserving existing producer logic.
- [x] Test-first `replay-process` command in the actual `e2e_latency_probe`
  CLI: explicit trace/candidate/identity inputs, bounded new-file output,
  per-candidate queue reports/refusals, caller-supplied origin, qualified=false,
  strict parsing/input caps and no stream/device discovery.
- [x] Export final real Process collectors' trace metadata and packet/read
  histories after worker joins, with completeness/closure separate from cleanup.
  Retain incomplete diagnostics, no successful trace invented on prepare failure.
- [x] Run focused/full audio/probe/app/RT tests, strict affected lint/check/fmt,
  actual CLI integration without audio, independent read-only review, document
  exact failures/results/open sustained-window limits, commit/push each slice
  and verify remote SHA. Full S1-S3 objective remains active.

Implementation receipts: `43ea5eb`, `b24fe8b`, `400def8`, `8faa0e7`, `93143b9`,
each pushed and remote-hash verified. Exact commands, red/green failures,
actual-binary receipts and sustained-window/physical gaps:
[`../../validation/2026-10-29-process-replay-cli.md`](../../validation/2026-10-29-process-replay-cli.md).
