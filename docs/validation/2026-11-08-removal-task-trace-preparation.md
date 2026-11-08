# Retained task and trace preparation

Date: 2026-11-08, Asia/Shanghai. This extends the
[managed collector](2026-11-08-removal-collector-worker.md) without changing
the frozen predecessor. No user reboot confirmation has been received. No
remote command, actual scheduled task, ETW/audio/OBS, driver install/restart or
system reboot occurs. All original S0-S5 requirements remain open.

## Repository implementation

`scripts/vad-removal/` now retains the real finite-tool definitions and task/
trace preparation with host-only tests, rather than leaving executable work
only in external evidence. The finite tool retains its exact frozen SHA256
`C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B`.
Trace's default helper and composition-test paths are now adjacent relative
paths; the external predecessors are not overwritten.

Registration intent and Scheduler/folder/definition are retained on a caller-
owned object before TASK_CREATE may commit. Unknown registration/dispatch is
sticky and never authorizes another attempt. RunEx retains its exact instance;
EnginePID is never adopted as the worker. Real worker binding holds the first
Process and independently checks creation, session, image, command line, file
pins, SID, elevation and administrator membership through its original handle.
Task last-run/state/instances are read from the retained COM task.

Behavioral RED/GREEN reproduces unknown-registration ownership loss and a false
terminal path that previously accepted detached JSON without a real worker.
Known scheduler/worker exit now produces only WorkerTerminal. Full Terminal,
ProbeOwnershipVerified and qualification stay false: actual native probe-child
admission/lifetime ownership is not wired. A reported probe exit remains
ReportedProbeExitCode. The real registration/dispatch guard always refuses;
changing a mutable approval field cannot activate it.

Trace preparation composes exact finite-tool owners for inventory/create/raw
loss/stop, each once with 15-second observation and 1-MiB redirected-output cap.
It keeps the fixed provider/flags/level/perf clock/16-MiB ETL contract. Garbage,
truncated or unrecognized inventory cannot prove absence. Only the complete
reviewed English table fixture is admitted; the actual target format is not
qualified. Exact successful create is a command-result ownership basis, not
independent session/provider/output readback. Raw loss remains unknown even
when tracelog exits0. A loss error does not suppress one confirmed-owned stop.
The real native launcher, including default stop, remains refused. A future
runner must separately authorize cleanup of its confirmed-owned session.

## Verification and limits

Parent repository-path reruns pass on both WindowsPowerShell5.1 and PowerShell7:

- finite tool16; task mocks17; real harmless worker/token/CIM binding4;
- trace mocks20; actual finite-tool/harmless-child composition2;
- total59 checks per interpreter, without scheduler/native operations.

The independent reviewer reruns task17+4 on both engines and reviews trace
ownership, repository relocation and CI wiring. A live-worker test assertion
gap is repaired before final verification. The Windows CI step runs55
noninteractive checks per interpreter; the4 real-worker checks require an
existing nonzero, non-elevated local session and are intentionally not claimed
as hosted-runner evidence. Workflow YAML parses; hosted CI for these new
changes is not yet observed.

This is not a runnable approved diagnostic or a removal repair. Real worker/
probe handshake, fresh post-install boot/provenance, collector/privileged trace/
Limited worker session binding, bounded receipts, independently confirmed trace,
raw loss/decode/archive and complete no-owner terminal admission remain open.
COM/CIM/file/hash calls are synchronous; no hard cancellation, durable crash
recovery or atomic safety snapshot is claimed.

## Evidence and storage

Evidence reuses `D:\coding\moiren-evidence\20261108-112100-collector-worker`.
All326members of `host-only-task-trace-supplement.zip` independently verify
length/SHA256. The217402-byte archive SHA256 is
`E43BFA122FD0AED183024908A4D4582056036E2B8754AF3B2D0E4CD6665E0771`.
Its manifest pins118supplemental evidence files; repository source/test outputs
are also archived. The12frozen collector and29original draft/cache pins remain
unchanged. No OBS executable tree is copied.

Removal of verified owned test fixtures was rejected by the execution policy;
no alternate shell/language deletion is attempted. Those small closed fixtures
remain, explicitly not a completed cleanup. No test task or native process was
created to retire. Existing unrelated caches and original worktree drafts are
preserved.
