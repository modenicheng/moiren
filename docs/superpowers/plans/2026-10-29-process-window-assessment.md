# Finite Process Replay Assessment Implementation Plan

**Goal:** Make a completed finite replay useful for explicit candidate acceptance
without claiming sustained stability, measured sample age or physical latency.

**Architecture:** Keep the actual bridge/replay and strict imported provenance.
Read existing candidate queue-history chunks on the offline control path to
retain exact native retirement/discard accounting. No new realtime recorder,
audio capacity, default tuning or automatic profile application is needed.
Expose a typed finite-window assessment and an opt-in CLI gate; raw results
remain available on rejection and all reports remain `qualified: false`.

**Alternatives:** Automatic profile promotion would confuse finite synthetic
queue evidence with future-safe admission; shrinking buffers first would skip
the existing demand/reserve contract. Neither is part of this approved S1-S3
continuation. The selected bounded assessment supports the requested admission
and validation workflow while hardware timing remains deferred.

## Constraints

- No native audio execution, VM/deployment, driver build/install or production
  buffer/default changes. Preserve correctness/strict policy and 0..3000 ms grace.
- Do not replace actual bridge behavior with nominal frame subtraction.
- Count native input retirement/discards separately from output-rate demand.
- Separate allowed initial startup loss from later starvation/reprime loss.
- Explicit policy; no hidden allowance, future extrapolation or physical claim.
- Preserve unrelated gate.log. Atomic commits and prompt remote-SHA verification.

## Work

- [x] Tests first: candidate reset-held losses, whole finite input conservation,
  event-boundary inventory extrema, producer-only tail, 48/96 kHz conversion,
  startup/never-started/dense demand/trim/reset cases.
- [x] Replay accumulates existing queue-history chunks every 256 reads, never
  loses an overwritten prefix, and checks final native inventory conservation.
  Publish candidate reset discards plus explicit inventory/sample boundaries.
- [x] Typed assessment distinguishes observed violation, inconclusive and no
  violation in this finite window. Policy explicitly limits startup unserved
  output and initial prime-discarded native input, and requires minimum output
  frames, output read count (at least two) and output span. Later unserved,
  drops, backlog/age/reset discards or resets violate the strict window policy.
- [x] Opt-in CLI `--window-policy FILE` loads explicit strict policy, retains
  per-candidate assessments and exits 1 when any candidate is refused, violates
  or is inconclusive; absent policy preserves existing sweep semantics.
- [x] Focused/full audio/probe/RT/app tests, strict Clippy/fmt, Linux cross-check,
  actual offline binary, independent review, evidence/ledger and commit/push.

The full S1-S3 goal stays active. A finite queue assessment cannot complete
sample-age provenance, matched physical measurements or sustained hardware
qualification. No success criterion is weakened to avoid the unavailable VM.

## Completion receipt

Core `5a0a76c` and CLI `ff0dd5c` are individually pushed and remote-SHA verified.
Fresh results: replay34, audio543 (2 ignored), probe298, app464 (3 ignored),
RT142, bridge31/packet8/inventory12/service6. Strict audio/app/RT Clippy,
root/scoped fmt and warning-free Linux cross-check pass. Actual offline binary
checks acceptance/violation/inconclusive/refusal exits and output protection.
Independent core/CLI reviews have no Important/Critical findings. Evidence:
`docs/validation/2026-10-29-process-window-assessment.md`.
