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

### 2026-11-05 Release And Cross-Platform Follow-Up

The ordinary release subsequently completes at 00:31:55, exit 0. The pinned
`moiren-app.exe` is 35177984 bytes, SHA-256
`261D0C4990DC518DF189240BB8DD16C190CE00CE8078C3B5D130928C0B52B928`,
built from integration commit `b471975b144c372163e1f814b99e61b69a8160fc`,
tree `e759c9c58b843c61535b6e5aa111411040e0168f`. It is not deployed or run
natively. `release-pins-20261105.json` retains the corrected quoted tree query;
the earlier PowerShell argument failure has its own receipt, not a new build.

That integration commit incorporates pinned remote main `95972f2`, resolves
the twelve merge conflicts, and is pushed normally to
`origin/feat/minimal-io-latency`. PR26 remains Draft and mergeable. This is a
development/evidence integration checkpoint, not authorization to merge the
entire branch into main; the existing split-before-merge policy remains.

Exact-head CI run `41868136039` exposes two distinct portability/test-boundary
failures. Linux test/Clippy and both VAD Miri jobs reject unused Windows-only
callback helpers. The repair narrows their cfg to their actual Windows callers;
portable callback tests and the lint policy are unchanged. Both engine compiler
Miri jobs reach the native plugin fixture's `Command::status` and fail on an
unsupported process-spawn foreign function. Only that native cdylib fixture
module is excluded under Miri; its native test and the remaining portable
compiler suite remain enabled. Original CI failures are retained separately.

The callback repair has fresh Windows callback coverage (26 passed, 1 ignored),
Linux VAD coverage (51 passed), and both Miri borrow models (51 passed each).
The Tree Borrows run completes at 00:41:07. The compiler repair's Windows native
suite passes all 54 tests, including the plugin fixture; strict Windows engine/
audio all-target Clippy and root fmt also exit 0. Linux native compiler passes
54 tests, while both compiler Miri modes pass all 53 portable tests (the one
native fixture remains exercised outside Miri). The Tree run completes at
00:55:52. Exact overlay hashes match the integration worktree.

The first Linux strict-Clippy command cannot run because nightly lacks its
Clippy component; the component is then installed. The next command reaches an
unrelated existing EQ `needless_range_loop` diagnostic on nightly 1.101.0.
No production loop or lint gate is changed to make this portability patch pass.
CI uses stable 1.99.1, so that exact toolchain is being prepared for the relevant
strict-Clippy check. Both original failures are retained rather than reported
as passes. The portability repairs are committed as `fa47ffc`; fresh Windows
audio-library regression passes 873 tests with 5 ignored, and the unchanged
VAD integration example passes all 117 tests. These edits still require new
exact-head remote CI; neither this follow-up nor the previous local workspace
success proves remote CI is green.

The original temporary WSL source directory is absent on the fresh inventory,
with no Cargo/Miri process running. The next verification uses a new isolated
persistent cache directory, the pinned archive, the required ABI header and
exact overlay hashes. It does not restart an observed live process or treat an
observation timeout as terminal.

The independently reviewed S4 software-output category candidate is committed
as `c321409`. All 18 host groups, 13 MSYS2 portable groups and 13 Linux portable
groups pass against the imported source; the analyzed WDK build and explicit
INF validation pass. Its unsigned package has separate integration pins in the
[candidate receipt](2026-11-05-s4-virtual-playback-category.md). Actual Windows
names, native streaming/default-role/migration behavior and the broader S4
matrix are still unverified. The strict historical gates remain unchanged.

At 00:50:48 all 29 original non-callback dirty/cache hashes still match; no
draft is absorbed. The 00:48:44 remote read-only physical-target preflight
confirms available memory and no owned audio app/probe or running Moiren task.
It also finds the older stalled transfer shell, recorded without broad cleanup.
No new native test, unchanged soak or host driver/UI operation follows.

At the next source checkpoint the prior CI run has successful Windows tests,
Windows Clippy, driver host checks and the other completed portable/Miri jobs,
but retains its six known failures and a still-running engine host Tree job.
A normal batch push of the actual repaired source can supersede that run under
the repository's concurrency policy. Any cancelled old job is unverified, not
a pass; this is a new source checkpoint, not a retry because polling expired.
The matching new head must be inspected independently and the PR stays Draft.

### Portable Shadow Caller Follow-Up

The normal three-commit batch push completes at 01:07:46 with remote head
`796e5ce759e2f63d6b39b22c790b88ab824e6f9b` and tree
`7926992ffa32084d436a9b8648c06761f2bfab71`. New exact-head CI is
`41874555383`. The previous run finishes independently with thirteen successful
jobs and seven failures (six underlying failures plus its aggregate Miri gate);
no old in-progress job needs restarting or cancellation to obtain this result.

The new Linux test/Clippy jobs expose a missed portable caller in
`tests/virtual_clock_observation.rs`: it compiles the production clock module
under a counting allocator and must retain callback publication coverage. The
earlier cfg-only repair was therefore incomplete. The follow-up restores the
original portable test cfgs and adds a library regression exercising the same
callback helpers, owner isolation and unchanged clock counters. This removes
the Linux dead-code cause without deleting the shadow test or weakening lints.

The isolated RED compiles that new regression with the incorrect Windows-only
cfg and fails with the same missing-method error. With the correct cfg, Linux
virtual-owner library coverage passes 72 tests (2 ignored), Linux and Windows
shadow targets each pass 9 tests, and Linux VAD coverage passes 51. Strict
Windows audio all-target Clippy and fmt pass; Windows callback library coverage
passes 27 tests with 1 ignored. Both VAD Miri models pass 51 tests on the exact
follow-up bytes, Tree finishing at 01:15:52. The first-head CI failure and prior
cfg patch remain visible, not rewritten as success. CI-matching Linux stable
installation is still live and making download progress; no unchanged process
is restarted because a tool observation expired.

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
