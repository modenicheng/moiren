# Segmented Marker Delay Trends

**Goal:** Extend S1 offline diagnostics with relative delay trends without
mistaking VM scheduling gaps, failed markers or packet resets for continuity.

**Architecture:** An optional schedule `trend` requests a bounded, off-thread
linear fit over final accepted common-recorder marker delays. Failed markers
and every invalid packet span between observation windows split segments.
Raw marker delays and their distribution remain unchanged. Fits describe
relative delay change, not hardware clock skew, physical uncertainty or a
latency qualification.

**Constraints:** No VM/native audio/driver/deployment. Production queues,
whole-wake admission and validation/grace policy remain unchanged. Preserve
unrelated `gate.log`. Make scoped local atomic commits and retain this slice
locally; batch the next push at a meaningful integration milestone.

## Task 1: Optional Offline Trend Report

Files: `examples/e2e_latency_probe/pcm.rs`, `pcm/trend.rs`,
`pcm/trend_tests.rs` under `crates/moiren-windows-audio/`.

- [x] Write and run failing real-PCM tests for constant, rising and falling
  delays; legacy field omission; failed middle marker; insufficient count/span.
- [x] Add `trend: Option<TrendConfig>` with `min_markers >= 3` and positive
  `min_span_frames`; reject invalid config before reading PCM. The count cannot
  exceed the schedule's marker cap. Integer deltas and delays must fit the
  explicit exact-f64 integer range (2^53 - 1) before fitting.
- [x] Implement `trend::analyze(results, config, continuity)` with checked
  origin-relative integer differences, centered OLS and descriptive residual
  RMS `sqrt(SSE/n)` / maximum. Emit every nonempty segment, including refusal
  reasons. No outlier rejection, delay correction or qualification promotion.
- [x] Test large absolute origins, explicit precision refusal, nonmonotone
  reference positions, outliers retained and legacy behavior. Trend opts into
  report schemas5/6; schemas1/2/3/4 remain unchanged without trend.

## Task 2: Packet Continuity And Actual Report Entry Points

Files: `pcm/packets.rs`, `pcm/trend_tests.rs`, `pcm/rate_tests.rs`,
`live.rs`, `cli.rs` in the same example.

- [x] Write and run a failing gap-only reset test; extend it with exact
  half-open boundaries, loss/timestamp/flags, spanning faults and coverage
  mismatch regressions. Retain exit codes.
- [x] Extend monotone packet traversal in O(packets + windows), annotating a
  break before each window when an invalid span occurs between windows. Fit
  only after invalidating overlapping marker windows. Clean metadata is only
  diagnostic continuity; no metadata means explicitly unverified continuity.
- [x] Add failing trend-only imported-live test. Preserve full analysis in a
  separate optional `trend_measurement` field; do not duplicate rate-only
  evidence or change legacy field omissions. Document config and caveats in
  CLI help. Test positive/negative global resampling with unchanged raw delays.
- [x] Make scoped local feature/test commits after focused checks.

## Task 3: Review And Verification

- [x] Obtain independent source review; reproduce and fix material findings
  with failing-then-passing regression tests.
- [x] Run full probe tests, audio library tests, strict example Clippy, fmt,
  Linux example cross-check and diff checks; report existing warnings honestly.
- [x] Record exact commands/results and remaining S1-S3 hardware gates in
  `docs/validation/2026-10-29-segmented-marker-trends.md` and rolling ledger.
- [x] Commit validation locally. Retain this batch without another push for
  now; combine the next remote update at a meaningful integration milestone.
  Compare full local/remote SHA when pushed. Do not merge without passing CI.

Full S1-S3 remains active: stable physical sub-10-ms IO/Bus, actual lower native
periods, source-age provenance and sustained service still need qualification.
