# Removal613 real read-boundary preparation, not native approval

Date: 2026-11-08, Asia/Shanghai. This increment follows the
[contract-only controller preparation](2026-11-08-vad-removal-stage-deployment.md#post-reboot-controller-preparation-not-a-native-result).
The user has not reported a completed reboot. No remote execution, native
audio, OBS launch, ETW session, task, driver change or reboot occurs here.
The original [S0-S5 requirements](../vad-low-latency-goals-2026-10-28.md)
remain the completion criteria.

Evidence directory:
`D:\coding\moiren-evidence\20261108-105800-removal-613-runner`.
The preceding controller/evidence directory remains unchanged.

## New real definitions

`finite-tool.ps1` retains one actual System.Diagnostics.Process and its original
handle/PID/creation, pins the executable and CRT-quotes argv. Both create-new
logs share one byte budget. Exit, pipe drain and flush observation share a
finite deadline. Timeout/overflow/output errors remain sticky, and later natural
exit cannot turn failure into success. Re-observing a completed timely receipt
does not invent a timeout. Unknown dispatch/identity/live pipes remain retained;
no automatic kill, PID reacquisition, dispose or relaunch is provided.

`readonly-preflight.ps1` implements definitions for real typed GetTickCount64
uptime, corroborated post-install boot, pinned candidate/rollback manifests and
files, signatures/catalog membership, installed613driver/PnP provenance,
defaults/render controls, security and process/task/raw-child collection.
Dot-sourcing the file does not invoke these collectors. GetTickCount64 is
exercised locally; WinPS5 Environment.TickCount64 is not used.

Fresh external OBS/QQMusic identities are protected, not adopted as test-owned;
an explicitly observed empty list does not require launching external software.
The dedicated fixed OBS path is busy. Queued/Unknown Moiren tasks must not
disappear into an idle result. Process identities are collected last with their
actual enumeration interval, not stamped with a later collector's completion
time. SSH collector session is observed rather than forced to desktop session1.

## Verification and limitations

Fresh parent runs pass42checks on each of Windows PowerShell5.1 and PowerShell7:
16finite-tool,13preflight,10full mocked-collector and3actual helper/read-wrapper
composition checks. Real children only emit text, sleep or exit; no audio or
device boundary is exercised. Tests substitute all target/device/security/
signature/audio/task collectors. Behavioral failures before the deadline,
pending-task and observation-time fixes are retained alongside final logs.
Local Stop preference is also hardened, but the parent does not reproduce a
false-empty creation-conversion result inside its try/catch harness.

Parent verifies the finite helper SHA256
`C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B`
and tests SHA256
`F15424DB54E75ABB6CD17BD4CD87277E7DD6A3F31F22DF36618058A62C63A40C`.
`preparation-manifest.json` pins17files, all with native approval/activity false.
The563-member relative-path archive independently verifies every length/hash:
`host-only-preparation-v2.zip`,199029bytes, SHA256
`B521A23A1AA6758C3AEBB83099F7080D6FE6F67CBF1055268F93417C6CF9EA63`.
The earlier convenience archive is not the authoritative verified archive.
An attempted removal of archived local scratch is rejected by execution policy;
no alternate-shell deletion bypass follows. No new executable copies exist.

This is still NOT NATIVE APPROVED. In-process PnP/task/Authenticode/SecureBoot/
CIM calls are not wrapped in a finite owned worker; operation timeouts are not
an overall hard deadline. The frozen controller's session1 contract must not
be satisfied by relabelling SSH session0 evidence. The next runner must bind
collector/trace-controller and Limited audio-worker identities separately,
retain exact task/action/principal/HasRun/probe ownership, and wire confirmed
trace start/raw loss/owned stop, bounded receipts and matching decode/archive.
No single snapshot is an atomic proof of all earlier observations. Async flush
does not attest power-loss durability, and process exit is not process-tree exit.

Original29draft/cache file hashes verify unchanged at11:08. The native613
removal result, actual repair, final Windows naming plus identity/settings
preservation, matched waveform, ordinary desktop and sustained/calibrated
latency acceptance remain open. PR26 remains Draft/open.
