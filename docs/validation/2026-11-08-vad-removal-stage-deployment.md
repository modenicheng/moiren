# VAD diagnostic deployment awaiting reboot and bounded offline reader

Date: 2026-11-08, Asia/Shanghai. This follows the
[removal-stage implementation](2026-11-07-vad-removal-stage-diagnostics.md).
Full S0-S5 acceptance remains open. The canonical verdict is
[`driver/vad/STATUS.md`](../../driver/vad/STATUS.md).

## Once-only deployment and observed reboot reason

Evidence root:
`D:\coding\moiren-evidence\20261107-233656-removal-diagnostic`.
The separate standalone target is accessed directly over `moiren-lan`;
VM startup and visible desktop unlock are not prerequisites for these checks.

The signed3.1.1.613 package is built from
`23807f3dc482090c0dd6f0a9775cd91b8d6c1da6`. Its manifest SHA256 is
`DEF621D149EF43753D27B8CE2A8DFA1237E6D8B106D369931A50F07E24AC1317`;
the152472-byte SYS SHA256 is
`0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76`.
This adds bounded diagnostics, not a repaired native removal implementation.

The adapted deployment runner SHA256 is
`57B0412C262435E75C5230CB2FFA0BC35DDC7913417D0BA7252650EECAC1DA2F`.
It validates the exact diagnostic/source/PDB/rollback contract, including
560bytes, six stages and native-cleanup-repair=false. Literal version mapping
avoids chained substitutions. PowerShell5.1 and7 each pass137mocked-boundary
checks, including no-op installation rejection through the real final binding
validator. These are software tests, not native driver acceptance.

Remote ValidateOnly reports Verified=true with no install or restart and
unchanged safety audits. At00:10 the single real `pnputil /add-driver ...
/install` returns3010. The final receipt retains:

```text
InstallAttempted=true
InstallExitCode=3010
Verified=false
RebootRequired=true
RestartAttempted=false
RebootRequested=false
AuditErrors=[]
```

Service-file/PnP metadata reports Running, OK/problem0, version3.1.1.613 and
oem207.inf with the expected SYS hash. This proves the staged binding/file,
not completion of the running kernel image transition. Before/after boot
timestamps remain `2026-10-30T09:50:55.3826800+08:00`.

The bounded SetupAPI readback is retained separately at
`D:\coding\moiren-evidence\20261108-001810-reboot-reason`.
It reads the last2MiB and records:

> Service image path changed. Restart required for any devices using this service.

The adjacent record says the previous MoirenVad package needs reconfiguration.
Which specific remaining device/reference holds the old lifetime is not
established. An earlier unbounded read-only query times out; only its exact
owned process identity is stopped before the bounded replacement read. The
protected external PowerShell process remains untouched.

No device restart, automatic reboot, retry, rollback, endpoint enablement,
identity rotation, native audio or desktop input follows3010. Defaults, render
volume/mute, security and protected process identities compare unchanged;
the terminal audit finds no test owners/present raw children/running tests.
The612rollback package remains preserved, with SYS SHA256
`B25CA2E8DB8DD3E98D9B90AA6CF19822C879E6CA13D52B35D160CEF2056D1737`.

The92-member deployment archive independently verifies every member's
length/SHA256 after download. `deployment-613-terminal.zip` is75572bytes,
SHA256 `B793810FF3146D116FAC3170E9B144A9AC9A7D13F267D30877121470F1718B22`.
Original remote evidence remains retained.

The user performs any needed reboot personally, after the running OBS cleanup
has finished. Reboot permits a fresh boot/provenance check; it does not itself
prove the previously failing cleanup is repaired.

## Offline decoded-stage association

`scripts/vad-removal-stages.py` consumes decoded WPP text, its decoder summary
and one retained native report. Each input has an explicit maximum of16MiB;
output has an independent1MiB maximum and create-new semantics. It performs no
native operation. The exact five-scalar typed QUERY stamp selects runtime and
session; cached/failed/missing provenance cannot establish direct Removed.

The output keeps these independent facts:

- Partial decoded-stage association and observed/unobserved stages.
- Actual Fresh QUERY state Removed.
- Explicitly successful source Stop and worker join.
- Closed Engine with no reported cleanup error.
- Original native error, status text and native-call flag.
- Decoder statistics, retained overwrite/saturation evidence and unknown raw
  ETW loss.

First/last/first-failure kinds of the same scalar sequence are deduplicated
without inventing retries. Conflicting scalar records, QPC/frequency changes,
counter/watermark regression or impossible growth, malformed/duplicate decoder
statistics, missing stage headers, loss and saturation refuse association.
Duplicate report JSON keys and nonfinite constants refuse before output,
preventing an ambiguous state field from being adopted as direct Removed.
First-failure observations require HasStatus and failure bits for numeric
NTSTATUS; unknown symbolic status names are retained without invented bits.
A new-runtime overwrite naming the selected old runtime also refuses
association rather than disappearing into the foreign-fragment count.

All29tests pass after retained RED/GREEN runs. Five earlier review failures,
seven later subcase failures and two ambiguous-JSON cases are reproduced before
fixes. All eight neighboring offline suites also pass153tests in total on
the Windows development host; CI surface mapping passes58checks. Tests are
synthetic plus a production-format/source contract check; there is no new613
native trace. CI runs the reader suite alongside the existing offline VAD
readers.

Exit0 means only the retained decoded association subset is available. Exit1
retains refusal reasons; exit2 indicates input/IO/limit failure. Every report
keeps qualified=false. Cleanup/Destroy are callback entry, ReleaseEnter may
include pre-ACX quiescence, and decoder loss0 cannot attest raw trace
completeness. None substitutes for direct Removed or physical qualification.

## Next native question after user reboot

Re-establish target identity, boot time, package/signature/SYS/PnP provenance
and protected external process identities. Old process pins may legitimately
change across the user's reboot; preserve the newly observed external owners.
Use the fixed fixture and matching613TMFs for one finite removal diagnostic,
keeping the same control owner alive through source Stop/join and Engine
closure, with no intervening CREATE/slot reuse. Retain the unchanged20s direct
Removed deadline and trace ownership/loss evidence. No unchanged Process/VAD
pair, timeout extension, reset or default promotion is authorized by this
diagnostic checkpoint.
