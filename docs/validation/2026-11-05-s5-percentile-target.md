# S5 Offline Qualification Enforces Both Percentile Targets

Date: 2026-11-05, Asia/Shanghai. Source base: `dccfb20`. This is an
offline acceptance-contract repair, not a new latency or waveform acquisition.

## Requirement And Reproduced Mismatch

The current S5 requirement in `docs/vad-low-latency-goals-2026-10-28.md`
requires p95 and p99 below 10 ms. The older 2026-10-26 design and evaluator
check only p95 <= 10 ms; one existing test explicitly accepts p99 = 15 ms.
That historical rule is not sufficient for the current goal. Historical
receipts and the original design are retained, not silently reclassified.

Actual call chain: `e2e_latency_probe evaluate` parses the submitted reports,
calls `evaluate_qualification`, and writes its result before returning the
qualification exit code. `scripts/latency-qualification.ps1` verifies the
artifacts and consumes that same CLI result. A passing p95 with failing p99
therefore produces `qualified: true`, exit 0, and accepted offline evidence.
This can be reproduced without devices, calibration hardware or native audio.

## Failure-First Evidence And Correction

Before changing production code:

- The real qualification integration harness has 24 passing tests and two
  assertion failures: a p99 target miss and the strict 10 ms boundary both
  return `Ok(())` rather than `LatencyTargetMissed`.
- A regression through the actual CLI returns exit 0 for p99 = 15 ms where
  exit 1 is required. This is an assertion failure, not a compile failure.
- The extended PowerShell wrapper regression also fails with a retained
  pre-fix executable. Its exact earlier binary source was not re-certified;
  that additional characterization is not substituted for the source-pinned
  Rust RED results.

The evaluator now requires both p95 and p99, each plus the existing calibration
uncertainty, to be strictly below the unchanged 10 ms target. Equality is not
"below". The existing `LatencyTargetMissed` serialized error is preserved;
report fields, schema, percentile calculation, sample population, physical
boundary, provenance, duration, repetitions and zero-quality-fault requirements
are unchanged. p50 <= 8 ms remains exploratory, and max remains reported rather
than acquiring an invented release threshold. No device-limited fallback is
inferred or automatically selected by the offline evaluator.

The real CLI regression requires a failed result and retains the original
distribution. The actual PowerShell wrapper additionally checks refused
evidence, exit 1 and `latency_target_missed` for p99 = 15 ms and p99 = 9.5 ms
with uncertainty = 0.5 ms. Synthetic fixtures are explicitly not hardware
qualification evidence.

## Verification

Evidence root: `D:\coding\moiren-evidence\20261105-s5-percentile-gate`.
The four edited code/test files and the isolated Linux overlay are pinned by
`fix-manifest.json` and `fix-overlay.tar`. Linux rehashes all four source files
before executing the checks.

| Check | Result |
| --- | --- |
| Windows qualification integration tests | 26 passed, zero failed |
| Windows full offline probe example tests | 741 passed, 1 ignored, zero failed |
| Linux stable Rust 1.99.1 qualification tests | 26 passed, zero failed |
| Linux stable full offline probe example tests | 583 passed, zero failed |
| Windows and Linux audio all-target strict Clippy | Exit 0 on each |
| Windows non-test probe build and real PowerShell 7 wrapper regressions | Exit 0 |
| Windows workspace all-target check with integration-probe feature | Exit 0 |
| Full workspace/headless with both `gpui-test,vad-integration-probe` | 2625 passed, 10 ignored, zero failed, 73 harnesses; completed 03:19:53 |
| Workspace fmt and whitespace checks | Exit 0 |

The full workspace/headless terminal receipt and the independently parsed
summary are retained. Explicit probe-example tests are not added to the
workspace total. The 29 original draft/cache hashes still match their pins.

## CI And Qualification Limits

Existing exact remote head `fb87ec8` completes CI `41888724160` at 03:13:37,
with all 20 jobs successful, including both test/Clippy platforms, driver host
and every Miri suite/aggregate. The existing watcher exits 0; it was not
restarted or superseded. This green certifies that earlier source only, not
`52d0c39` PCM-readiness or this later S5 correction. New published source needs
its own matching-head CI. PR26 remains Draft/open and split-before-merge.

No native audio, desktop UI, OBS, device or driver operation runs. The original
29 feature-worktree draft/cache hashes remain unchanged. The full S0-S5
objective remains open: matched routes and waveform correspondence, native
period/quality combinations, desktop retained Evaluate/Apply, native naming
and lifecycle matrix, sustained/load/fault runs and calibrated physical p95/p99
acceptance are not supplied by this correction. The driver VM still reports
zero running guests. Neither a sub-10-ms pass nor target-device impossibility,
lower default or VAD/Process winner follows.
