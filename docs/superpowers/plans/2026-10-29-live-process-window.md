# Live Process observed-prefix window implementation plan

> **For agentic workers:** Use test-driven-development and independent review. No native audio, VM, driver or push operations.

**Goal:** Assess candidate queue supply/demand over an immutable, sequence-one prefix while the original native owner remains active, without inventing writer closure or entry inventory.

**Architecture:** Reuse existing ordered production queue replay and finite-window policy. A live archive must be complete from sequence one, with both writers' next retained bracket starts sealing an earlier prefix. Strictly earlier complete envelopes are replayed from the genuine owner-start empty state; ambiguous order, missed/capped evidence or unsupported freshness returns inconclusive. The pinned native collector authenticates the original source association separately from editable archives.

**Tech Stack:** Existing Rust audio crate, ProcessTraceCollector, packet/read observations, production capture_bridge, ReplayWindowPolicy. No dependencies or RT instrumentation changes.

**Spec:** S3 in `docs/vad-low-latency-goals-2026-10-28.md` and remaining observed-window gap in `docs/validation/2026-10-29-s1-s3-software-ledger.md`.

## Global constraints

- Preserve existing closed replay/retained Apply and measured per-wake defaults.
- All work remains off RT; no source timestamps/sample age, hardware result or future scheduling guarantee.
- Public archive labels remain CallerSupplied. Only pinned backend collector may report RetainedNativeOwner.
- Actual writer_finished remains actual. A live prefix never claims both writers closed or transport cleanup.
- At most8192 retained events per writer and existing frame/work limits. Missed/capped/conflicting prefixes are inconclusive; no restart with invented empty/full queue.
- Feasible means no violation of explicit finite-window policy in the replayed prefix. Insufficient duration/output and ambiguous evidence remain inconclusive; observed loss dominates insufficient output.
- Grace and production queue capacities unchanged. Native/physical validation and >cap checkpoint support remain separate.
- Atomic local commits, batch remote checkpoints later. Preserve unrelated gate.log.

## Review focus

- An in-flight final producer/read envelope may start before a later opposite-writer event: exclude unsealed tails using both writer horizons.
- Same-stamp or overlapping cross-writer envelopes and acquired publication frontiers must still refuse.
- Edited or conflicting sentinel evidence must not manufacture a later horizon.
- Open histories must not enter the existing closed-retained Apply API or gain fake closure.
- Native provenance, option identity/rate/demand and freshness guards must survive wrappers.

## Task 1: Portable prefix assessment

Files: `process_loopback/replay.rs`, new `process_loopback/replay/live.rs`, its tests, and reexports in `process_loopback.rs`.

- [x] Add genuine failing tests for an actual open bridge with sealed prefix, withheld trailing read/packet, candidate starvation despite per-wake fit, insufficient/tied/overlapping evidence, altered sentinel metadata, capped/missed history and freshness refusal.
- [x] Extract closed-versus-complete-prefix structural validation without fabricating writer_finished; keep existing closed constructors strict.
- [x] Implement explicit live-prefix report with window status, sealing metadata, optional actual replay/assessment, bounded refusal and qualified=false. Reuse actual candidate bridge replay.
- [x] Verify open histories never satisfy old closed constructor, raw caller provenance unchanged and common prefix stable after tails advance.

## Task 2: Actual pinned native collector API

Files: `engine_backend/native_observations/process_trace.rs` and scoped tests.

- [x] Add native-free actual retained collector tests for original process/generation, registry replacement and full report origin/closure separation.
- [x] Expose live window assessment through the existing pinned collector; no cancellation, resource ownership or fake writer closure. Freshness-enabled original owner refuses rather than dropping intent.
- [x] Run focused tests and check production adapter calls the portable implementation.

## Task 3: Verification and delivery

- [x] Probe and audio regressions, appropriate RT checks, strict Clippy/fmt/diff, Linux compile and portable tests where available.
- [x] Independent scoped review or explicitly record tool unavailability; fix concrete findings with regression evidence.
- [x] Source atomic commit plus receipt/ledger/plan docs commit; no automatic push.

Source: `5df48d03168ba3aed344038d6d2f782babfe0a8c`. See the
[validation receipt](../../validation/2026-10-29-live-process-prefix.md) for
counts, genuine RED failures, the repaired RT shadow compile failure and
platform warning boundaries. Dewey and Newton independently reviewed the
watermark/source and compatibility/provenance boundaries; no scoped finding
remained. Linux checks were compilation only.

## Rulings and completion boundary

- Ruling: implement a bounded live-prefix API before application admission - the existing closed-window replay/Apply is already present and must not be relabeled - app/CLI selection still needs explicit policy and proof binding in a later slice.
- Ruling: return inconclusive at archive cap instead of silently restarting queue state - continuous checkpointed replay needs additional persistent state and provenance - this slice does not claim support beyond8192 events per writer.
- Ruling: seal at the minimum of the latest retained packet/read envelope starts, retaining only envelopes ending strictly before it - each writer's future events follow its own retained completed horizon - overlapping/tied prefix order still refuses.

This is the live observed-prefix core and native API. Full live app/CLI admission, checkpointed >cap windows, freshness-decision provenance, acquired audio and physical sub-10-ms qualification remain open.
