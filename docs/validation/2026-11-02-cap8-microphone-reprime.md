# Engine64 / packet480 reset recovery contract

Date: 2026-11-02. Baseline: `bf90e3701f2b7c7f82dd296ea339fb5c1821ffd8`.
This is a test-only S2 increment, not a production fix or native qualification.
The [target1536 native failure](2026-11-02-format-query-and-target1536.md)
remains failed; these models do not explain its initiating gate change.

## Why this test

The original S2 scope requires reconciling a bounded multi-block virtual-clock
repayment with lower operating targets, including microphone reset reprime.
The existing compiled graph matrix covers Engine256, recovery cap1 and native48.
It does not exercise the Engine64 / cap8 / native480 configuration used in the
recent bounded native attempt. Focused read-only audits found no demonstrated
budget-to-runtime defect: threshold and max-wait are applied, and the two servo
directions remain consistent. Do not invent an audio-policy fix for that result.

## Test changes

`vad/provider/pcm_worker/recovery_burst_tests.rs` adds four tests. The existing
compiled microphone helper accepts an explicit block size; all existing calls
still pass256. Only the test module registration and these test sources change.
No production code, driver, default, quality gate or first-packet policy changes.

The exact declared model is48k/stereo, Engine64, cap8, native480,
target1536/capacity4800, packet bound480, gap bound512 and jitter96. The gap
remains a declaration, not measured native service. Assertions cover:

- Prepared threshold1536 and one45.3989ms deadline (`453989`100ns ticks),
  unchanged across repeated pump/wait calls. Empty expiry submits exactly one
  packet and counts480 silent frames; waits remain at most1ms.
- A1535-frame carrier cannot submit before expiry; reaching1536 releases the
  first packet. This separate threshold-only test deliberately uses fixture
  prefill. Initial startup stays immediate, while cancellation or a new epoch
  fences consumption and clears reset reprime.
- A compiled constant Source -> unity Bus -> microphone graph uses the real
  SRC and carrier. Producer/consumer reset acknowledgment and empty-carrier
  discard precede fresh production; there is no test-side fresh prefill here.
- `ClockSchedule::due` runs at one unchanged overdue timestamp and yields
  backlogs8 through1: eight separate64-frame renders, zero clamps, then no due
  block. The complete burst remains below1536 actual available native frames;
  continued scheduled publication must reach the threshold before submission.
- Both f32/f64, three first-wake phases, three synthetic10ms completion phases
  and both graph/pump orderings give36 compiled scenarios. Each160ms model
  resumes at least ten writes with nonzero content, no counted silence,
  carrier xrun, driver drop or additional epoch change.

Synthetic completions and a constant source validate the nominal compiled
recovery contract only. They do not exercise Windows scheduling, native gate
rebasing, sample identity/continuity, audible quality or physical latency.

## Verification evidence

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s2-cap8-reprime-20261102-1940`.
The clean baseline archive SHA256 is
`EFC02B7BFE8072E48A982814D8308539ECDFB92666CBE00A7ADEE89A713A18CB`.
`exact-export-verification.json` compares every archived file byte-for-byte:
only the two intended existing test files differ, and only the new recovery
test file is added. The27 preexisting dirty/cache files are excluded.

An initial test compilation fails on an incorrectly named telemetry field;
`focused.log` retains that error and is not RED behavior evidence. After the
field/closure correction, `focused-v2.log` passes all four tests. No production
change was needed or made.

Three isolated mutation controls then fail by assertion, each exit101, not by
compilation error: substituting two packets for the prepared threshold, adding
one second to the forwarded wait, and reducing the fixture schedule cap to7.
Every mutated file is restored byte-for-byte in a `finally` block. The restored
four-test suite passes. These demonstrate sensitivity of the new assertions,
not a claim that baseline production had any of those injected defects.

Fresh clean-export checks pass the full Windows audio library765/4ignored,
all51 marker/rate/qualification tests and20 measured desktop UI tests. The
library includes all five zero-refill tests. Strict audio-crate all-target
Clippy and workspace formatting also pass. Focused counts overlap larger suites.

The full clean-export Windows workspace test with `moiren-app/gpui-test` passes
2311 tests,9 ignored,0 failed across71 test/doc surfaces, including13 UI
acceptance cases. `cargo check --locked --workspace --all-targets -j1` also
passes, as does strict all-target workspace Clippy with the same UI feature.
Exact commands, exits and elapsed times are retained in
`workspace-checks.json`; totals are in `workspace-counts.json`. This is local
software verification, not a Linux/Miri/remote CI result or native audio test.

## Remote and error-classification boundary

A read-only SSH audit confirms `LAPTOP-NP49RG6H` has no test audio/Python owner,
only the bare Moiren control bus, the unchanged pinned running driver, and the
same external OBS/QQMusic process creation identities. No native audio run,
process termination, UI launch, device mutation or unchanged soak accompanies
this increment; development proceeds without waiting for absent physical gear.

The retirement review does not justify changing cleanup classification.
`0x8007051A` denotes revision mismatch, distinct from WRITE expiry; the driver
can return it for identity or shape disagreement, not only expected old-gate
fencing. Current caller boundaries retain aggregated errors, without enough
typed per-slot context to exempt only a proven benign old-owner completion.
Keep all existing failure evidence and behavior; do not filter error strings,
touch dirty PCM diagnostics or revive withdrawn retirement experiments.

The smaller native candidate, matched-path waveform quality and calibrated
physical qualification remain open. No lower default, path winner, sub-10ms
result or overall S1-S3 completion follows from these software tests.
