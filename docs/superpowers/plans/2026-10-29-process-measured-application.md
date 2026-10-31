# Apply measured Process bridges through the actual application

Continues the authorized S1-S3 plan using `executing-plans` and test-first
implementation. Previous goal turn made progress: commits `4642657`/`cc32329`
are present, pushed and recorded; only unrelated `gate.log` is untracked.
No native audio, VM, deployment or driver installation in this software slice.

## Design and contracts

- Add a machine-session-only typed measured selection, separate from persisted
  declared transport settings. Keep source node, stable Process identity,
  originating app session/native owner/graph revision, full evidence and target.
- One explicit `ApplyMeasuredProcessBridge` service request selects evidence
  from its own matching Running snapshot and schedules a replacement owner.
  Require current candidate/active revision, app/native generations and Process
  binding to match. Reject stale/missing/invalid evidence before retiring audio.
- Capture the entire immutable selection with Apply settings. Both precision
  factories pass it to `ProfessionalBackend::with_measured_process_bridge`.
  Confirm selection only with the actual Running owner; a later draft cannot
  mutate a pending replacement. Keep refusal visible without fake active state.
- Normal transport changes and document replacement clear desired measurement
  selection; explicit clear returns to declared/compatible intent on next Apply.
  Never persist evidence as trusted proof or resize a live audio queue.
- Extend actual `monitor` CLI with an explicit bounded measurement phase,
  then apply its selected source's profile and time the requested run only after
  a new matching owner/active configuration confirms. Separate measurement and
  run generations/durations; no automatic production tuning or qualified claim.
- Tests inject observations/backends; no native stream is opened. Keep process
  creation time and distinct graph/app/native generation fences. Preserve all
  cleanup/error checks and a single uninterrupted total startup deadline per
  preparation phase; jitter must not renew deadlines indefinitely.

## Tasks

- [x] Implement typed ephemeral selections, origin summaries and pure validation.
- [x] Wire service capture/replacement, both factories and requested/active state.
- [x] Add real monitor parsing, measured apply state machine and CLI regressions.
- [x] Review stale identity/generation/revision, refusal, cleanup and precision;
  run affected suites, strict Clippy/fmt, record evidence, commit/push explicit
  paths and verify remote hashes. Leave full S1-S3 objective active.

Service commit `380e249` and CLI commit `356669e` are individually pushed with
matching remote hashes. Final app library: 449 passed, 3 ignored; both real CLI
integration tests, strict all-target Clippy/check and formatting pass. Failed
initial cleanup recovery and monitor boundary attempts remain in the evidence
record, not relabeled PASS. See
`docs/validation/2026-10-29-process-measured-application.md`.

Finite-window queue-feasibility replay and physical route/stability qualification
remain open. This application path must not present per-wake admission as proof
of continuous output or a sub-10-ms result.
