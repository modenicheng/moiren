# Live Process window admission implementation plan

> **For agentic workers:** Use test-driven-development and independent review.
> Continue the authorized S3 scope. No native audio, VM, driver or push operations.

**Goal:** Let the application service and monitor CLI explicitly require a
sealed observed supply/demand window before replacing a live Process owner.

**Architecture:** Preserve existing per-wake selection and stopped retained
Apply. A separate request supplies the five explicit finite-window thresholds.
The managed service freezes measured options through its already pinned
original collection and replays them on the bounded Prepare worker. Only a retained-native, no-violation result
can construct the private profile proof. Captured origin/settings/source and
the original measured rate/demand envelope survive replacement.

**Tech Stack:** Existing Rust service, ProcessTraceSessionOrigin,
NativeProcessTraceCollector, MeasuredProcessProfile and ReplayWindowPolicy.
No dependencies, persisted proof, queue resizing or RT instrumentation.

## Global constraints

- Preserve compatibility defaults, per-wake selection, stopped Apply and buffers.
- Service request accepts policy and target, never an editable report/token.
- Five explicit thresholds; positive frames/span and at least two output reads.
- A software fixture may exercise control flow but cannot certify native IO.
- Missing, capped, stale, ambiguous or violating evidence refuses before retirement.
- Bind document, session/native generations, process, graph/host revisions,
  audio/transport/profile settings, source and admitted rate/whole-wake demand.
- No checkpoint claim beyond8192 events per writer, freshness composition,
  physical timing or future stability claim.
- Local scoped commits; preserve unrelated gate.log; batch pushes later.

## Task 1: Service admission and proof

Files: `service/measured_process.rs`, new `service/measured_process/live.rs`,
`service/managed/measured.rs`, `service/managed/process_traces.rs`,
`service/managed/lifecycle.rs`, `service/managed/driver.rs`, model, mailbox and
fallback dispatcher, with regression fixtures beside managed measured tests.

Interfaces:

- `ProcessWindowPolicy`: five u64 fields matching ReplayWindowPolicy,
  portable validation and Windows-only conversion.
- `AppRequest::ApplyWindowedProcessBridge { request, policy }`.
- `Collection::prepare_live_window(input, policy)`: private trusted native
  adapter returning bounded worker evaluation; default unavailable otherwise.
- Private profile proof retains origin and exact report. Public summary exposes
  policy, cutoff and replayed packet/read counts; it has no constructor authority.
- `begin_windowed_process` / `finish_windowed_process`: settled live request,
  actual pinned collection preparation, worker replay, pre-publication origin
  and private instance rechecks, then private proof issuance.
- Lifecycle validation rejects changed origin/settings/source before retiring
  the predecessor; captured settings are checked again before opening.
- Backend uses `with_evaluated_process_bridge` to forbid larger demand or
  different rate than replayed, preserving the existing preparation check.

- [x] Add real bridge-driven RED cases for positive opt-in, repeated starvation,
  insufficient exposure, caller origin, stale identity/settings and changed graph.
- [x] Implement portable policy and service private binding/selection.
- [x] Verify previous desired/active owner survives each refusal, accepted
  settings reach both precision factories, and invalid/missing policy is visible.

## Task 2: Explicit monitor CLI

Files: `cli.rs`, `cli/options.rs`, `cli/monitor.rs`,
`cli/monitor/measured.rs` and CLI regression files.

- [x] Parse all five `--window-*` thresholds together with measured mode.
- [x] Reject incomplete/invalid policy or use without `--measure-seconds`;
  preserve per-wake omission behavior.
- [x] Dispatch windowed request and require exact active policy confirmation.
- [x] Preserve requested policy in JSON and explain finite-prefix/cap limits.

## Task 3: Verification and delivery

- [x] Run focused new tests and full app regressions; audio regressions if changed.
- [x] Run strict relevant Clippy, workspace fmt/diff and Linux compile when available.
- [x] Independent source/provenance and compatibility review; fix real findings.
- [x] Commit coherent source atomically, then receipt/plan/ledger documentation.

Sources `88853ab` and `75481a8`; see the
[receipt](../../validation/2026-10-29-windowed-process-admission.md) for final
checks, earlier failures, private test-fixture limits and honest256-event native
measurement-history refusal. The8192 replay cap is not the sole admission cap.

Hardware acceptance, changing acquired PCM/common reference, checkpointed long
windows, desktop controls, freshness composition and physical sub-10-ms
qualification remain separate gates.
