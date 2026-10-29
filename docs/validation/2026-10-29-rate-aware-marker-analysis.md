# Bounded Rate-Aware Offline Marker Analysis

Date: 2026-10-29, Asia/Shanghai. Parent:
`394202c5f6477e71e9af44a2964f46201e5fd4f0`.
Worktree/branch: `minimal-io-latency` / `feat/minimal-io-latency`.

## S1 Software Progress

The old detector can reject longer broadband markers after sample-rate
mismatch. A new explicitly opt-in offline detector searches a finite template
rate grid using caller-provided template/onset-score scratch:

- Positive ppm stretches about the marker frame-zero onset. Templates use
  zero-extended linear interpolation; captured PCM is never padded or corrected.
- Preparation admits the complete largest warped template, all searched onsets,
  source/template size caps, scratch and checked work before scanning samples.
  A maximum 129 hypotheses prevents an unbounded grid. Nondivisible endpoint
  ranges, invalid ratios/steps and insufficient budgets are refused.
- Every distinct onset competes across every rate, including adjacent, opposite
  polarity and weak peaks. Rate alternatives at the same onset do not falsely
  become competing onsets. Quarter-marker energy/shape/polarity gates remain.
- Returned ppm is a template hypothesis, not hardware clock skew. Rate margin,
  ties, edge hits and integer-onset resolution remain visible. The grid cannot
  guarantee off-grid, time-varying or filtered-SRC matches. No physical
  uncertainty bound or hardware calibration is inferred.
- `analyze-pcm` optionally accepts schedule `rate_search` with explicit
  min/max/step/count/work. It independently detects both channels against the
  original marker, retaining absolute common-recording onsets and raw delays.
  Work includes both channels and the second rate-score pass. Window/read
  budgets cover the warped tail before any PCM read/seek/allocation.
- Rate reports use schemas3/4 for PCM/packet-validated analysis. Omitted rate
  search retains legacy schemas1/2 and omits all extra fields on serialization.
- Packet loss/reset/timestamp/silence invalidation covers expanded windows,
  including an event only in the additional tail. Failed windows retain details;
  successful-only distributions never become a full-waveform continuity claim.
- The live imported-measurement loader retains opt-in analysis in
  `rate_search_measurement`, so rate model, hypotheses, resolution and failures
  cannot disappear behind a bare distribution. No new live collection occurs.

Raw delays are never detrended. Real global-resampling fixtures show
23/33/43 frames under +2000ppm and23/13/3 under -2000ppm. These changes stay in
the original distribution. The fixture uses one continuous source coordinate,
not a new rate origin for each marker. Rigid detection fails the stretched
long-marker cases; rate-aware detection recovers their known onsets.

## Atomic Commits

All code commits were pushed promptly and compared against the full SHA from
`git ls-remote origin refs/heads/feat/minimal-io-latency`:

- `7f0e4ed15e971652f8a6ca24a421578c102f16c5`: core detector, explicit caps,
  preparation, ambiguity/error semantics, allocation guard and scoped plan.
- `edf996cf2a6fbecdd5c558cbd20718f7186a6660`: actual offline schedule/CLI,
  expanded packet coverage, live-report provenance and integration regressions.
- `24f20354245f56429cfeaa2dca37e33c0b7ee164`: global positive/negative accumulated
  resampling drift and legacy report omission regressions.

## Validation

Host commands use `CARGO_TARGET_DIR=D:\coding\moiren\target`. Logs remain
under the worktree's `target/s1-s3-20261028/` and are not staged.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib --test latency_marker --test latency_marker_rate --test latency_qualification` | 551 library passed/2 ignored; 19 legacy marker + 8 rate marker + 24 qualification passed | `marker-rate-audio-full.log` |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe` | 308 passed | `marker-rate-probe-final.log` |
| `cargo test --locked -p moiren-app --features gpui-test --lib --test cli --test transport_settings --test ui_acceptance` | 511 library passed/3 ignored; 2 CLI + 8 preferences + 13 UI passed | `marker-rate-app-full.log` |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` with `CARGO_INCREMENTAL=0` | 176 passed | `marker-rate-rt.log` |
| `cargo clippy --locked -p moiren-windows-audio --all-targets -- -D warnings` | passed | `marker-rate-final-clippy2.log` |
| `cargo clippy --locked -p moiren-app --features gpui-test --all-targets -- -D warnings` | passed | `marker-rate-app-clippy.log` |
| `cargo clippy --locked --manifest-path tools/rt-check/Cargo.toml --all-targets -- -D warnings` | passed | `marker-rate-rt-clippy.log` |
| `cargo check --locked -p moiren-windows-audio --target x86_64-unknown-linux-gnu --all-targets` | passed; existing cfg-specific dead-code warnings | `marker-rate-linux.log` |
| `cargo fmt --all -- --check`, scoped probe `rustfmt --edition 2024 --check`, `git diff --check` | passed | direct command output |
| `cargo run --locked -p moiren-windows-audio --example e2e_latency_probe -- --help` | new schedule/model caveats present; no native IO | `marker-rate-help.log` |

The RT harness total includes shadow unit tests, not176 allocation-guarded
checks. The new offline detector has its own successful-call allocation/free
guard with fixtures and scratch allocated outside the measured scope. This
does not authorize running offline correlation on the audio thread.

Cross-target warnings are in existing cfg-specific `progress_grace`,
`with_publication_gap_100ns`, and `with_retained_owner_origin`, not the new
detector. No warning-free Linux claim or unrelated suppression is made.

### Failing-Then-Passing And Review Evidence

- Initial core tests fail to compile before the new API exists: compile RED,
  not a claimed behavioral defect reproduction.
- New offline schedule/CLI tests first fail on unknown `rate_search`, then
  pass actual resampled marker detection and expanded-tail reset invalidation.
- Independent review finds the second all-rate score pass missing from the
  work total. Zero-delay exact8192/8191 regression reproduces it; preparation
  now includes the second pass. Other preparation/validation/section checks are
  documented linear overhead, never another onset search.
- Review also finds a degenerate compressed marker mislabeled as recording
  silence. The regression first returns `SilentRecording`, then correctly
  returns `InvalidMarker` after template-energy validity is tracked separately.
- A live-loader test initially assumes no QualificationReport object during
  dry-run. Existing behavior does create an explicitly unqualified object; the
  test is corrected to false steady-state/load/native flags, not changed to
  force a new product behavior. It then genuinely fails because the full rate
  analysis is absent. The optional retained analysis fixes that provenance gap.
- Panic-on-read/seek tests prove combined-channel and total work, read/window,
  template and hypothesis refusal before PCM access. Exact expanded boundary
  passes; one-frame-short and missing middle marker remain failed results.
- Core coverage includes both rate signs, zero, off-grid refusal, distinct-onset
  ambiguity across rates, same-onset rate alternatives, silence, erasure,
  clipping/nonfinite input, truncation, arithmetic and source/scratch caps.

Two independent read-only reviewers close the work/error and live-provenance
findings and identify no remaining Important/Critical issue in their scopes.
They do not execute cargo, native audio or VM commands. Failed/corrected logs
remain retained and are never counted as passing evidence.

## Full S1-S3 Remains Active

This advances actual offline drift handling; it does not complete S1 physical
measurement or the whole objective. A bounded multi-marker relative-delay trend
with inter-window packet-continuity segmentation is a next useful software
slice. It must never identify clock skew from varying latency alone or fit
across failed markers/loss/reset. Source-age provenance, smaller actual native
period negotiation, sustained/future-safe service and matched stable physical
sub-10-ms IO/Bus output remain open or deferred.

No VM access, driver build/install, live audio, deployment, remote CI or Miri
occurs. Correctness continues timing/quality recording with0..3000ms extra
progress grace; latency-sensitive remains fail-fast. Grace is not audio latency
and never enlarges production buffers. Identity/schema/resource/native IO/
storage/cleanup faults stay hard. Unrelated `gate.log` remains untouched.
