# Bounded VAD removal-stage diagnostics

Date: 2026-11-07, Asia/Shanghai. This follows the failed runtime21 cleanup in
the [paired waveform receipt](2026-11-07-matched-waveform-cleanup.md).
It closes a source-supported observability gap, not the native cleanup defect.
The full S0-S5 pipeline and all original acceptance gates remain open.

## Evidence gap and unchanged behavior

The retained native result has a joined WASAPI source, Closed Engine and an
exact Removing/Closed VAD snapshot, but no ReleaseHardware result or final
destruction sequence. Existing612 cannot supply the missing events after the
fact. Returning a failed `AcxDeviceRemoveCircuit` status without retaining it
does not establish that the API failed in runtime21.

The new collector records six stages: root Remove entry/return, audio
ReleaseHardware entry/return, child Cleanup entry and child Destroy entry.
There is no ABI, endpoint identity/name, status/snapshot, gate, timeout, retry,
ownership, native mode/period, queue or default-policy change. Actual release
return values and circuit/pin ownership remain unchanged. No delay, allocation
or logging is added to PCM/timer/Run/Pause callbacks.

Each endpoint gets560bytes in the existing nonpaged shared allocation. The
shared spin lock orders fixed scalar records containing exact runtime/session,
sequence, QPC/frequency, stage count, first/last observation and sticky first
failure. Status presence and whether a native remove call actually occurred
are independent fields. Counter exhaustion is explicit, never wraps. No
handle, pointer, PCM, user name or path is recorded.

Control QUERY copies/drains pending evidence under SharedLock, then emits WPP
at PASSIVE_LEVEL outside the lock. Disabled tracing does not drain evidence;
Destroy never enters WPP/pageable code. Each fragment repeats runtime/session;
a single export has at most25fragments, including a possible overwrite notice.

CREATE may reuse a destroyed slot before a later QUERY. Independent review
found that an initial reset silently erased pending old evidence; a new test
reproduced that failure. Reset now retains explicit cumulative lost-lifetime
count, last discarded identity/session/sequence/stage mask and saturation.
`Removal overwritten` reports that loss without reconstructing missing events
or delaying reuse. A drained loss notice remains provenance, not a native pass.

## Tests and build

Evidence: `D:\coding\moiren-evidence\20261107-233656-removal-diagnostic`.

- Production-extracted ReleaseHardware regression first fails because entry
  evidence is absent, then passes four failure/success/unpublished cases.
  It checks the original NTSTATUS, unlocked ACX call, exact stop/delete count,
  retained failure ownership and cleared success handles.
- Bounded-state regression first fails for missing implementation, then passes
  unknown stages, failure/retry, runtime isolation and saturation. The separate
  unexported-reuse red case passes after explicit loss provenance is added.
- Production-extracted collector/export regression first fails for the missing
  exporter, then passes DISPATCH-safe collection, passive unlocked WPP, disabled
  retention, exact identity/status, query idempotence, slot reuse/loss export
  and worst-case25fragment bound.
- Existing production REMOVE/reference-lifetime tests now also verify real
  removal failure, idempotent no-native-call return and Cleanup/Destroy hooks.
  They still manually supply framework callbacks, not native PnP timing proof.
- All21MSVC host suites pass; all16portable-script suites pass with GCC on
  Windows/MSYS2. This is not a Linux native/CI result. The pure pause fence
  retains54scenarios/0failures; default-off PCM observer paths remain unchanged.
- WDK Analyze build, catalog generation and native InfVerif pass with no
  warnings/errors. TracePDB generates matching TMFs containing all three new
  event schemas. That proves format metadata generation, not live trace loss.

Early development failures are retained: kernel CRT include collision from
using host `stdint.h` is fixed by reusing the existing kernel-safe ABI types;
the production reserve extraction fixture gains the new diagnostic array.
Signed status bits are stored explicitly as uint32 and reconstructed only for
WPP formatting. The first MSYS shell lacks its tool PATH; the corrected scoped
PATH run is separate. No warning suppression or test/gate removal is used.

## What the records can and cannot prove

- RemoveReturn success proves only the actual call's returned status; an
  idempotent return has native_callfalse. Neither proves final Removed.
- Release entry with no return can include pre-ACX quiescence; it cannot locate
  a stall specifically inside `AcxDeviceRemoveCircuit`.
- Cleanup/Destroy markers mean callback entry, not callback completion. The
  direct QUERY snapshot must itself say Removed before acceptance.
- First/last/count preserve bounded history and failures, but do not pair every
  repeated attempt. Loss/saturation or missing trace records limit conclusions.
- Native WPP collection must keep the exact control owner/runtime after source
  Stop/join and Engine closure, forbid intervening CREATE/reuse, retain the
  unchanged deadline, then flush/decode the owned trace with loss evidence.

At this checkpoint there is no installation or native acquisition. The23:52:47
read-only direct-remote check still reports Running612 with the same SYS hash,
protected process identities, no owners/present raw children/running test tasks.
All29original dirty/cache pins remain unchanged. A separately pinned package,
rollback/safety admission and finite remote diagnostic are needed next; this
does not require VM startup or visible desktop unlock. PR26 stays Draft/open.
