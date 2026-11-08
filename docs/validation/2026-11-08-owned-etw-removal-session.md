# Owned ETW removal-session composition

Date: 2026-11-08, Asia/Shanghai. This is a guarded implementation checkpoint
toward the S1 removal diagnosis, not an actual target capture, removal repair,
zero-loss assertion, physical measurement or S0-S5 completion.

## Production chain

`trace-native.ps1` owns a fresh session GUID and retains the exact successful
StartTraceW handle before provider enablement or readback. Failed/ambiguous
start output is preserved but grants no adoption or cleanup authority. The
native owner does not adopt by session name, retry a failed STOP, close an ETW
consumer handle, or issue cleanup from a finalizer.

`trace-query.ps1` implements actual ControlTraceW QUERY and classic WPP
EnumerateTraceGuids parsing. It queries the retained session, enumerates the
exact provider, queries the provider's full ULONG LoggerId and queries the
original session again. All three identities/configurations must agree.
Provider level and flags retain their native uint32 types. The classic layout
is distinct from modern TRACE_ENABLE_INFO; no unsupported 16-bit LoggerId cap
or guessed handle-bit extraction is used. Bounded buffers, string offsets,
UTF-16, enumeration counts and nondecreasing counters are checked.

`trace-readback.ps1` freezes copies of independently observed initial/latest
snapshots. `trace-session.ps1` composes start/enable/query/admission and exact
cleanup. Both native and query mock boundaries must be complete and supplied
together, so a partial mock cannot fall through into real ETW.

The managed collector retains that exact trace session/readback and refreshes
it immediately before publishing probe admission. Cached initial readback is
not sufficient. A missing/changed/errored fresh query makes admission invalid;
the collector continues observing the already retained worker and probe.

## Final STOP evidence

Before STOP, the native owner queries and verifies its exact retained handle,
GUID, session name, ETL path, QPC clock, sequential mode and 16-MiB cap. The
original STOP receipt is retained once, even when STOP fails or is pending.
Successful returned properties are copied into read-only scalar fields before
publication. Dispatcher-only, stage-bound, one-shot capture prevents later
replacement of the first status, identity or final payload.

- Status0 can establish final counters only from the complete returned STOP
  buffer, exact owner/receipt and counters not below either pre-STOP or latest
  independently observed values.
- Status234/MORE_DATA establishes stopped, but final counters remain unknown.
- Status2402/ACTIVE_CONNECTIONS retains a pending owner; no retry or final-loss
  assertion follows. Other failure codes are not reclassified as this status.
- No query of a stopped session or still-enabled provider is required to
  establish final STOP counters. Nonzero loss is preserved, not normalized.
- An equal-looking cloned receipt, mutable pre-STOP DTO or caller-supplied
  success flag cannot replace the original retained native receipt.

The provider-idle check before enable is not an atomic reservation. Classic
provider ownership is exclusive; the future combined runner must exclude
competing controllers. This is internal same-process correlation, not a
hostile-process authorization or durable controller-crash recovery boundary.

## Verification

The parent runs all13 Windows CI preparation suites under fresh Windows
PowerShell5.1 and PowerShell7 processes: **168 checks per interpreter, all pass**.
Of these, the new query/readback/native/session/composition suites account for
19+10+24+7+9=69 checks per interpreter. Actual C# native layouts, Marshal
buffers, parser, ownership, readback and final receipt are exercised with only
API delegates replaced. No host ETW, scheduler, audio, OBS, remote device or
kernel driver operation occurs.

The nine full-composition cases include fresh query error5/throw, stale provider
refusal, sticky invalid admission with exact owned cleanup, nonzero final loss,
final counter regression against a later query, MORE_DATA, pending2402 without
retry, equal-looking receipt rejection while initially valid and final snapshot
setter refusal. Focused behavioral RED/GREEN receipts cover the earlier native
type, receipt, one-sided boundary, fresh-admission and mutable STOP failures.
Independent static review clears the final guarded native owner and composition;
it is not permission for a target run.

Evidence reuses
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.
`trace-final-test-receipts.json` pins the26 final command logs. The older command
trace boundary is retained as historical guarded preparation, not relabelled as
an independently observed native session.

## Remaining target pipeline

Real task registration/dispatch and native trace-owner dispatch still refuse.
Complete combined preflight must bind fresh post-user-reboot host/boot/SYS
provenance, privileged trace controller, Limited worker and pinned managed
probe, competing-owner absence, bounded collection, exact closed ETL/hash,
matching613 TMFs, decoder/report association, safety readback and authorized
task cleanup. A fresh managed release/pin is still required; the frozen old
probe and installed613 package are unchanged.

The user owns reboot. Use the standalone remote test computer directly over
SSH, not a mandatory VM. No OBS or unlocked desktop is required for the planned
eight-second removal diagnostic. Keep its existing20-second direct Removed
gate. The fixed dedicated OBS executable path is unchanged for later waveform
tests; no EXE tree or firewall/security policy is copied/changed here.

Owned host-test fixture deletion was previously policy-rejected. These closed
small fixtures remain; no alternate deletion mechanism is attempted and no
complete-cleanup claim follows. Unrelated caches and original-worktree drafts
remain untouched.

## Primary contracts

- <https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew>
- <https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-controltracew>
- <https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-enumeratetraceguids>
- <https://learn.microsoft.com/en-us/windows/win32/api/evntrace/ns-evntrace-trace_guid_properties>
- <https://learn.microsoft.com/en-us/windows/win32/etw/configuring-and-starting-an-event-tracing-session>
