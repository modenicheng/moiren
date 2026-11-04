# Minimum I/O Pipeline Integration Checkpoint

Date: 2026-11-04, Asia/Shanghai.
Last verification update: 2026-11-05, Asia/Shanghai.

The active objective remains the complete S0-S5 pipeline in
`docs/vad-low-latency-goals-2026-10-28.md`. Callback instrumentation, source
integration and CI are necessary increments, not replacement completion criteria.

## Callback Integration Closure

`1b33b68` commits the24intended callback code files and their two validation
records. The earlier v5 integration receipts are verified without relabeling
their original source identities:2476workspace tests with10ignored,260RT tests,
strict workspace/RT Clippy, both fmt checks and ordinary release report exit0.
The previously pending release completed on2026-11-03at19:23:33.

Fresh isolated-source checks pass26audio callback tests with1ignored, the
actual application callback publication/dedup regression,260RT tests and fmt.
All24current callback code hashes match the final overlay manifest. The29other
dirty/cache file hashes match the initial snapshot; none is staged or reverted.
The exact scope and earlier failed attempts remain in the
[callback receipt](2026-11-03-virtual-callback-timing.md).

## Current Main Integration

The existing draft PR26cannot merge its old head into current `main`. A
read-only merge preview finds12conflict paths. Integration therefore uses the
separate `minimal-latency-integration` worktree, starting from1b33b68and merging
pinned remote main95972f2. The original worktree and its remaining drafts stay
available. This checkpoint does not include the root worktree's local unpushed
dependency bump.

Conflict resolution preserves current main's ASIO/backend selection, capture
module split, spectrum/meter signal architecture and consolidated UI editor
enumeration, alongside exact period requests, Process/VAD transport intent,
native evidence and retained-owner behavior from the latency branch.

The retired example `LevelMeter` tests refer to an API removed by main. Their
exact-window, partial-tail and epoch/frame-discontinuity contracts already have
current `SignalLevelMeter` coverage in `meter/signal.rs`, including
`multiple_exact_windows_publish_and_contiguous_partial_tail_is_retained` and
`epoch_frame_gap_rewind_and_rate_changes_restart_only_measurement`.

## Integration Regressions And Current Verification

Measured, retained-evaluation and live-window Apply previously cancelled only
automatic startup. Main also schedules automatic resume, so an explicit Apply
could leave older automation pending. The integrated paths now use
`cancel_startup_automation`, with assertions in each corresponding test group.
The focused managed run passes 116 tests with 2 ignored. This is software
evidence, not native desktop retained-evaluation acceptance.

The first complete merged workspace attempt ends at 2026-11-04 23:49:35 with
exit 101: the app library reports 833 passed, 1 failed and 3 ignored. The failure
is `legacy_meter_wakes_on_changes_only_not_on_publications`. Subsequent checks
in the serial runner have not run and must not be reported as passed.

Investigation finds no production meter diff against pinned main. The test
slept without draining queued windows, then expected the first drain to report
a delivery stall. An explicit wait for two queued output publications reproduces
the failure: both statuses remain Live and the newly observed delivery ages are
about 76 microseconds, despite the preceding sleep. The test now polls through
the queued tail, requires the actual Stale transition to wake, verifies a
persisting stall stays quiet, and requires recovery to Live to wake. Production
freshness and value/status-only wake rules are unchanged. Focused and complete
verification remain distinct: the focused monitoring group now passes all
30 tests; complete final-source verification is still pending.

Independent UI review also finds omitted settings cache inputs: transport-only
draft changes and measured/exact/retained/live-window state. Headless
frozen-snapshot regressions reproduce these failures. Their scoped repair and
verification pass all 70 settings tests, including 7 cache-interaction cases,
3 measured frozen-snapshot cases and transport-only draft equality. The key
tracks rendered local state and raw input identity/text, while unchanged polling
reuses the cached paint. Full final-source verification is still in progress;
historical suite results do not substitute for it.

Other current merged-source checks pass: 508 core/signal/engine tests, 17 portable
driver groups, 17 Python suites and 5 portable PowerShell suites. The Python
plotting fixture import repair and the PowerShell 7 requirement are recorded
with their original failed attempts. None starts native audio or loads a driver.

The current driver also passes `driver/vad/build.ps1 -Analyze` at
2026-11-05 00:13:52 with WDK 10.0.28000.0, code analysis, catalog generation and
explicit INF validation (exit 0, no reported errors or warnings). SYS/package
hashes are retained in `wdk-package-pins-20261105.json`. The package is unsigned,
uninstalled and not native-qualified; no source change to terminal categories
or naming policy is included.

The final-source workspace run completes at 2026-11-05 00:19:47 with exit 0:
2607 passed, 10 ignored and 0 failed across 73 harnesses, including headless
GPUI tests. The all-target workspace check completes at 00:20:38 with exit 0.
All 885 recorded code/script/manifest hashes still match the pre-run snapshot.
Strict Clippy, standalone RT, probe examples, final fmt, release and current
remote CI are not yet closed by these two successful commands.

Strict workspace Clippy subsequently passes at 00:21:34. The standalone RT
entry initially refuses `--locked`: incoming main adds `moiren-plugins` to the
engine, but its independent lockfile lacks that path dependency. An offline
Cargo resolution adds only `moiren-plugins`, `libloading 0.8.9`, `cfg-if 1.0.5`
and the corresponding dependency edges; no existing locked package is changed.
The RT suite then passes 269 tests. Its strict Clippy identifies unused COM
helpers and production-only `probe-tools` cfg in the newly shadowed negotiation
module. The harness now applies its existing documented dead-code/probe-cfg
allowance only to that shadow module; library lint policy remains unchanged.
The final RT/Clippy rerun, probe examples, fmt and ordinary release follow this
scoped harness repair. Original lock/refusal and lint-failure receipts remain.

The final standalone RT rerun passes 269 tests and strict RT Clippy exits 0.
The opt-in end-to-end probe initially fails compilation because three physical
test fixtures omit the new optional callback field; they now explicitly retain
`virtual_callback: None` alongside their absent virtual clock. No production
projection, assertion or qualification gate changes. Its rerun passes 740 tests
with 1 ignored, and the VAD app integration example passes 117 tests. Root and
standalone RT fmt checks both exit 0. The original workspace's source remains
identical except these two probe-only fixture files and the two RT harness
lock/lint files; those exact changes have their separate successful checks.
Ordinary release and current remote CI remain pending at this checkpoint.

At 2026-11-05 00:01:56 all 29 non-callback dirty/cache file hashes in the original
latency worktree still match the initial snapshot. The independent integration
worktree does not absorb or discard those drafts.

## Target Readiness

One read-only SSH preflight reaches the existing pinned `moiren-lan` target at
23:15:31. It reports machine `LAPTOP-NP49RG6H`, build22631, boot2026-10-30,
24266188KiB available memory and144438063104bytes free onD:. The DAWN PRO2
render endpoint is present. No app/probe/analysis owner or running Moiren task
is returned. Existing OBS12648and QQMusic15096identities are retained. No new
native audio, UI, recorder or driver operation starts.

The first console JSON has legacy-encoding replacement characters in localized
endpoint labels; its IDs and ASCII fields are usable, but its localized text
is not Windows-name acceptance evidence.

At 2026-11-05 00:11, `vmrun -T ws list` reports zero running VMs. The verified
dedicated `D:\VM\Windows 11 x64\Windows 11 x64.vmx` exists, but its single
headless start attempt exits -1 because a password is required. No password
search, bypass, settings change or driver operation follows. The earlier SSH
timeout is therefore not evidence of a running guest failure. Native driver
work requires the user to unlock/start that existing VM; independent source,
portable and integration work continues.

## Requirements Still Open

- S1: successful matched routes and complete changing-waveform correspondence.
  The closed timeline acquisition still retains two delivery-gap violations
  and2827/689baseline/replacement frozen-model residual frames. Residuals alone
  do not identify dropped frames or a production root cause.
- S2: configuration-specific native period/block and sustained VAD acceptance.
  Genuine `0x8007051A` and epoch/rebase evidence remains separate from the
  deferred initial packet.
- S3: ordinary desktop retained Stop/join/Evaluate/explicit Apply acceptance,
  plus continuous waveform quality. CLI measured replacement does not cover it.
- S4: supported Windows-visible naming, documented native endpoint identity
  consequences and the required streaming/held-client/race/repetition matrix.
  An unchanged rerun of the known generic-speaker/composed-MIC result cannot
  close it. The required stable identity is the project identity. Name-derived
  interface references deliberately refresh Windows endpoint composition;
  replacing them with fixed references would restore an earlier stale-name
  failure and is not an accepted repair.
- S5: repeated-duration, controlled-load, injected-fault and sustained non-silent
  qualification with a calibrated physical reference. Neither sub-10-ms success
  nor impossibility on the declared target is established.

No smaller default, physical delay, winner between VAD/Process, or overall
pipeline completion follows from this checkpoint.

## Evidence

Current host evidence:
`D:\coding\moiren-evidence\20261104-pipeline-resume`.

The retained callback aggregate:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-virtual-callback-20261103-1800\verified-integration-v5-summary.json`.
