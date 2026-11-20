# Ordered Process Trace Replay Implementation Plan

**Goal:** Run admitted target candidates through the real capture bridge over
a finite observed packet/request trace, without inventing starting inventory.

**Architecture:** Closed unique-writer packet/read histories provide bounded
complete seq1 traces. Validation reconstructs canonical history bookkeeping,
requires strictly nonoverlapping host envelopes and checks every baseline
consumer publication frontier against accepted packet prefixes. A new empty
bridge executes full offered packets and full requested reads in that unique
order. Candidate servo/prime/trim behavior is real, not baseline retirement.

**Tech Stack:** Pure Rust control-side replay, existing capture bridge,
preparation admission and bounded histories. No native device or VM.

## Contract

- Writer-finished means that history's unique writer is destroyed, not transport
  cleanup success. `running=false` cannot substitute for this closure evidence.
- Whole offered supply at each recorded order position is an explicit replay
  assumption when baseline accepted only a prefix. Baseline loss stays visible.
- The result is `Replayed`, never Safe/Admitted/Qualified. It reports startup,
  unserved requests, overflow, discards, resets and terminal inventory. It is
  not a prediction of counterfactual execution duration or future stability.
- Unknown clocks, truncated histories, ambiguous/tied envelopes, malformed
  conservation, mixed origin, wrong identity/rates/generation and unadmitted
  candidate options are refused. No invented prefix fill or unknown timestamps.
- All allocation/work occurs off RT with independent work/storage limits.

## Tasks

- [x] Add test-first writer-finished markers to packet/read histories, driven
  only by unique-writer Drop and included in storage budgeting/snapshots.
- [x] Add `ObservedProcessTrace` validation and a typed finite-window replay
  report. Replay real `ProcessBridgeOptions::clock_config` on all supported
  platforms; do not duplicate configuration conventions.
- [x] Test observed partial/zero supply, startup, small-target starvation,
  independent capacity, overflow/trim, discontinuity, rates, ambiguous order,
  missing/edited evidence, closure and private immutable validated snapshots.
- [x] Expose construction through generation-pinned native Process observation
  with explicit expected process/generation and missing-history refusal.
- [x] Run software validation, independently review, document exact limitations,
  commit/push explicit paths and verify remote SHA. Preserve unrelated gate.log.

Implementation: closure `c544fb1` and replay `1531eff`, each pushed and
remote-hash verified. Public snapshots validate only caller-supplied structure;
retained-native origin comes solely from the pinned read handle. Replay is a
bounded diagnostic, not full S3 admission or qualification. Actual app/CLI
replay selection, long-window feasibility and unknown sample age remain open.
See [`../../validation/2026-10-29-process-packet-publication.md`](../../validation/2026-10-29-process-packet-publication.md)
for exact tests, limitations and the corrected independent-review finding.
