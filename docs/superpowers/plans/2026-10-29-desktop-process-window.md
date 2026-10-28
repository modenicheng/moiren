# Desktop Retained Process Window Implementation Plan

**Goal:** Evaluate the actual last closed Process prefix in Settings without
requiring VM timing qualification or changing production admission.

**Architecture:** Continue the authorized S1-S3 work with a diagnostic-only
desktop slice. Evaluate immutable service-retained evidence on one background
worker using independent capacity/target/jitter and five explicit finite-window
thresholds. The replay uses conservative declared bounds derived from the closed
prefix, its original consumer rate and whole prepared wake demand. Display
provenance, assumptions, exposure, violations and an always-unqualified result.
Existing measured Apply remains a separate live whole-wake admission/replacement.

**Alternatives:** Reusing a retirement snapshot for production admission can
miss late delivery and confuse closed predecessor with current native evidence.
Promoting a successful segmented replay to production admission weakens the
whole-wake contract. Neither is part of this diagnostic implementation.

## Constraints

- No VM/native audio/driver/deployment operations or production default changes.
- No new dependencies, RT allocation/blocking, or enlarged audio/history rings.
- Never import or deserialize trusted native origin; preserve trace provenance.
- One retained predecessor, one in-flight evaluation; no replay during render.
- Fence results on exact retained Arc, candidate and policy; stale work cannot
  overwrite current feedback. Draft changes invalidate the displayed result.
- A window with no observed violation is not sustained service, age, waveform,
  physical latency or authorization to Apply. Do not add fallback zero values.
- Preserve unrelated untracked gate.log. Commit explicit paths, push, verify SHA.

## Tasks

- [ ] Add service candidate regressions using actual software bridge histories:
  exact independent options and conservative declared gap, original rate/wake,
  refusal/invalid policy, lost output, insufficient exposure and no qualification.
  Run focused app tests to observe RED, then implement the minimal evaluator in
  `crates/moiren-app/src/service/process_trace/evaluation.rs`.
- [ ] Add editor-state tests for one in-flight worker, stale predecessor/draft/
  policy results and retained error feedback. Implement the state in
  `ui/window/settings/measured/window_state.rs` and background dispatch/view in
  `ui/window/settings/measured/window.rs`, reusing existing candidate controls.
- [ ] Add actual Settings click tests for stopped retained evidence and absent
  evidence. Verify evaluation dispatches no audio requests, shows explicit
  policy/status/provenance/assumptions and leaves live measured Apply unchanged.
  Add locale keys using the actual English/Simplified Chinese catalog.
- [ ] Run full app library and relevant CLI/transport/UI integration tests,
  strict all-target app Clippy, fmt and diff checks. Request independent review,
  resolve findings, commit/push scoped changes and verify full remote SHA.
- [ ] Record exact results and remaining production evaluated-apply/physical
  gates in the validation ledger. Full S1-S3 remains active.

## Verification Commands

```powershell
$env:CARGO_TARGET_DIR='D:\coding\moiren\target'
cargo test --locked -p moiren-app --lib process_window --features gpui-test
cargo test --locked -p moiren-app --lib --features gpui-test
cargo test --locked -p moiren-app --test cli --test transport_settings --test ui_acceptance --features gpui-test
cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings
cargo fmt --all -- --check
git diff --check
```
