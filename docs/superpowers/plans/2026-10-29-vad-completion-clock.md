# VAD Completion Clock Observation Plan

**Goal:** Preserve the meaning and measured service delay of actual VAD PCM
packet stamps without mislabeling them as sample age or end-to-end latency.

**Architecture:** Preparation obtains the system QPC frequency once. At each
actual completion, the worker observes raw system QPC once and compares it
with the completion's existing raw stamp. Playback stamps originate at the
driver transfer callback; microphone completions echo the host submission
stamp. Independent fixed atomic summaries retain valid observations, missing
clocks, future stamps and min/max conservative floor/ceil 100 ns deltas.
No PCM metadata, kernel ABI or production buffer changes are necessary.

## Constraints

- No native audio execution, VM, deployment, driver build/install or tuning.
- All clock reads happen per completed packet, not per sample or Engine node.
- No hot-path allocation/deallocation, locks, retries, logging or clock-frequency
  queries. Summary publication uses fixed atomic operations only.
- Never infer sample timestamps, oldest FIFO age, physical latency or source
  continuity from transfer or submission stamps. Unknown remains unknown.
- Stale/dropped completions remain separately counted; quality diagnostics
  must not turn correctness mode into an early-stop timing gate.
- Preserve unrelated gate.log; focused commits, prompt push and SHA checks.

## Work

- [x] Test-first portable fixed completion-clock summary: direction/provenance,
  exact large ticks, delta-before-conversion rounding, missing/zero/future stamps,
  arithmetic limits, zero observations and saturation; allocation guard.
- [x] Prepare raw QPC access once and wire actual playback, microphone and
  intentionally dropped microphone completions. Preserve existing packet order,
  reset, cancellation and pumping semantics. Mock regressions, no native stream.
- [x] Expose separate playback transfer-service and microphone submission-service
  values through the existing telemetry DTO and app diagnostics, with explicit
  unknown source age and live-summary consistency limitations.
- [x] Full affected audio/app/probe/RT tests, strict Clippy/fmt and Linux
  cross-check; independent review, evidence ledger, atomic commit/push.

This is S1 diagnostic progress. It does not close physical S1 measurements,
VAD sample-age propagation or sustained sub-10-ms qualification.

Implementation receipts: audio `592a4e4`, app `f6ac8c4`, both pushed and
remote-SHA verified. Evidence and remaining gates are recorded in
`docs/validation/2026-10-29-vad-completion-clock.md`.
