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

- [ ] Add failing serde tests for valid explicit caps and invalid range/type.
- [ ] Add optional persisted setting, portable validation and native propagation.
- [ ] Test default eight periods, exact cap, small target/cap admission and full
  physical wake refusal; cover both actual precision factories and frozen Apply.
- [ ] Run focused service/preferences checks; commit and push exact paths.

## 2. Expose In Existing Settings

- [ ] Add failing independent rebase/conflict and actual UI Save/restart tests.
- [ ] Add Compatible (8) and 1..8 choices for VAD, including Compatible input.
- [ ] Include requested/active cap in summaries and English/Chinese copy.
- [ ] Preserve explicit Save/restart and Process evidence invalidation semantics.
- [ ] Run UI/headless integration checks; commit and push exact paths.

## 3. Review And Verify

- [ ] Independent scoped review, address reproducible findings.
- [ ] Run app/audio affected suites, strict Clippy, fmt and diff checks.
- [ ] Record precise evidence and deferred physical gates in validation ledger.
- [ ] Commit validation record; push and compare full local/remote SHA.
