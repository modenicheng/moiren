# Managed removal probe admission

Date: 2026-11-08, Asia/Shanghai. This connects the retained task preparation
to a real worker/probe admission protocol. No user reboot confirmation has
been received. No remote command, native audio, OBS, ETW, scheduler mutation,
driver installation/restart or system reboot is performed by this slice.

## Actual entry and ownership

The example `e2e_latency_probe` accepts an opt-in `managed-removal` prefix.
Before entering even its logging/normal CLI, it opens controller and worker
handles with query/synchronize rights, checks exact creation times and live
status, and publishes `probe-ready.json`. It permits only a nested `live-vad`
command. Admission requires the exact matching `probe-admit.json` within at
most 15 seconds and a final live-owner/time recheck. Ordinary CLI arguments and
exit codes are unchanged. No suspend/terminate/relaunch path is introduced.

Both receipts use schema 1, nonce, exact probe/controller/worker PIDs and numeric
DateTime UTC ticks plus actual probe token session. Each is at most 8192 bytes
and published create-new without overwrite. This is correlation, not a security
boundary against another same-user process already able to write the directory.

The collector in `scripts/vad-removal/collector-boundary.ps1` stages only five
small scripts and a pinned run configuration. It never copies the probe or OBS
executable. The actual worker action validates source/configuration pins, holds
the original child Process and jointly capped stdout/stderr, and reports its
exact exit. Independently, the collector binds the waiting live probe through
its first retained Process, creation, executable, token/SID/session, parent PID,
exact command line and disk hash before it writes admission.

The task's scheduler EnginePID is never treated as the worker. Full terminal
composition takes the retained task owner and its current observation and
invokes the existing worker check internally. It also requires independently
observed probe exit to match the reported probe identity/code. A failed native
exit can be terminal ownership; it never sets `NativePassed` or `Qualified`.

## Failure behavior

The worker uses a separate 120-second observation budget; the frozen generic
tool remains capped at 15 seconds. Neither changes the planned eight-second
acquisition or unchanged 20-second direct-Removed gate. On deadline the worker
writes one pending receipt and retains/observes the same child without kill or
restart. The collector samples retained owners even without a result file.

Review reproduced and repaired an output-drain snapshot race, detached task
proof, missed observation without a receipt, and post-launch receipt/identity
failure paths. Ambiguous initial dispatch retains the original Process and
observes it; it never reacquires by PID or launches again. A failed admission
does not authorize another admission or stop observation of existing owners.

## Verification and evidence scope

The parent independently runs all Windows example tests: **755 pass, 1 ignored**;
the managed subset passes **14**, including actual harmless test-executable
children for exact process times/session, natural exit, timeout, mismatched ack,
owner exit and admitted stub execution. Strict example Clippy and workspace
format checking pass. The native CLI closure is never invoked by these tests.

The PowerShell suites cover real worker and harmless probe processes under both
Windows PowerShell 5.1 and PowerShell 7. Scheduler and target host are fixtures,
not actual scheduled tasks. Independent process/token/CIM reads, bounded output,
admission, natural error exit and full ownership composition are exercised.
Focused RED/GREEN receipts are retained for the failures above. The production
worker loader passes12actual-entry checks per interpreter; these are not target
admission. The current total is107checks per interpreter, including10checks
with real harmless task-worker/probe children, not scheduler mutation.
An additional13checks per interpreter exercise the actual Rust test executable
ready receipt through PowerShell's bounded parser, original-handle binding and
admission writer. The Rust admitted closure only writes a stub marker and
returns1; libtest's0exit is not native success. No native release EXE is used.

The preceding published head `4c0d8c0` completes CI run42446894528 with all20jobs
successful at13:05on November8. Its original watcher exits0. That result covers
the preceding source, not this later managed-admission slice.

Parent logs reuse `D:\coding\moiren-evidence\20261108-112100-collector-worker`.
Rust implementation evidence is in
`D:\coding\moiren-evidence\20261108-managed-removal-admission`, including its
source manifest and Windows/Linux-check/isolated-Debian logs. An isolated
portable Debian suite is not a full Ubuntu workspace acceptance claim.

## Remaining native pipeline

Task registration/dispatch and default trace launch still unconditionally
refuse. Native dispatch requires fresh post-user-reboot provenance, reviewed
collector/privileged trace/Limited worker binding and independent ETW session,
provider/output readback. Raw loss, decode/report association, safety audit,
archival, authorized trace/task cleanup and actual kernel removal diagnosis
remain open. The managed binary needs a new release/pin; the historical probe
and installed diagnostic 613 package are not relabelled or replaced.

This is not durable controller-crash recovery, an actual removal repair, naming
qualification, a waveform/physical latency verdict or completion of S0-S5.
The separate remote target remains direct SSH, not a mandatory VM. Only normal
visible-input testing needs an unobstructed interactive desktop. Reboot remains
the user's action; the fixed dedicated OBS path remains unchanged.
