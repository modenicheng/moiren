# Physical-Master Target1536 Continuous Source

Date: 2026-11-03, Asia/Shanghai. Software baseline: `0d686c8`.

## Scope And Priority

First-packet investigation remains deferred and is not a prerequisite for
independent application development. The user's approximately ten-minute manual
run is useful usability feedback, not calibrated physical latency evidence.
No unchanged soak, speculative production repair or new mandatory S1-S3 feature
is introduced. Missing physical multichannel equipment does not block software
work. Native audio, OBS and desktop execution stay on `moiren-lan`; the host
only compiles and runs offline/headless checks.

This slice asks one different bounded question: does the physical-master route
with an explicitly admitted1536-frame virtual target preserve a continuously
changing six-second source in an independent OBS output recording? Earlier
sparse Compatible physical and continuous pure-virtual recordings answer
different questions. This is not a parameter sweep or a lower-default promotion.

## Exact Candidate

The graph is VAD Playback -> unity Bus -> DAWN physical render, F32/48k/Engine64.
Source Shared480 must actually negotiate Client3/F32/capacity1056, without
fallback or automatic format conversion. Render Shared336 must actually retain
capacity738, target512, wake64 and callback64. Virtual input is target1536,
capacity4800, jitter96, declared packet480/gap512, recovery cap8. Requested
values are not substituted for the published native preparation readback.

The existing whole-wake model gives704 Engine frames,706 native transfer,
738 low water and1346 modeled minimum target. Exact minimum storage is2754:
1536 target +480 publication +706 transfer +32 FIR. A new preparation-time
regression covers these numbers,2753 refusal,2754/8192 admission without target
changes, and native packet480 admission versus481 refusal. All23 virtual-budget
tests pass; this is a characterization test, not a reproduced production defect
or evidence of a native quality pass.

The reused probe is built from clean `50c97e5`,7612416bytes, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
It is not labelled a new HEAD build. The only intervening Windows-audio source
changes are test files, not the product implementation or probe sources.
The source is12 contiguous24000-frame indexed-v2 markers,288000 total frames,
stereo48k, amplitude0.005, with no scheduled interior silence.

## Isolation And Lifecycle

Remote root: `D:\MoirenTest\20261103-physical-target1536-0910`.
Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-physical-target1536-20261103-0910`.

One owned portable OBS instance uses fresh settings, loopback port4471,
lossless PCM24 stereo48k, monitoring disabled, gain1 and sync offset0. The
original OBS/QQMusic owners are external and protected. The return boundary is
WASAPI output loopback, not DAC/acoustic capture or calibrated physical delay.
Raw PCM/scalar data remain remote; only compact evidence may transfer.

Task lifecycle has cooperative125-second checks and finite held-process joins,
plus a scheduled420-second outer limit. This is not a hard125-second watchdog
or an unconditional process-tree cleanup guarantee. Recording shutdown must
observe inactive, release its websocket, then close/join the owned OBS window;
unresolved transitions remain failures with held-owner cleanup. There is one
native attempt, with no unchanged retry after failure.

Fresh pure runner checks pass2 target,6 source/master and14 composite shutdown
tests. The standalone older lifecycle suite retains one superseded socket-order
assertion failure and is not called green. Independent safety review finds no
parameter/provenance blocker and explicitly notes the cooperative deadline.

Initial deployment verification fails on PowerShell5 treating unittest stderr
as a terminating NativeCommandError. A follow-up verifier then observes a null
Start-Process ExitCode. Both original failures are retained; neither starts
audio or changes the pinned runner. A held Diagnostics.Process with captured
streams/exit status verifies all22 pure tests and the relocated isolated
environment fingerprint. Final selfcheck completes Ready/0 before native
launch. These are harness issues, not audio production regressions.

## Acceptance Boundary

No source-to-recorder or calibrated physical latency result is implied by
budget admission, source submission, a clean process exit or a software test.

The single native task completes Ready/0; acquisition runs09:18:50.810-
09:19:30.301 +08:00. Its acquisition wrapper completes,
but the graph probe exits1 normally and reports unqualified: no external
marker-bearing measurement was analyzed by that probe, calibration is absent,
and the internal/WASAPI boundary cannot qualify physical end-to-end latency.
The wrapper intentionally permits this diagnostic exit; it is not a quality
PASS. OBS exits0 and no process requires forced termination.

Actual source480/F32/capacity1056 and master336/capacity738 are exact Client3
negotiations with no fallback. Published virtual preparation retains704/706,
low738, modeled minimum1346, target1536 and capacity4800. Source submission
acknowledges all288000 changing frames, then stops/joins normally. The raw
Playback witness retains384960frames/1539840bytes across802 successful calls,
no stale/epoch exclusions, and closes with identity_changed at teardown.
This raw witness precedes decoding and is not graph/output delivery evidence.

The last retained running qualification reports zero drop, silence, underrun,
discontinuity, epoch-reset and delivery-bound counters. The provider's decode
envelope maximum106319*100ns=10.6319ms is within the declared512-frame bound;
its own scope is explicitly not a complete carrier-publication/delivery proof.
Final native output reports388256 written frames and zero dropped/silent/wait
timeouts. Final teardown retains a discontinuous SRC checkpoint with xrun1;
its event phase is not silently classified as a steady-state production fault
or removed. Trace stream completes but scalar history is incomplete:
6023 retained callbacks, refusal missing_callbacks. Unknown spans must remain
unsupported during replay rather than being filled, cropped or called clean.

The09:22:09 remote audit confirms no remaining test owners/running tasks, a
healthy bare `ROOT\MEDIA\0001` bus, unchanged driver/boot and original protected
OBS/QQMusic identities. Fresh catalog defaults, render volume/mute/mix formats
match the native pre-catalog, and all10 deployment pins remain exact. This
checks observable state; preserved process identities do not prove unrelated
user audio was uninterrupted. No host audio/UI or host process cleanup occurs.

Closed-data waveform analysis follows after its actual completion; no native
retry or background soak is left running.

## Concurrent Software Integration

The host uses an external clean `git archive 0d686c8` plus only the explicit
virtual-budget test overlay, not the preserved dirty worktree sources. All1399
archive file entries are accounted for; every unoverlaid file matches the
archive bytes, and the overlay matches the intended worktree test exactly.
All27 original dirty/cache hashes remain unchanged.

Fresh targeted tests pass5 zero-refill,19+8+24 analyzer,20 measured headless UI
and23 virtual-budget cases. Full feature-on Windows workspace testing passes
2360tests/9ignored/0failed across71 harness summaries, including documentation
tests. Feature-on workspace all-target check, strict Clippy `-D warnings` and
workspace rustfmt check exit0. The bounded host verification finishes09:26:32
while the independent remote acquisition/analysis proceeds. No Linux/Miri/CI
result or native-window acceptance is implied by these host results.

## Closed Recording And Visual Result

One reviewed offline adapter reuses the unchanged pinned Playback helper,
nominal master helper and frozen association/audit implementation. Fourteen
adapter tests pass locally and in the existing remote isolated analysis
environment. The first local read during the agent's unfinished edit retains
its missing-field RED; the final fixed files and14-pass result are separately
verified. The adapter itself changes no production logic, gate, alignment,
rate model or native acquisition.

The closed MKV is10429105bytes. FFprobe confirms one48k stereo PCM24 audio
stream; FFmpeg decodes all830464frames,17.301333seconds, without resampling,
downmixing or a cropped-prefix success. The strict Playback helper refuses
`closed trace schema/qualification/completeness` for the incomplete scalar
history. Consequently the nominal master helper never runs; there is no
frozen anchor or full waveform comparison. All830464frames remain explicitly
unassessed_no_frozen_anchor. This is an instrumentation/analysis limit, not
evidence of lost audio or a newly identified product defect. No threshold is
relaxed, missing callback interpolated, partial interval called passed, or
native test repeated to obtain a lucky trace.

The actual `analysis-output/rms-coverage.png` is downloaded and visually
reviewed. Its top panel shows an early large, unequal-channel transient near
recorder frame140000 and a later approximately six-second lower-energy interval
near310000-598000. Neither interval is source-aligned or attributed to Moiren,
Windows, contamination or another application. The transient is not cropped
out. The bottom panel keeps the entire recording unassessed. An RMS envelope
cannot establish waveform continuity, identify delay, or certify output purity.

All19 closed input/tool files are hashed before and after analysis without
changes. The adapter records original unqualified/exit1, missing_callbacks,
identity_changed and teardown checkpoint observations. Every quality/physical
qualification flag staysfalse and all source/physical delay fields staynull.
The offline analysis exits0 only because it completes this bounded refusal and
evidence accounting, not because the waveform passes.

The first compact-export attempt expects an unattempted master log and refuses
before creating an archive. The corrected export checks the command inventory
and preserves that explicit non-attempt; it does not rerun analysis or audio.
Only138114bytes transfer, SHA256
`7A03D8475140EC5BE809B4DE58CD95D1EC06DA9179FD128609C26D036237D9B4`.
All45 payload entries plus the exact manifest are length/SHA256 verified before
extraction; no raw PCM or scalar stream transfers. Final09:28:35 remote audit
again confirms no owners/running tasks and unchanged protected identities,
driver/boot, healthy bare bus, defaults and render controls. No background soak,
push, default promotion, desktop retained-Apply pass or physical qualification
follows. Independent product development is not blocked on another recording.
