# App VAD Recovery Cap Implementation Plan

**Goal:** Complete the existing S2 application integration for bounded virtual
clock recovery without changing compatibility defaults or physical wake demand.

**Architecture:** Persist an independent optional `vad_recovery_cap_periods`
alongside Process/VAD input budgets. `None` preserves eight periods; explicit
values must be 1 through 8 and pass unchanged to the existing native owner.
Use the current settings draft, rebase, save and explicit owner-restart workflow.
The cap limits scheduling catch-up, not normal residency or measured latency.

**Constraints:** No VM/native driver operations; no changes to production
defaults, whole-wake admission, RT allocation, or jitter-test policy. Preserve
unrelated `gate.log`. Software coverage does not qualify physical sub-10 ms IO.

## 1. Persist And Admit The Independent Cap

- [x] Add failing serde tests for valid explicit caps and invalid range/type.
- [x] Add optional persisted setting, portable validation and native propagation.
- [x] Test default eight periods, exact cap, small target/cap admission and full
  physical wake refusal; cover both injected precision factories and frozen Apply.
  Inspect both production Windows factories separately, without opening devices.
- [x] Run focused service/preferences checks; commit and push exact paths.

## 2. Expose In Existing Settings

- [x] Add independent rebase/conflict and failing actual UI Save/restart tests.
- [x] Add Compatible (8) and 1..8 choices for VAD, including Compatible input.
- [x] Include requested/active cap in summaries and English/Chinese copy.
- [x] Preserve explicit Save/restart and Process evidence invalidation semantics.
- [x] Run UI/headless integration checks; commit and push exact paths.

## 3. Review And Verify

Scope refinement after independent audit: the actual VAD probe also drops the
cap at its adapter. Close this separate entry-point gap before final validation.

- [x] Test `--vad-recovery-cap-periods` parsing, 1..8/no-clamp bounds, duplicate
  rejection and Process-route refusal; retain requested options in dry manifest.
- [x] Forward exactly to the VAD backend, retaining full physical wake admission.
- [x] Update both root and live-vad help; no live invocation or default change.

- [x] Independent scoped review, address reproducible findings.
- [x] Run app/audio affected suites, strict Clippy, fmt and diff checks.
- [x] Record precise evidence and deferred physical gates in validation ledger.
- [x] Commit validation record; push and compare full local/remote SHA.

Software scope is complete; full S1-S3 remains active. Next independent work is
bounded drift-aware offline marker analysis, not repeated VM timing attempts.
