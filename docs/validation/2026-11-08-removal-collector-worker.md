# Whole-collector managed ownership and pushed-head CI

Date: 2026-11-08, Asia/Shanghai. This follows the
[read-boundary preparation](2026-11-08-removal-read-boundaries.md). No reboot
confirmation has been received; no remote/audio/OBS/ETW/task/driver operation
occurs. The user remains responsible for any system reboot. All original
[S0-S5 requirements](../vad-low-latency-goals-2026-10-28.md) remain open.

Evidence:
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.

## Managed collection boundary

`collector-worker.ps1` retains one PowerShell managed runspace/IAsyncResult and
shared live .NET references to the preceding collector's exact inner Process
owners. It does not spawn a separate controller process or serialize ownership
away. Parent observation has a finite interval, production maximum120seconds.
Timeout retains the same invocation, never calls Stop/Dispose/relaunch and
cannot later become an accepted result. Close requires invocation completion
and every inner tool owner explicitly Closed. Input hashes are checked before
dispatch and again inside the runspace.

One dictionary snapshot is validated while streaming. Wrong activity flags,
unexpected/multiple output, oversized/deep snapshots and diagnostics refuse
acceptance. Compiled DataAdded callbacks drain diagnostic objects while keeping
only their count. A nonterminating Error cannot coexist with an accepted
snapshot. Unexpected top-level dot-source output is rejected immediately;
EndInvoke also refuses any residual unexpected pipeline output.

Fifteen tests pass on both WindowsPowerShell5.1 and PowerShell7. These include
one actual harmless nested PowerShell child whose timeout retains its original
Process across the runspace boundary, then explicit observation of natural
exit and cleanup. Actual collector invocation on the development host refuses
at exact-host identity before creating target evidence or reading devices.
This is wrong-host refusal, not target/native collection. Behavioral RED logs
for output/error/size/drain/top-level fixes precede final GREEN logs.

The helper SHA256 is
`9922314AD48E30B3FE9CA616EE4867260B8299EA280F2814B7163D9A202FF78B`;
tests SHA256 is
`1D7FA0FFD36F0CDCD658243E2D38B1D24FB86444B18220E6EAC22C53A8A554C0`.
The preceding17read-boundary pins and original29draft/cache pins are unchanged
at11:34. Neither frozen predecessor controller nor copied historical runner
is silently overwritten or reactivated.

An independent reviewer reruns15/15tests in both interpreters with identical
source hashes and no new finding. The12-file preparation manifest pins source,
tests, parent/reviewer logs and terminal CI evidence. All438members of
`host-only-collector-worker.zip` independently verify length/SHA256;
the172358-byte archive SHA256 is
`446EBE74CC812D32A9F97227A08BBCFFADF7E9CC1C20B7D3C8621BEC83CE4B9E`.
No executable tree is copied. Prior local scratch deletion was policy-refused;
this increment does not attempt an alternate deletion mechanism.

## Limits before actual orchestration

This is managed finite observation, NOT hard cancellation, process isolation,
hard memory/resource containment or durable SSH/crash recovery. A hung system
call can outlive observation and later resume read-only work; retain its owner
and stop the pipeline. Synchronous pin/filesystem/dispatch calls themselves
are outside the observation bound. Snapshot acceptance caps do not prevent
large temporary allocations inside a trusted collector. No owner ambiguity
authorizes a retry or clean-state claim.

The next real orchestrator must still bind observed collector/privileged trace
controller/Limited audio-worker identities separately; session0 evidence must
not be relabelled to satisfy the old session1 contract. Exact task/action/
principal/HasRun/probe ownership, confirmed trace start/raw-loss/owned stop,
bounded create-new receipts, fresh postboot target provenance and matching
decode/archive remain unwired. No native approval or removal repair follows.

## Exact pushed-head CI

At11:30, run42440346496 for pushed head
`3f88d896b81f23904b1d94a32f2d7da41110cb12` completes SUCCESS,20/20jobs.
The original watcher32070 exits0; it is not restarted. Terminal job evidence
is saved in `ci-42440346496-terminal.json`. Cache/runner-deprecation warnings
do not turn the successful jobs into failures or prove native acceptance.

This verifies the pushed software/OBS/naming-audit checkpoint, not these
later local documentation commits or any actual613removal/rename/waveform/
desktop/calibrated-latency result. PR26 remains Draft/open; the full objective
is not complete.
