# Reused OBS installation and removal of redundant test copies

Date: 2026-11-08, Asia/Shanghai. The user requires a reused OBS installation,
allows one separate dedicated testing instance, and requires finished unused
test artifacts to be removed. Repeated per-run executable copies created many
different paths and unwanted firewall prompts.

## Fixed installation and frozen cleanup scope

Keep the existing dedicated test installation in place:

```text
D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable
```

Its executable is `bin\64bit\obs64.exe`,5131824bytes, SHA256
`30FB8CCF7700B6747FBF349845EC0EE0DDCBCD1189AA67607E1FF6ADC7EC9E80`.
Do not copy/move it to another per-run path. Serialize its use; preserve the
user's ordinary OBS instance. Future runnable harnesses must use this path
and retain only small per-run configuration/log evidence. The historical
off-repo `20261107-matched-waveform-2255\run-guarded.py` still contains its old
copytree setup and is frozen evidence, not a reusable future runner.

Inventory finds31identical OBS trees. Freeze exactly30other `obs-portable`
directories for deletion:65453files,14584284421bytes. Their root layouts contain
only OBS application/configuration assets; no recording files or reparse
points are present. Recordings elsewhere, driver/rollback packages, ordinary
user OBS, firewall rules and security settings are outside the deletion scope.

Evidence root:
`D:\coding\moiren-evidence\20261108-001641-obs-reuse-cleanup`.
The remote cleanup workspace uses the same suffix under `D:\MoirenTest`.

Small configuration/log evidence and obsolete task XML are archived before
deletion. `retained-config-and-tasks.zip` is1138973bytes with538members,
SHA256 `59066CD21B6F9702B0691F2BFA6C6AC1C0690D571D163F7C12B60683FBD032BB`.
Every archived member independently verifies length/SHA256 after download.
The13568976-byte path/size/mtime inventory SHA256 is
`2179B49A3FED070546C50D682FD851BB7C01C085909E0C03A9015457FB67A440`.
The reviewed preparation file is separately pinned by literal SHA256
`226B9F8E7AAC80BB278E9390455EA85014079AC364726566E24FD9A2AF2F535A`.

## Bounded deletion and readback

The reviewed script validates the exact target identity, canonical keep path,
all30literal target paths, containment, no-reparse ancestry/children, executable
hashes and the full frozen path/length/mtime inventory. It refuses active
OBS/test owners and running Moiren tasks. Six unsafe-target checks pass
locally; independent review's unpinned task-list finding is fixed with the
literal preparation hash before execution.

One invocation first unregisters66exact archived, stopped, same-SID test tasks
after exported XML hash comparison. This prevents obsolete scheduled runners
from recreating the retired copies. It then deletes the30literal trees and
checks the surviving executable and task absence.

SSH transport observation times out after270seconds, but the exact original
cleanup owner remains alive and progresses. Read-only observations retain its
command hash, PID15660 and creation time1794068870704. No deletion is relaunched.
At00:35 only5old tasks remain; at00:37 only16target directories remain; at00:39
all30targets are absent.

The00:40 independent readback finds exactly one OBS executable, at the fixed
path with the original hash, and zero archived tasks or old target directories.
No test/OBS processes are present. The boot timestamp remains unchanged.
The previously protected PowerShell identity remains present; the previously
observed QQMusic process is absent. Its disappearance time/cause is unknown,
and external playback continuity is not established. The deletion script does
not stop external processes. Consequently the broad combined readback flag,
which also requires both old external identities, is false; this must not be
presented as a complete unchanged-safety verdict.

By00:41 the original cleanup owner has exited. Its final receipt reports
Verified=true, all30trees removed,14584284421bytes removed, all66tasks
unregistered, and the one expected surviving OBS executable. This flag
describes the frozen cleanup scope, not external playback continuity. The
downloaded `cleanup-result.json` independently verifies SHA256
`69D1D891D59C776ACE949A6AAFFD12D7B888BDBD26B7ADF206EC23C17EF347A5`.
Its At field is the invocation start; terminal readback establishes completion.
The target is now idle for the user's manually initiated reboot.

No system reboot, driver write, firewall/security change, endpoint reset or
native audio acquisition is part of this cleanup.
