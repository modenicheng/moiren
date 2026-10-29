# Rate-Aware Offline Marker Analysis

**Goal:** Advance S1 with bounded detection of known changing markers under
explicit sample-rate mismatch, without inventing hardware clock calibration.

**Architecture:** Add a control/offline-only template-rate grid search. The
caller supplies template and per-offset score scratch, explicit frame/work and
hypothesis caps. Templates use zero-extended linear interpolation. All distinct
onsets compete across all rates, including weak candidates; alternative rates
at the same onset do not create a false second onset. Retain direct common
recording-clock offsets and their uncorrected distribution.

**Scope:** Default legacy analysis is unchanged. Optional schedule rate search
expands admitted windows and multiplies admitted total work before reading PCM.
Selected rates are template hypotheses, not hardware skew measurements or
physical uncertainty bounds. Never detrend latency or infer a native epoch.

**Constraints:** No VM/live audio/driver/deployment; preserve unrelated gate.log.
Whole-wake production admission and correctness/strict test policy unchanged.

## Core Detector

- [ ] Add tests for actual resampled long markers at positive/negative rate
  offsets, known delay, rigid-detector refusal, ambiguity across rates, silence,
  clipping/nonfinite input, truncation, invalid grid, exact work/scratch limits.
- [ ] Implement preparation with checked frame/work arithmetic before scanning.
- [ ] Implement search with caller scratch; reject ambiguity/partial markers.
- [ ] Verify and commit/push the core slice with full remote SHA comparison.

## Actual Offline Entry Point

- [ ] Add failing analyze-pcm tests for explicit rate-search schedules, exact
  preserved offsets, legacy compatibility, whole work/window admission.
- [ ] Integrate independent reference/processed detection with expanded bounds,
  requested search config and selected hypothesis/resolution in reports.
- [ ] Retain packet loss/reset invalidation and failing marker details.
- [ ] Update CLI help, verify and commit/push the integration slice.

## Verification

- [ ] Independent review of algorithm, budgets and provenance.
- [ ] Run focused/full audio/probe tests, app dependent checks, strict Clippy,
  root/scoped fmt and diff checks; correct reproducible findings.
- [ ] Record evidence and remaining physical/sustained S1-S3 gaps; commit/push.
