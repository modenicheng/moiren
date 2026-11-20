# Probe whole-wake Process admission implementation plan

> For agentic workers: execute this existing S1-S3 invariant repair with
> failing-then-passing tests, focused review, and scoped local commits.

**Goal:** Make both real E2E Process collection paths admit the complete
native render wake before preparing capture, instead of treating the Engine
segment size as the consumption burst.

**Architecture:** Native render preparation publishes its actual immutable
timing and admitted render queue while activation remains gated. A portable
preparation helper rebinds Process options to the queue target at the probe's
fixed 48 kHz rate. Both collection paths retain this admitted bound in their
trace metadata, even though individual ClockSource reads remain segmented.

**Tech stack:** Rust 2024, existing WASAPI owner and ProcessBridgeOptions,
hardware-free preparation and orchestration regressions.

## Existing requirement and alternatives

The existing S3 contract requires consumer-demand admission and expressly
forbids using another endpoint's period as Process delivery timing. Existing
ProcessBridgeOptions documents a complete consumption burst; segmentation
must not reduce reserve. Current probe code instead sets the bound to
engine_block_frames and prepares capture before observing render geometry.
Production ProfessionalBackend already observes its whole native wake.

1. Selected: prepare the gated native renderer, admit its complete queue target,
   then prepare capture and attach the graph. This uses the actual owner demand
   and preserves independently declared/observed Process delivery bounds.
2. Rejected: multiply Engine segments by a guessed count or use render period
   as capture delivery. Neither proves startup/whole-wake consumption.
3. Rejected: reduce render queues/defaults to make small input targets pass.
   This changes the experiment and hides rather than repairs admission.

## Global constraints

- No native audio, VM access, driver install/deployment, or production defaults.
- Preserve unrelated gate.log. Local atomic commits; batch pushes only.
- Keep no allocation/deallocation/blocking/per-sample-clock RT constraints.
- Prepare all budgets before activation. Refuse invalid requests without
  fallback or target/capacity inflation.
- Preserve full capture/render partial reports and cancellation-before-join
  ordering. Disconnect a parked renderer before joining its thread.
- Native queue target is consumption demand, not Process packet cadence,
  sample age, source clock, or physical-latency evidence.
- Passing tests does not finish hardware-dependent S1-S3 requirements.

## Work

### 1. Portable admission rule and native evidence

- [x] Add tests for an input target admitted by Engine64 but refused by an
  actual render512 wake, FullCapacity versus Target, exact accepted bounds,
  invalid geometry/rates, policy preservation, and segmentation independence.
- [x] Run RED with the old segment-only behavior. Add a small control-only
  helper that validates actual native queue geometry and calls for_consumer
  with the full target at 48 kHz. Keep delivery bounds unchanged.
- [x] NativeRenderInfo carries actual PreparedRenderQueue from native
  preparation, separate from the requested render policy and native timing.

### 2. Actual collection orchestration

- [x] In process_collection/graph.rs prepare render first, retain it before
  fallible admission/capture, then bind trace demand to the admitted options.
- [x] Apply the same rule to ordinary native::run in live.rs, retaining partial
  render/capture evidence on refusal. Do not leave an unattached renderer
  blocked on receive while joining it.
- [x] Add hardware-free seams that execute actual post-render admission/capture
  ownership; refusal never prepares capture or activates owners. Source-review
  the remaining real attach/activation continuation. Cancellation and failure
  reports remain visible. Native activation is not executed by these tests.
- [x] Validate multi-segment native output cannot lower archive prepared demand.
- [x] Resolve review's legacy capture-join report loss with a retaining API and
  after-ready panic regressions, preserving the legacy error semantics.

### 3. Verification and delivery

- [x] Run focused and complete E2E tests, strict normal/test-profile example
  Clippy, root fmt/diff, and Linux normal/test-profile cross-checks.
- [x] Independently review both actual callers and lifecycle failure paths.
- [x] Record exact evidence and remaining S1/S2/S3 target gates. Commit coherent
  code and docs locally; no per-commit push.

## Verified checkpoint

Final host checks: audio831 passed / 3 ignored, example367 passed, strict audio
all-target and example test-profile Clippy, root fmt/diff, Linux normal/test
cross-checks (existing cfg dead-code warnings, not Linux execution). Independent
review has no remaining concrete source finding within this slice. See
`docs/validation/2026-10-29-probe-whole-wake-admission.md` for exact RED/GREEN
logs, scope limits and remaining S1-S3 work. No VM/native/driver/deployment/push.
Local source checkpoints: `a2a275a` (retaining capture join) and `2b09537`
(whole-wake probe integration); documentation is a separate local checkpoint.
