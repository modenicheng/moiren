# Standalone postboot diagnostic and failed-run recovery

Date: 2026-11-08, Asia/Shanghai. This is a failed native diagnostic, not an audio,
naming, removal, lossless ETW or physical latency pass. No automatic reboot,
firewall change, copied OBS installation or ordinary application operation is
authorized by this receipt.

## Reboot and fresh admission

The prior driver installation returned3010 (reboot required). Disk service/PnP
metadata alone did not establish completion of that transition. The user
performs the reboot; fresh target observation verifies boot
2026-11-08T13:26:07.5000000Z, or21:26:07.5local. The current failure below is a
script configuration defect and does not justify another reboot.
Microsoft defines this PnPUtil result as successful completion requiring a
system reboot: <https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/pnputil-return-values>.

The standalone target is `moiren-lan`, host `LAPTOP-NP49RG6H`, Windows22631.
Fresh full read-only admission verifies613Running/Started, PnP OK/problem0,
oem207.inf, the152472-byte SYS SHA256
`0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76`,
package/rollback signatures and catalog membership,14known tool/package pins,
and original Limited Explorer PID10344/session1. Six default roles and render
volume/mute are collected. Running tasks, raw children, native owners and
external owners are empty. The original collector closes normally. This is
fresh service-file/PnP provenance, not an in-memory image attestation or proof
of audio access. No unlocked-desktop assertion follows from SSH availability.

Host evidence root:
`D:\coding\moiren-evidence\20261108-post-reboot-verification`.
The actual preflight result is5662bytes, SHA256
`B4A957BAB03FF0B5B4E991BA99EEBFDEAA11FFE1D7719EE2EC3A541F485FEB93`.
The original actual manifest is10648bytes, SHA256
`079526FD2D7380D7CBC5CB3DDCBC5093751EB004C3819A5194F193F679D44541`.
It belongs to the old source bundle and must not be relabeled as repaired.

## Original failed diagnostic

One original entry starts at14:21:49Z in
`D:\MoirenTest\vad-removal-current\removal613-postboot-01`, nonce
`01ed91943c7d4b269dc8f3a413cba205`. Original BEFORE collection completes. The
trace starts, but no task is registered and no managed probe/audio runs.

The actual raw ETW mode is8388609 (`0x00800001`), not the requested sequential
mode1. Microsoft documents that ETW chooses a hybrid-shutdown default when
neither STOP_ON_HYBRID_SHUTDOWN nor PERSIST_ON_HYBRID_SHUTDOWN is specified:
<https://learn.microsoft.com/en-us/windows/win32/etw/logging-mode-constants>.
Old capture readback and original pre-STOP identity checks both reject the
extra mode. Their exact-once attempt latches leave the controller PendingTrace
after its acceptance deadline; elapsed time is not permission to fabricate
STOP or normal export.

The repair explicitly requests `0x00400001` (SEQUENTIAL plus
STOP_ON_HYBRID_SHUTDOWN) at the marshaled Start boundary and requires the exact
same raw mode through C#/PowerShell readback, pre-STOP/final capture and decode.
Old defaults, real-time and unknown additional bits remain rejected. This is
not blanket masking of unknown configuration, a driver change or a timing
acceptance relaxation. TDD retains the original failing cases, then24focused
suites/296checks pass across PowerShell5.1/7. These are mocked native boundaries.
Independent review retains a P2: persistent different pre-STOP config can still
prevent exact-owner STOP. The existing composition case only corrupts initial
readback while its cleanup query returns the expected mode. The narrow explicit
mode repair does not fix this general failure path. A separate TDD cleanup
repair is required before another native attempt, preserving strict capture
admission and exact retained handle/GUID/name/path identity.

## External recovery, not normal closure

Recovery is independently reviewed, including correction of an initially
insufficient original run-spec pin. Executed recovery is7133bytes, SHA256
`9BC7A340FB8969A44D4E9EF5621E5050FB86B4E0499CFD6709E80A449F23CDB2`.
It retains exact original process handles and the archived run-spec, checks
two fresh handle37/GUID/name/path/config observations, issues one exact-handle
STOP, and requires STOP0 followed by QUERY4201 before ending the failed child.
It neither adopts a session by name nor forges the original controller seal.

At14:34:50Z STOP0/QUERY4201 are confirmed. Only failed controller18452 is ended;
original entry2024 naturally drains and writes its own failed terminal. That
terminal reports exit-1, TimedOut=true, OutputStable=true, no local
acknowledgment, NativePassed=false and Qualified=false. Its1396-byte SHA256 is
`7BBBEDDB12069BD980AB7F57EC1B192BC629BBB8153127B81907331FA7D77AAE`.
The SSH wrapper exiting0 only delivers this pin; it is not diagnostic success.
Neither original AFTER nor a normal closed-controller export is invented.

At14:38:30Z a fresh read-only query finds no original controller/entry, no
exact-nonce scheduled tasks, only the expected Moiren control bus, and handle37
QUERY4201. The failed ETL is65536bytes. SFTP downloads all68closed-run,
original-source and adapter members; every original length/SHA256 matches and
the resulting ZIP is fully read back. Archive:
`failed-native-archive\failed-native-closed-evidence.zip`,183074bytes, SHA256
`FF8F6C9A16FF2536B21233F04480B8381C046CA14AEE4111894342D6196FC989`.
Remote scratch deletion is not yet claimed by this archive receipt.

Subsequent exact cleanup at14:46:11Zremoves35archived failed-run files,
226202bytes, then removes only verified-empty directories. A fresh read-only
query at14:46:44Zfinds the exact run and sidecars absent, fixed OBS hash
unchanged, original reusable probe/source retained and the same boot. No
recursive wildcard deletion, executable deletion or reboot occurs. The
separate59local fixture roots whose earlier cleanup was policy-refused remain
untouched; this remote cleanup is not a workaround for that refusal.

## CI and transport boundaries

Published head04686d6dhas20terminal CI jobs/checks, but run42487741911fails the
Windows test job128753252982. Its15release-fixture cases receive a TEMP root
with a noncanonical dot/short-path spelling. Canonicalizing only the test-owned
root and retaining production strictness passes16checks per interpreter,
32total. This local test repair is not a new-head CI pass.

The host transport also exceeds the Windows command-length limit with a full
encoded diagnostic. A short bounded/hash-checked loader and stdin-lifetime
regressions pass75neighbor tests locally, but actual larger SSH stdin still
stalls. Those fixes are not called a successful remote transport. The original
native attempt instead uses a separately pinned SFTP file adapter invoking
the existing original entry, not replacement ownership logic. A minimal
production file handoff and actual delivery validation remain separate work.

## 2026-11-13 stalled-dispatch follow-up

The repaired explicit-mode attempt is
`D:\MoirenTest\vad-removal-current\removal613-etwmode-02`, nonce
`3992aa8f3cb14474bb67bb30b1007aa3`, with the same boot. It reaches BEFORE
readiness, ETW file creation, managed staging and exact task registration, but
the one retained `RunEx` request never yields a valid running instance. The task
stays Ready with LastResult267011 and no `worker-start.json`,
`probe-ready.json`, `probe-admit.json` or `worker-result.json`; no worker/probe
process is present. ETW later reads QUERY4201 after an exact-owner external
recovery. This is another failed run, not native/audio qualification.

Root cause is a controller state conflation: `DispatchAttempted` meant both
"RunEx was requested" and "a running instance was obtained". A failed/invalid
RunEx therefore entered the wait-for-managed-terminal branch forever. Abort now
distinguishes a retained string `InstanceGuid` from a dispatch request with no
instance, verifies the exact task still has Ready/no-instance/267011 identity,
writes `DispatchRequestedNoInstance` abort evidence, stops/drains the retained
trace first, and only then retires that known created never-run task once after
confirmed trace-stopped evidence and archived exact task XML. A genuinely
unknown-but-bound RunEx still retains the original controller and waits for real
terminal evidence; no retry or relaxed task identity is introduced.

The PS5 transport failure is likewise diagnosed rather than bypassed: routing
the same loader through `cmd.exe` can break/redirect its native stdin pipe so
PowerShell5.1 receives EOF and exits `-1` without stderr. The actual PS5/PS7
loader test now launches each engine directly while preserving the exact
transport frame. All44delivery-transport,14diagnostic,10file-handoff and the
affected PowerShell boundary suites pass locally; these remain harmless host
checks, not native qualification.

The failed run, six exact sidecars/recovery receipts, five host query/recovery
scripts and five readonly observation directories are archived in65members.
Archive:
`etwmode-failed-run-archive\removal613-etwmode-02-failed-run-evidence.zip`,
65417bytes, SHA256
`E5E4706A049BDA30A0FF10DA829D957DC50059BAA14E07ECA18A8D0B262EF608`. The ZIP
and every member length/SHA256 are read back. The November13 bounded cleanup
then verifies the same boot/host/SID, empty owners/tasks, `QUERY4201`, exact
35-file/182378-byte membership and every pinned length/SHA256 before deleting
only those files, their verified-empty directories and the run directory. A
fresh readonly query finds no run, sidecars, owners or exact-nonce task. The
receipt retains `NativePassed:false` and `Qualified:false`; reusable tools were
not changed by this scratch cleanup.

Next is a fresh create-only diagnostic using independently reviewed
repaired-source pins; no default or latency claim is promoted.

## Remaining work and user action

The user has completed the requested reboot. No VM setup, additional reboot,
OBS installation copy or blanket desktop unlock is currently required. The
agent must review/deploy the repaired source with new pins and run one fresh
finite diagnostic only after prior-resource closure, then archive and retire
unused owned scratch. Keep one fixed dedicated OBS installation and preserve
ordinary OBS/QQMusic. Visible interactive Apply/Stop tests require a usable
desktop when actually scheduled; noninteractive SSH/audio work has its own
session/token/device requirements rather than a blanket unlock rule.

A to B to A is repeated renaming of the same logical endpoint, verifying actual
Windows naming, stable logical identity/graph binding, old-generation drain,
confirmed removal before recreation, successful new audio, sibling isolation
and unchanged default roles. It does not promise that every Windows PnP
instance string remains identical. S0-S5, held-client/racing naming, ordinary
interactive Apply/Stop, accepted matched waveforms and sustained calibrated
p95/p99below10ms (or the documented evidence-based device fallback) stay open.
