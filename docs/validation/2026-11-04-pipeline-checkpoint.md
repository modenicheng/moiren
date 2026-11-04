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

### Windows-Only Trace Exporter Boundary

After the portable callback fix, the complete Linux audio all-target test
reaches a second independent platform error: `live::src_trace` and its native
fixtures import the Windows-only engine backend, clock factory and stop
signal. The original failure is retained in
`linux-portable-callback-all-targets.log`. Every production caller of this
trace exporter already lives in the Windows-only native runner. Restricting
that module to `cfg(windows)` preserves its Windows tests without hiding the
portable CLI, PCM analyzers or allocation-observation tests.

An independent read-only review finds no actionable issue. With that boundary
fixed, the isolated Linux audio all-target run completes at 01:20:44: 1259
passed, 2 ignored, 0 failed across 39 test harnesses. The Windows E2E probe
completes at 01:19:06: 740 passed, 1 ignored, 0 failed. The tested trace-exporter
source hash matches the integration worktree. Fresh Windows strict audio
all-target Clippy and root fmt also pass. These checks do not start native
audio or replace the pending exact-CI-stable Linux Clippy check.

At 01:24, remote head `796e5ce` still has fourteen successful jobs, two known
Linux failures and three live jobs in run `41874555383`. Both compiler Miri
models now pass. The callback follow-up is committed as `e59d7d9`, but neither
it nor this exporter-boundary fix has matching remote CI yet. The exact stable
toolchain installation remains a live process; it is not restarted to avoid
the download wait. Native S1-S5 acceptance remains unchanged.

The two-commit follow-up batch is pushed normally at 01:29:20. Remote head is
`3c2c094cb174fb4eeba63fb5903591e7c85704dc`, tree
`20207f1d69e08e5081ca8cf4ce6e164d45c28c4e`; new exact-head CI is
`41878002933`. At 01:44 it has actual jobs and a successful driver-host check,
not an all-green verdict. Intermittent GitHub TLS observation failures do not
mean its jobs stopped, and neither run is manually restarted. PR26 remains
Draft; the original 29 draft/cache hashes still match before this push.

At 01:53 the exact `3c2c094` CI Ubuntu test and Clippy jobs both report success;
the latter's archived log identifies Rust 1.99.1 and strict all-target commands,
including probe-tools and app ui-profiler. Windows Clippy also passes. The
previous two Linux failures are therefore repaired on the matching remote
source, not merely masked locally. The complete run still has live jobs, so
there is no overall green claim. Old run `41874555383` subsequently becomes
terminal cancelled under the existing superseding-run policy; unfinished old
jobs are not counted as passes. Local exact-stable installation remains live.

### Offline Terminal/Recorder Audit Refusal

A separately reviewed metadata-only audit is prepared under
`terminal-recorder-audit` in the host evidence root. It preserves the frozen
2827/689 residual ledgers and both delivery-gap violations. Nineteen pure
accounting/timestamp tests and a PS5/PS7 UTF-8 output regression pass; source
hashes match production commit `002aa21`. No audio decoding, refit, alignment,
waveform threshold or production code changes are part of this audit.

At 01:38:12 the one create-once deployment attempt refuses before creating its
directory: the existing SSH account has an administrator token. The subsequent
read-only identity inspection at 01:38:59 confirms the exact expected SID,
session0, `admin=true` and `package_exists=false`. There is no SCP, remote script
execution, ffprobe, native audio, task creation or settings change. The limited
token requirement is not relaxed and no unchanged execution is retried. Thus
no terminal-padding or MKV-PTS conclusion follows; the reviewed package and
original failures remain available for an actually non-admin foreground run.

The independent SFTP-only metadata archive subsequently succeeds at
01:47:59-01:48:00: eight existing JSON/JSONL files, 914870 bytes, all frozen
lengths/hashes matching. No remote shell, audit, ffprobe or PCM transfer is used.
Local full-interval accounting and independent recomputation establish
guaranteed client-FIFO-departed prefixes [0,97104) and [0,385056); each final
Write observes176padding and submits336, leaving at most512possibly queued.
Original residual counts in those prefixes are respectively2315exact and
177..689. The replacement's689residuals therefore cannot all be assigned to
remaining client Stop-tail inventory. This is not DAC presentation, dropped
audio, a residual explanation or a quality pass. The
[FIFO-bound receipt](2026-11-05-terminal-client-fifo-bound.md) retains the exact
scope and original2827/689residual and two-gap failures; recorder PTS stays unknown.

### Streaming Rename Harness Integration

The separately developed S4 streaming harness is imported as twelve frozen
files on `3c2c094`, with an independent review finding no actionable issue.
It adds explicit `--exercise --rename-streaming`, actual old/new Pair Starts,
strict phase-specific PCM, edit while the old Pair pumps, direct old Removed,
exact raw Windows name and fresh Engine/native identity confirmation. Partial
Starts, Stop failures and interruption evidence are retained; diagnostic
completion cannot erase a failed acceptance gate. Legacy `--rename` is not
replaced or weakened. This is one rename per direction, not the complete
race/repetition/sibling/default-role matrix.

Fresh imported example tests pass 127, its non-test build and strict Clippy
pass, and workspace/gpui regression completes at 01:44:35 with 2609 passed,
10 ignored, 0 failed across 73 harnesses. All-target workspace check/strict
Clippy with the explicit probe feature and fmt pass. The optional local Linux
app example check is blocked by missing desktop development libraries, not
reported as a pass. See the [harness receipt](2026-11-05-s4-streaming-rename-harness.md).
Neither a native execution nor Windows name/PCM acceptance has occurred.

The optional local Linux app check subsequently passes 106 tests after the
CI desktop development packages are installed and the unchanged root-logo
assets missing from the isolated source archive are restored from `3c2c094`.
Original environment-preparation failures remain archived. This is a portable
nightly test result, not a CI-stable lint or native-name pass. The harness is
committed as `15ba9d0`; its code is unchanged from the reviewed frozen import.

At 01:55 exact `3c2c094` CI has eighteen successful jobs and one live engine-host
Tree Miri job. It is not timed out or restarted, and the aggregate gate still
needs its terminal outcome. This does not certify the later `15ba9d0` harness
source; that next verified source checkpoint needs its own remote run.

At 02:00 the matching `3c2c094` run `41878002933` completes successfully: all
20 jobs pass, including Windows/Ubuntu tests and Clippy, driver-host checks,
both borrow models and the aggregate Miri gate. The final exact-head receipt
is retained as `ci-41878002933-final.json`. This closes the portability-repair
CI checkpoint only, not the subsequent streaming-harness source or any native
S1-S5 requirement. The next batch retains PR26's Draft/split-before-merge rule.

### Published Streaming/FIFO Checkpoint

The four-commit source/evidence batch is pushed normally at 02:01:01. Remote
head is `303e481d2eda41b721506b32e98e4e2dd9c785c8`, tree
`4b65852e10c952d5408425ef2a64353f4c4866d7`; the PR stays Draft/open with its
original split-before-merge prefix preserved. New exact-head CI `41883280004`
appears after the initial event delay and has actual live jobs. At 02:05 four
jobs succeed and fifteen remain live, with no observed failure; it is not yet
an all-green result. No duplicate workflow dispatch or unchanged rerun is used.

The original exact-CI-stable 1.99.1 download also completes without restarting
its live session. Local strict audio all-target Clippy passes at 02:04:09 and
the thirteen portable driver groups finish at 02:04:11. The preexisting nightly
EQ lint remains retained; no production loop or lint gate is changed. Windows
example `--help` alone is checked inert, reporting the new mode without opening
the driver, audio or UI. Original draft/cache hashes still match 29/29.

At 2026-11-05 00:01:56 all 29 non-callback dirty/cache file hashes in the original
latency worktree still match the initial snapshot. The independent integration
worktree does not absorb or discard those drafts.

At 02:32:42 the published streaming/FIFO head `303e481` completes its own CI
`41883280004` with all twenty jobs successful. Windows/Ubuntu tests and strict
Clippy, driver host suites and all Stacked/Tree Miri suites and aggregate pass.
The exact-head final receipt is retained in
`20261105-s4-error-latching/ci-41883280004-final.json`. This closes that source's
software CI requirement, not the subsequent error-latching correction or any
native/name/physical gate. PR26 remains Draft/open and split-before-merge.

### Explicit Runtime-Error Acceptance Repair

An independent source audit identifies two false-success paths in the probe:
streaming transition/retirement runtime errors can be overwritten by clean
snapshots, and Exited/Removing plus a successful join can ignore an explicit
shutdown error. Three regression tests fail before the fix. Actual observation
paths now retain deduplicated stage/source/text failures, and clean exit rejects
explicit service/graph errors while still allowing error-free Removing.
Windows example tests pass 130; exact Linux stable 1.99.1 tests pass 109. Strict
Windows all-target and Linux example Clippy, workspace check, non-test build,
fmt and the existing application shutdown-timeout regression pass. The
[failure-first receipt](2026-11-05-s4-sticky-runtime-errors.md) preserves the
original RED, corrected lint invocation and unchanged qualification limits.
Fresh full workspace/headless verification with both `gpui-test` and
`vad-integration-probe` on committed `8995d84` completes at 02:34:21: 2623 passed,
10 ignored, zero failed across 73 harnesses. The differing feature set is
recorded; this total is not presented as a delta from earlier gpui-only totals.
No native or broader S4 acceptance is claimed.

### PCM Completion Readmission Consistency

The [PCM completion repair](2026-11-05-s4-pcm-readmission.md) fixes an existing
harness inconsistency: a legal bounded Start re-admission can be pending in the
main loop but becomes an immediate failure in its second completion snapshot.
Both observations now use the same tested owner/gate predicate; pending is
neither success nor failure, and final completion still needs strict 48000-frame
PCM, both Starts, both open gates and all original deadlines. A separate
failure-first regression prevents post-pump snapshot age above the existing
two-second limit from passing. Windows example tests pass 135 and Linux stable
tests pass 114; strict Clippy/check/build/fmt pass. Full workspace/headless
regression with both opt-in features finishes at 02:54:18: 2623 passed, 10
ignored, zero failed across 73 harnesses. The committed correction is `52d0c39`;
CI `41888724160` remains independently live for earlier head `fb87ec8` at this
observation. No native or new repetition acceptance is inferred.

### S5 Percentile Contract And Exact Prior CI

Exact earlier head `fb87ec8` completes CI `41888724160` at 03:13:37 with all
20 jobs successful. The existing watcher exits 0 without a restart or duplicate
dispatch. That green does not certify subsequent PCM-readiness or S5 source.

The [S5 offline percentile correction](2026-11-05-s5-percentile-target.md)
reproduces an acceptance gap: the old evaluator accepts p99 = 15 ms when p95
passes, despite the current goal requiring both percentiles below 10 ms. Actual
library and CLI assertion REDs become green after enforcing both percentile
targets plus the existing calibration uncertainty, with strict headroom.
Historical p95-only receipts are not relabelled. Fresh Windows/Linux stable
qualification tests pass 26 each; full probe tests pass 741 with 1 ignored on
Windows and 583 on Linux. Both strict audio Clippy checks, workspace all-target
check, non-test build, real PowerShell wrapper and fmt pass. Full workspace/
headless regression completes at 03:19:53 with 2625 passed, 10 ignored, zero
failed across 73 harnesses. Original 29 draft/cache hashes remain unchanged.
No native waveform, sustained, physical or device-limited verdict follows.

### Published Percentile Checkpoint And PCM Receipt Repair

The normal batch push publishes `919d10f`, including the prior PCM readiness
and S5 percentile corrections, after the earlier CI is terminal green. Remote
hash and PR26 Draft/open are verified. Its original split-before-merge prefix
is preserved verbatim; new matching-head CI `41896388334` is discovered rather
than manually dispatched. That run remains live, not a pass at this point.

Independent review then reproduces two actual
[native-PCM audit receipt defects](2026-11-05-s1-pcm-refusal-persistence.md).
An output flush failure leaves a successful structural summary despite CLI
exit 2, and an overflowing JSON number can truncate the refusal receipt.
Clean assertion REDs (32 pass, 2 fail) become 35 passing tests on both Windows
and Debian after finite decoding and prepublication all-map finalization.
All 17 Python suites pass 321 tests. The first owner's persistence refusal
now stays visible while the next owner is still audited. Independent final
two-file review finds no blocker; code is committed as `0523359`, not pushed
over the existing live CI. Original 29 draft/cache hashes still match.
No real PCM/OBS acquisition, refit, remote raw transfer, native operation or
waveform/physical qualification follows; all full-pipeline gates remain.

### Current Release Probe Preparation

Both ordinary optimized examples build from clean `73e1fc2`: VAD integration
7806976 bytes and E2E latency probe 7729152 bytes. Exact source/tree and binary
SHA-256 pins are retained in the
[release readiness receipt](2026-11-05-release-probe-readiness.md). The actual
release E2E executable passes the existing PowerShell 7 synthetic offline
qualification regressions; the VAD integration binary is not executed. Neither
artifact is deployed or presented as native acceptance.

Fresh strict-known-host SSH at host 03:36:14 confirms the expected target/SID,
session 0 and an administrator token. A corrected literal-path check confirms
the planned audit package remains absent; an earlier Python path-escape error
and its limited receipt are retained. The non-admin foreground metadata-reader
guard remains unsatisfied and unchanged. No remote deployment/ffprobe/task or
audio/device/UI/driver operation follows. VMware inventory remains zero running.
The published `919d10f` CI is still live and is not superseded by local work.

### Streaming Pre-Client Admission Repair

The [pre-client admission receipt](2026-11-05-s4-pre-client-admission.md)
reproduces a real coordinator/provider mismatch: replacement is checked before
the new Pair is prepared, but admission rejects the legal zero stream generation
of an endpoint without a native client. Three assertion REDs become 138 Windows
and 117 Linux stable example passes. Admission now accepts that prepared state;
final PCM and the old completed owner still require nonzero stream generations.
Identity, gate epochs, strict 48,000-frame PCM, freshness and deadlines remain.

Windows app all-target and Linux example strict Clippy, workspace all-target
check, non-test example build and fmt pass. Full workspace/headless with both
opt-in features finishes at 03:48:36: 2625 passed, 10 ignored, zero failed across
73 harnesses. Independent review finds no actionable blocker. All four code
pins match and the original 29 draft/cache pins remain unchanged. The existing
published CI is still live; this local repair has no matching-head CI yet.
The earlier release VAD example predates it and is not deployed or relabelled.
The corrected example is then rebuilt from clean `ee25064`, with source/tree,
length and SHA-256 pinned to an independently verified immutable evidence copy.
It is not executed or deployed; the earlier shared-target pin stays historical.

### Readiness/Percentile CI Closure And Published Batch

The existing exact-head `919d10f` run `41896388334` completes at 03:59 with
all 20 jobs successful, including Windows/Ubuntu tests and strict Clippy,
driver host, all Miri suites and the aggregate. The retained watcher exits
zero; there is no rerun, duplicate dispatch or premature superseding push.
Final API and watcher receipts are archived. This closes CI for the earlier
PCM-readiness/strict-percentile source, not for later `0523359` PCM-audit
persistence or `ee25064` pre-client admission corrections. Those verified local
slices are normally batch-pushed in `fc05652` at 04:00:50. Remote hash and
PR26 Draft/open are verified, preserving the entire prior body verbatim before
appending the new checkpoint. Matching-head CI `41902537510` is automatically
discovered and watched without dispatch/restart; it remains live.

A bounded S3 source/artifact audit finds no new product-code defect or unused
complete native acceptance package. The original stopped-input combined
child/obstruction exception cannot identify its hit. A later frozen
`s3-input-boundary-20261103-0040` observation explicitly records a different-PID
`LockScreenBackstopFrame`, with `SameOwner=false` and `HitIsChild=false`.
That identifies external obstruction for that historical run, not current
desktop readiness. The strict input guard remains appropriate; no identical
retry, child-hit relaxation or manufactured new diagnostic follows. The
metadata reader's own non-admin requirement is not a blanket desktop blocker.
Retained Evaluate's actual overlapping event envelopes remain a separate
refusal, not fixed by desktop accessibility. Fresh 03:58 VM inventory still
returns zero running VMs.

### Current Desktop Readiness Observation

The [fresh observer receipt](2026-11-05-desktop-readiness.md) uses only path/task
adaptations of the existing read-only session1 observer, with explicit PS5/PS7
parse results and transferred script pins. Its one new Limited/Interactive
45-second task finishes Ready/result0, with no remaining observer process.
The 330-byte receipt is independently remote/local length/hash-verified.
Protected OBS/QQMusic creation identities and both script pins remain unchanged.
No app, audio, input, foreground/window change or driver/device exercise occurs.

The current foreground is the Windows default lock-screen interface on the
Default input desktop. This is current obstruction evidence, not a WTS lock
flag or input-consumption result. No unchanged UI trial or capture follows.
Native desktop acceptance needs an unobstructed interactive target; the
metadata reader and stopped driver VM remain separate prerequisites.

### PCM/Pre-Client CI Closure And Terminal Reader Handoff

Exact published `fc05652` finishes run `41902537510` with all20 jobs successful;
the last Miri aggregate completes at 04:22:49 GitHub time converted to +08:00.
The existing watcher exits0. This certifies the published PCM-audit and
pre-client repairs, not native or physical acceptance. The later local
readiness/handoff documents do not change the tested product/probe source.

The [terminal handoff receipt](2026-11-05-terminal-audit-handoff.md) separates
the already-authorized two-file placement from guarded reader execution. A
reviewed stage-only wrapper creates the exact fresh remote package once and
SCP-transfers only the unchanged script and manifest at 04:30:28..04:30:33.
Both remote lengths/hashes match, exactly two regular files are present, and
all32 original local package pins still match. Placement records admin=true,
session0 and the exact SID. It stops without reader/ffprobe invocation, original
input reads, executable/raw transfer or token/ACL/task/native changes.

The reader still requires an actual exact-SID non-admin token. Its existing
foreground contract is synchronous execution and explicitly permits session0;
the desktop lock-screen observation does not independently block metadata
reading. The old combined deployer is not rerun; its frozen non-admin placement
guard remains unchanged. No terminal padding/PTS or waveform explanation is
claimed before the actual guarded audit. The staged files give the permitted
token a concrete existing command rather than another missing tool.

### Readiness Waits Preserve Both Route Exports' Errors

Existing exact-head CI `41907994546` on `ae60e9b` finishes all20 jobs
successfully, including the aggregate. The original watcher exits0; terminal
API and watcher receipts are archived without restart or duplicate dispatch.
Its host completion and GitHub completion timestamps stay separate. This
green result does not certify the subsequent wait correction.

A bounded source audit finds another existing sticky-error omission: the
streaming coordinator's three readiness-only waits could discard a current
error before a later successful snapshot. A first correction observing only
the renamed export still missed a peer QUERY error after global graph error
clearing. Both paths have genuine portable assertion REDs. The actual wait
now projects both configured IDs from the same snapshot, rejects pre-request
evidence, retains explicit errors before readiness, and publishes the Case
before returning. Existing PCM/name/Removed/cleanup/deadline gates stay intact.

The [wait-error receipt](2026-11-05-s4-wait-error-retention.md) keeps the two
RED stages, superseded selected-only verification, final-source pins and
remaining acceptance separate. Final Windows/Linux example suites pass145
and124 respectively; current full verification is recorded in that receipt.
The A -> B -> A mode is still only a proposal awaiting direct design approval,
not an implemented feature or a native acceptance result.

The scoped repair is normally pushed as `8761d07`; the remote hash and
PR26 Draft/open state are verified without replacing its previous body.
Matching-head CI `41915822264` is automatically discovered and observed by
one watcher. A clean-source optimized example then completes at05:27:39;
its 7,819,264-byte immutable copy hashes to
`1BFE06492DB7665EEA54E30EA5E9F9DA34265C6FC09514A7FF0883188B991DD9`.
The [release record](2026-11-05-release-probe-readiness.md) retains the exact
source/tree, command, terminal exit0 and independent copy verification.
It is not executed or deployed. The later artifact documentation stays local
while that existing CI is live; no superseding push or duplicate run follows.

### Both-Route CI Closure And Remaining Entry Conditions

Exact `8761d07` completes run `41915822264` with all20 jobs successful.
The aggregate's GitHub completion is06:01:19+08:00; the original watcher
finishes at06:01:24 host time with exit0. Final API/watcher/closure receipts
are retained without dispatch, restart or superseding push. This certifies
the both-route wait correction, not native or physical qualification.

The existing token/package preflight is adapted only to create fresh local
logs. At06:02:37..38 it verifies the exact host/SID, session0, admin=true
and the already-staged package; no original inputs, reader or ffprobe are
opened. The exact permitted non-admin reader token is still unavailable.
Current read-only VMware inventory remains zero running. Ordinary desktop
retained acceptance, S4 native/migration/name/default-role/held/race/repetition/
sibling acceptance, sustained quality and calibrated physical p95/p99 stay open.
The unimplemented round-trip proposal still needs direct design approval.

Later artifact and closure documents stay local; no new CI is triggered
solely to record the previous run's success. There is no test/native retry,
default promotion, VAD/Process winner or full S0-S5 completion claim.

One post-CI [bounded desktop observation](2026-11-05-desktop-readiness.md#post-ci-readiness-revalidation-at-0608)
then checks whether the S3 entry condition changed. A path/task-only adaptation
of the existing observer parses on explicit PS5/PS7, runs once, and closes
Ready/result0 without a remaining observer. Script pins and protected foreign
process identities match. The330-byte remote/local receipt again records
the Windows default lock-screen interface in the foreground, not a WTS flag
or input result. The48payload-plus-manifest archive is independently verified.
No app/audio/input/driver activity follows; the ordinary retained desktop
acceptance remains open. No unchanged re-observation is planned absent an
external readiness change.

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
