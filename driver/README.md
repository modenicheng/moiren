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

## Host integration checkpoint - 2026-10-20

The [current client checkpoint](../docs/vad-lifecycle-validation.md#current-host-integration-checkpoint---2026-10-20)
and [implementation ledger](../docs/superpowers/plans/2026-10-19-vad-implementation.md#current-host-integration-checkpoint---2026-10-20)
record the committed user-space integration and completed host checks,
including the full workspace result of 1,290 passed, zero failed and six
ignored. Driver fix `709e870` passed fresh host tests and the analyzed build;
independent specification and code-quality reviews passed with no findings.
Build evidence is in `driver/vad/build/build.log` and
`driver/vad/build/infverif.log`, relative to the repository root.
Host/build evidence does not qualify an installed driver. The zero-VM and
no-target-evidence observation belonged to that 2026-10-20 checkpoint. Subsequent
test-signed guest installation and bounded native/App results are recorded in
[the current status](vad/STATUS.md); they do not qualify long-duration stability.
See [continuous soak instructions](../docs/vad-soak-validation.md) for the audit
follow-up and opt-in eight-hour tests.
