# Desktop Measured Process Selection Implementation Plan

**Goal:** Make the existing validated measured Process replacement operation
usable from the actual desktop, without implying finite-window or hardware
qualification and without persisting measurement evidence.

**Architecture:** Continue the authorized S1-S3 application plan. Reuse existing
Input Buffers controls, exact source/session/native/revision identities and
`ApplyMeasuredProcessBridge`. A session-local three-value draft is independent
of saved declared packet/gap assumptions. UI preflight improves feedback;
the service remains authoritative and revalidates before replacement. Track
mailbox consumption separately from matching replacement activation.

**Alternatives:** Inferring packet/gap defaults would undermine measured
admission. Automatically applying offline JSON would promote caller-supplied
evidence. Neither is selected. Bounded pinned-prefix collection is a separate
dependency for a later integrated finite-window workflow; this action uses the
already-implemented conservative whole-wake measured budget, not a new claim.

## Constraints

- No native audio, VM, deployment or driver operations. No production defaults
  or live queues change except through explicit user-initiated owner replacement.
- Keep all pending/desired/active origins distinct; never claim enqueue or
  consumption means successful activation. Unknown age stays unsupported.
- Preserve unsaved audio/transport/device drafts; disallow measured Apply while
  conflicting drafts or transactions exist. Do not serialize trusted evidence.
- Invalid/missing/stale observations produce visible reasons without stopping
  the existing owner. Service errors remain visible and editable values survive.
- Reuse established GPUI components/layout; no redesign or new dependencies.
- Atomic scoped commits, prompt push plus remote-SHA verification; keep gate.log.

## Tasks

- [x] Test-first session-local draft/preflight: exact Process identity and graph,
  source node, session/native generations, valid three-value target; refusal for
  missing/stale source/evidence and structurally insufficient target.
- [x] Render a measured subsection in Input Buffers with source selection,
  independent capacity/target/jitter, explicit replacement action and warning;
  render desired and active summaries separately.
- [x] Track one captured request through consumption and owner confirmation;
  forbid duplicate Apply after consumption, reject unrelated confirmation,
  handle stop/failure/stale origin and expose refusal without fallback.
- [x] Real GPUI click tests log exact request, held confirmation/duplicate guard,
  requested-versus-active state and refusal feedback; portable tests where possible.
- [x] Independently review, run app/audio/probe/RT relevant tests and strict
  Clippy/fmt, record remaining hardware/finite-window gates, commit/push/verify.

Primary files: `crates/moiren-app/src/ui/window/settings/measured.rs`, adjacent
focused model/GPUI tests, `settings/model.rs`, `settings/transport.rs`, and the
shell render completion hook. Existing service regression tests continue to
verify both precision factories and true owner replacement.

Commands: `cargo test --locked -p moiren-app --lib --features gpui-test`,
`cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings`,
`cargo fmt --all -- --check`. Tests do not open a native device.

## Completion receipt

Implemented in `68e6b549cff02e5d9e46027b6b3fb57ab612f430`, pushed and
remote-SHA verified. App 476 passed (3 ignored), final UI acceptance 13,
focused measured clicks 6, CLI 2, transport settings 6, audio 543 (2 ignored),
probe 298 and RT harness 142 passed. Strict app Clippy, app all-target check,
Linux RT cross-check, fmt and diff checks pass. Initial acceptance clipping
and two review-discovered UI defects have retained failing-then-passing
evidence. Independent final read-only review finds no Important/Critical
issue in scope. Details and limits:
`docs/validation/2026-10-29-desktop-measured-process.md`.

Only this desktop operation is complete. Integrated pinned-prefix/window
workflow, sample-age provenance, sustained admission, actual lower native
periods and physical stable sub-10-ms qualification are not claimed. No VM,
native audio, driver build/install, deployment, remote CI or Miri run occurs.
