# Moiren drivers

`vad/` is the independent x64 ACX/KMDF virtual-audio driver work.
See [its status](vad/STATUS.md), [control ABI](vad/ABI.md) and
[source provenance](vad/PROVENANCE.md) before using the unsigned artifacts.

From the repository root, in PowerShell with Visual C++ and WDK 10.0.28000.0:

```powershell
.\driver\vad\build.ps1
.\driver\vad\test.ps1
```

Build outputs and logs stay under `vad/build/`, which is ignored. The build
does not install a driver, sign a binary, alter boot/debug settings or access a
remote target. No Rust/Engine/application route uses this driver by default.

The existing `moiren/` directory is a separate user-owned project; the new
foundation does not depend on or modify it.

## Host integration checkpoint - 2026-10-20

The [current client checkpoint](../docs/vad-lifecycle-validation.md#current-host-integration-checkpoint---2026-10-20)
and [implementation ledger](../docs/superpowers/plans/2026-10-19-vad-implementation.md#current-host-integration-checkpoint---2026-10-20)
record the committed user-space integration and completed host checks,
including the full workspace result of 1,290 passed, zero failed and six
ignored. Driver fix `709e870` passed fresh host tests and the analyzed build;
independent specification and code-quality reviews passed with no findings.
Build evidence is in `driver/vad/build/build.log` and
`driver/vad/build/infverif.log`, relative to the repository root.
Host/build evidence does not qualify an installed driver. The latest read-only
audit found zero running VMs and no guest power-preflight success receipt; no
driver installation or target exercise has occurred.
