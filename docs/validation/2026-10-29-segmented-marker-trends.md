# Segmented Offline Marker Delay Trends

Date: 2026-10-29. Worktree: `D:\coding\moiren\.worktrees\minimal-io-latency`.
Branch: `feat/minimal-io-latency`. Baseline:
`5a09fa1d7068bf10d88521439a2a21438f30f389`.

## Scope

Local feature commit `252b1ea9b7682046421852fa6dd05467fb2d42ee` adds optional
`trend:{min_markers,min_span_frames}` to actual offline PCM schedules. Report
schemas5/6 opt in; omitted trend retains schemas1/2/3/4 and field omissions.
Every fit retains first/last window indices, reference endpoints, count/span,
origin-relative intercept, relative-delay slope and descriptive residuals.
Short/insufficient/precision-refused segments remain visible with reasons.

Fits use checked integer origin subtraction before float conversion, centered
OLS, explicit exact-integer limits2^53-1 and residual RMS `sqrt(SSE/n)`.
Absolute origins near `u64::MAX` do not lose nearby marker differences. No
outlier rejection or trend subtraction changes the raw delay distribution.
Slope is relative delay change, not identifiable hardware clock skew; residuals
and integer resolution are not physical uncertainty. Negative slopes do not
add support for negative processed-minus-reference detection offsets.

Failed markers split segments. Packet validation precedes fitting, and invalid
packet spans split segments even entirely between windows. Half-open endpoint
touches split fits without invalidating nonoverlapping markers. Coverage
mismatch invalidates every marker and cannot leave a stale prevalidation fit.
Traversal is O(packets + windows), not a Cartesian product.

Without metadata continuity is explicitly unverified. Clean packet metadata
remains diagnostic, unverified provenance, not a native epoch or whole-waveform
continuity. Packet faults still fail the existing CLI verdict even if every
scheduled marker succeeds; trend fit success never qualifies latency.

Actual imported-live reports retain full trend-only analysis under
`trend_measurement`. Combined rate+trend remains once under the existing
`rate_search_measurement`, preserving rate-only behavior and legacy omissions.
Dry-run tests open no audio device and stay unqualified with no native capture.

## Fresh Checks

Host commands use `CARGO_TARGET_DIR=D:\coding\moiren\target`. Logs are retained
under the worktree's `target/s1-s3-20261028/` and are not staged.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe` | 323 passed | `trend-probe-final.log` |
| `cargo test --locked -p moiren-windows-audio --lib --test latency_marker --test latency_marker_rate --test latency_qualification` | 551 library passed/2 ignored;19 legacy marker,8 rate marker,24 qualification passed | `trend-audio-full.log` |
| `cargo clippy --locked -p moiren-windows-audio --example e2e_latency_probe -- -D warnings` | passed | `trend-final-clippy.log` |
| `cargo check --locked -p moiren-windows-audio --target x86_64-unknown-linux-gnu --example e2e_latency_probe` | passed; existing cfg-specific unused `progress_grace` warning | `trend-linux.log` |
| `cargo fmt --all -- --check`, scoped probe `rustfmt --edition 2024 --check`, `git diff --check` | passed | direct output |
| `cargo run --locked -p moiren-windows-audio --example e2e_latency_probe -- --help` | new config/model/continuity/report caveats present | `trend-help.log` |

The slice only changes an example/report path, not library/production RT code.
App, RT harness, remote CI and Miri are not rerun in this slice. Linux check
is not warning-free. Existing ignored tests remain ignored, not counted passes.

## Regression Evidence

- First focused run:4 tests fail because `trend` is not yet accepted by the
  real schedule parser; the legacy omission test passes. This is missing
  feature evidence, not a claim of a preexisting production bug.
- First implementation:5 focused tests pass. Constant and both delay slope
  signs preserve raw delays/distribution and leave qualification unevaluated.
- Expanded suite:11 pass; trend-only live import genuinely fails because its
  report field is null. Retaining the full optional analysis fixes this gap.
- Final suite includes14 trend tests and the additional combined rate/trend
  import regression. Continuous global resampling fixtures retain raw
  positive23/33/43 and negative23/13/3 delays while reporting +/-2000 relative
  ppm. The fit never detrends those delays.
- Packet tests cover gap-only reset, sequence loss, unknown timestamp/flags,
  exact1423..1536 half-open gap, overlapping faults, one invalid packet spanning
  several windows and incomplete coverage. All faults retain diagnostic failure.
- Large absolute origins, insufficient count/span, nonincreasing positions,
  explicit precision refusal, retained outlier residuals and panic-on-read/seek
  invalid configuration checks are covered. Invalid config refuses before IO.

Two independent read-only reviews identify no actionable Important/Critical
findings. Their scopes include packet segmentation/half-open bounds, precision
policy, live retention, legacy omissions and diagnostic provenance. Reviewers
do not rerun cargo or access native audio/VM; the fresh checks above are the
controller's runs, not independently duplicated test evidence.

## Remaining Gates And Push Cadence

This advances S1 software measurement only; full S1-S3 remains active. Native
lower-period negotiation, source-age provenance, sustained/future-safe service
and stable matched physical IO/Bus below10ms remain open/deferred. VM timing
qualification stays deferred; no VM access, deployment, driver build/install,
native audio or physical measurement occurs.

Correctness-mode timing/quality continuation with0..3000ms extra progress grace,
latency-sensitive fail-fast and all hard identity/schema/resource/IO/storage/
cleanup faults remain unchanged. Grace never increases production buffers or
becomes an allowed latency target. Whole-native-drain/wake admission remains.

Updated user instruction: preserve local atomic commits but do not push each
small change. This slice is retained locally for a later meaningful integration
batch; no new push/CI trigger is performed here. Unrelated `gate.log` stays
untouched. Remote-ref state is not claimed freshly verified by this record.
