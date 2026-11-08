# Removal diagnostic preparation

These Windows PowerShell 5.1-compatible definitions prepare the next finite VAD
removal diagnostic. They are **not a native-approved runnable pipeline**.
Loading them launches no tools. Real task registration/dispatch and default
trace tools unconditionally refuse until reviewed orchestration exists;
changing an approval boolean is not authorization.

- `finite-tool.ps1` retains exact process/handle/creation and bounded redirected
  output. Timeout retains the same owner, not kill/relaunch or success.
- `task-boundary.ps1` retains scheduler registration intent before mutation,
  one RunEx instance and the first worker Process. It independently checks
  image, command line, creation, session and limited token through that handle.
- `trace-boundary.ps1` retains exact finite tool owners, validates inventory,
  preserves ambiguous create/stop failures and raw unknown loss. A successful
  exact create is not independent post-create session readback.

`WorkerTerminal` proves only observed scheduler/worker exit. Full `Terminal`
and `ProbeOwnershipVerified` remain false until actual native-child ownership
is wired; a worker's JSON probe exit is only `ReportedProbeExitCode`.

Run host-only checks from this directory with WindowsPowerShell 5.1 and 7:

```powershell
.\test-finite-tool.ps1
.\test-task-boundary.ps1
.\test-task-process.ps1
.\test-trace-boundary.ps1
.\test-trace-finite.ps1
```

The task scheduler and target identity are mocked. Real children only output,
wait and exit. No logman/tracelog, ETW, audio, OBS, remote target or driver is
operated. `test-task-process.ps1` requires a non-elevated Windows host process
in an existing nonzero session; it verifies real child token/CIM/command line
and natural exit, not actual scheduler behavior. Other device/security/task
boundaries remain mocked. Generated test outputs are ignored, not sources.

`finite-tool.ps1` is pinned by trace SHA256 to the reviewed predecessor. Its
synchronous file/hash/dispatch calls are not hard-bounded. COM/CIM/property
reads are not hard cancellation or crash recovery. Retain unresolved owners.

Remaining integration includes fresh post-reboot provenance, independently
bound collector/privileged trace/Limited worker, worker-to-probe admission and
lifetime handshake, bounded create-new receipts, independently confirmed ETW
identity/provider/output, raw loss/decode/archive and safe authorized cleanup.
This preparation's default stop also refuses; it must not be adopted as a
running native trace's cleanup authorization policy.

Full S0-S5 acceptance is unchanged. Use the separate standalone remote target
directly; no VM or blanket desktop-unlock prerequisite. The user owns reboot.
Reuse the fixed dedicated OBS installation; never copy executable trees or
change firewall/security policy to suppress prompts.
