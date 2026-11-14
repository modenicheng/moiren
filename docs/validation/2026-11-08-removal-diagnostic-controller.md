# Retained diagnostic controller and safety-after composition

Date: 2026-11-08, Asia/Shanghai. This follows the
[read-only target readiness](2026-11-08-removal-target-readiness.md) and
[managed finish](2026-11-08-managed-removal-finish.md) checkpoints. It advances
the S1 diagnostic pipeline, not driver removal repair or S0-S5 acceptance.
The user has not confirmed reboot. No remote collection, driver/device action,
native audio/ETW/scheduler operation, OBS launch/copy or reboot occurs here.

## Actual software entry and pinned inputs

`scripts/vad-removal/invoke-diagnostic.ps1` now composes the original lifetime:
writer lease, BEFORE target readiness, managed script staging, trace ownership,
task/worker/probe observation, finish/decode/retirement, AFTER safety collection
and a readback-hashed root result. Missing user reboot confirmation or wrong
target host/SID refuses before imports or files. The entry requires an already
elevated controller; it never elevates or reboots automatically. Lower native
registration/RunEx, ETW, decoder and retirement approval guards remain closed.
This is not an approved native runner or a new installed image.

`diagnostic-lease.ps1` retains a real ReadWrite FileStream with FileShare.Read
at `D:\MoirenTest\removal-613-controller.lock`. Competing writers are excluded
by that live handle, not by PID text or an abandoned lock file. The file is
reused after close, never rotated/deleted to steal ownership. The original
controller identity governs release of its own retained handle; failed
admission or changed bytes cannot authorize closing a foreign replacement.

`run-inputs.ps1` requires an independently supplied manifest SHA256. It hashes
every production script, rejects missing/unlisted dependencies, and retains
the exact managed26cd706 release, signed613 package, matching PDB/TMF set,
original source schedule and tool identities. Its diagnostic argv preserves
the stable waveform endpoint/name, shared source480/render336, target512,
engine/wake/callback64, eight-second acquisition and unchanged direct20-second
Removed gate. These arguments are not a new measured latency claim. Actual
ETL/report pins are constructed only after original STOP and managed terminal
ownership; input mutation cannot be repinned into acceptance.

## Historical baseline and actual AFTER collection

`safety-audit.ps1` binds the original completed/persisted BEFORE evidence before
native birth. It does not require renewed120-second admission freshness or a
still-live Explorer to inspect AFTER. Audit-mode preflight can retain nonempty
native-owner, pending-task and raw-child observations while keeping Ready=false.
It compares host/SID/controller session, boot, driver binding, exact external
process identities, six default roles, render controls and security state.
Missing/replaced/added external identities are unsafe observations, not
permission to stop/reopen them. Malformed/failed observations and refusal
reasons are preserved in an unsafe receipt, not discarded as empty success.

SignTool/catalog/BcdEdit identities now pass from original inputs through
readiness/preflight into the actual collector runspace as detached JSON.
Each collector validates exact paths, lengths and original expected hashes
before its first tool launch; each finite dispatch checks its expected hash
again. Standalone repository preflight freezes the three identities at owner
construction. AFTER inherits that same JSON. It cannot adopt whatever bcdedit
happens to be present later. Changed live BEFORE pins are evidenced as unsafe
after collection, rather than suppressing the original AFTER invocation.

## Failure ownership and retained evidence

Timeout closes new admission but never kills/relaunches original owners. The
root observes pending original BEFORE/AFTER collector runspaces and finish
decoder children before refusing changed outer references. A late original
worker/probe receipt may still bind the actual process for natural-exit
observation after a root error; it cannot write new audio admission.

Unknown registration/RunEx/Start/deletion stays unresolved without retry or
by-name adoption. A captured nonzero uint32 Start error is known unsuccessful,
not ownership of an existing trace; no STOP follows. Successful Start with
failed enable/readback retains exact-handle/GUID self-owned STOP independently
of current admission. Existing finish ownership cannot be bypassed by a root
abort to skip pending tools, changed input/evidence refusal or uncertain task
retirement. Confirmed never-dispatched tasks have separate archive-before-delete
retirement; already retired tasks are not deleted again while AFTER is pending.

The root keeps its writer until the actual AFTER result is complete and its
original receipt is verified, then persists/flushed-readbacks the root result
before release. Missing AFTER/result persistence stays pending. NativePassed
and Qualified remain false even when orchestration is safe. A completed unsafe
or diagnostic-refused orchestration is not a native cleanup or quality pass.

## Verification scope

Evidence root:
`D:\coding\moiren-evidence\20261108-160558-diagnostic-controller`.

The controller boundary suite exercises ordering, pending ownership, original
receipt persistence, no-retry deletion and known-failed versus unknown Start.
It mocks lower APIs; it is not actual scheduler/process acceptance. Separate
lease tests exercise actual filesystem writer exclusion. Input tests use real
file hashing with small synthetic content constants; entry tests execute the
byte-identical isolated production entry under each interpreter and prove
guard refusal before imports/files.

The lifetime suite uses the actual root controller, real FileStream lease,
actual retained synthetic BEFORE/AFTER collector runspaces/receipts, managed
script staging and production C# trace-owner/Marshal buffers with injected API
delegates. Start0/Enable5 must produce exactly one original self-owned STOP,
actual safe AFTER evidence and a persisted failed root result before release.
A second case keeps an actual harmless timed-out child and both output drains
through root timeout and public trace/readback replacement; only root
observation may satisfy natural-exit/closed-owner assertions. The deliberately
changed root remains incomplete; fixture teardown then releases its own lease.
Negative controls that omit original lease release or original AFTER
observation fail these tests. No scheduler, ETW or audio API is invoked.

The parent final matrix passes all30CI preparation suites,370checks per
PowerShell5.1and7, with source hashes unchanged across each full run. Separate
actual harmless task-process/worker-process checks pass10checks per shell.
The offline reader passes29tests; all PowerShell files parse, workflow YAML,
rustfmt, diff whitespace and58CI-surface mapping checks pass. All six new
suites are wired into the existing Windows CI step under both interpreters.
Final matrix/log pins and independent archive-member readback are retained in
the evidence root; the archive sidecar records exact length/SHA256.

Predecessor b3bc3ea's original CI42459303613 watcher exits0 and its exact head
finishes20/20jobs successfully. That result covers the predecessor, not this
new controller delta.

## Remaining completion gates

The later [owner-aware operation preparation](2026-11-08-removal-native-admission.md)
adds resource-bound lower guards while its executable release remains closed.
Reviewed actual target orchestration,
bounded closed-run archive/transfer and fresh post-reboot target provenance
remain before a finite native diagnostic. Current guards deliberately do not
accept generic Reviewed/NativeApproved booleans. Cleanup authority must remain
bound to original resource identity, not expired capture admission.

After explicit user reboot confirmation, use the standalone remote target
directly; no VM or blanket unlocked-desktop requirement is introduced. Retain
the fixed dedicated OBS installation for any later acquisition, not per-run
EXE copies. The installed613diagnostic remains unchanged. Direct Removed,
matched Process/VAD waveform/quality, native naming/default/held/streaming/race
A-to-B-to-A cases, sustained tests and calibrated p95/p99 plus uncertainty
strictly below10ms (or evidenced device-limited fallback) remain open. The
canonical verdict stays in `driver/vad/STATUS.md`.
