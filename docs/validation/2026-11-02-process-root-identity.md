# Process-tree root identity and caller creation-time pin

## Resolved retained PID discrepancy

The [frozen replay coverage](2026-11-02-process-replay-coverage.md) correctly
retained different player/native PIDs as unresolved. Bounded read-only remote
historical evidence now explains the difference; no recording or stability run
is repeated to obtain it.

Target `LAPTOP-NP49RG6H`, original run
`D:\MoirenTest\20261102-060101-native-pcm-smoke`:

- Security event4688, record54239356, November2,2026 at06:06:10.1789013+08:00
  records launcherPID16768, the venv `Scripts\python.exe`.
- Event4688, record54239357 at06:06:10.1892921+08:00 records childPID24944,
  `C:\Python310\python.exe`, created byPID16768. This is the actual historical
  parent/child event, not merely a present-day reproduction.
- `run.py:175` launches player.py via `sys.executable`; `run.py:204` passes
  `Popen.pid`16768 to Moiren. Cleanup at`run.py:325` records that same direct
  launch handle. `player.py:95` uses `os.getpid()`, hence24944.
- Both original native origins identify16768 with exact creationFILETIME
  `134380443701788661`. The child's exact historical creationFILETIME was not
  retained; millisecond CIM or audit-event timestamps must not substitute for it.

Production activation explicitly uses
`PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` in
`crates/moiren-windows-audio/src/process_loopback/activation.rs`. Capturing the
pinned launcher therefore includes its emitting descendant. The old route is
not shown to be an unrelated-process miscapture by this PID difference. This
resolves the parent/child provenance question, not exact child-lifetime binding,
absence of other descendants, sample-exact owner replacement or rendered epoch
coordinates. Original receipts and quality faults remain unchanged.

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
The root-cause handoff is
`20261102-pid-discrepancy-root-cause-handoff-20261101-232846852-07ed76f5.md`;
historical event/source hashes are in
`20261102-pid-discrepancy-history-sidecar-20261101-232803969-be8364e8.json`.
Approved source copies match run.py SHA256
`56F8A68F5C8D2B62BB78490A9BEAEDFFF01D9F8EBEFF7D709A2BDE6EF01AFBA5`
and player.py SHA256
`63C6B61E06977F36BF59ACAFBEDD2DDA8F5D6E6D864D77FC99BC15FEB88C5EE5`.
The bounded06:06:09..06:06:12Security query examines12events with a100event cap;
Sysmon is unavailable. No audit policy or device setting changes.

## Implemented caller identity contract

Monitor accepts optional `--process-creation-time-100ns N`, an exact nonzero
Windows creationFILETIME with full `u64` precision. PID remains mandatory.
Zero, negative, fractional, malformed, overflow and duplicate values refuse.
Omission preserves the existing behavior: inspect the selected PID, then pin
that resolved PID+creation identity through native owner preparation.

With a supplied stamp, mismatch refuses immediately after `inspect_process`,
before driver, diagnostic PCM, cursor directory or service creation. The shared
`context` path also enforces it before a test/custom service can prepare.
Native `open_selected` retains its existing exact identity/liveness recheck
when opening the capture owner; the earlier caller pin does not replace that
race protection. No descendant is discovered or substituted by executable name.

Monitor report schema3 adds `requested_process_creation_time_100ns` (null on
omission) and `process_capture_scope:"target_process_tree"`, separate from
resolved `process_pid` and `process_creation_time_100ns`. Scope explicitly means
root plus descendants, not an authenticated emitting child. Report qualification
stays false. No production queue, default, admission, recovery, SRC or first
packet behavior changes; no stream-thread work or allocation is added.

```powershell
moiren-app monitor --process ROOT_PID --process-creation-time-100ns EXACT_FILETIME --output ENDPOINT
```

A future player harness must separately retain launcher and emitting-child
identity and choose its intended root explicitly. For child-root capture, use
the validated self-reported PID/creation stamp and a held native process handle,
retaining launcher handles for cleanup. This increment does not rewrite or
retroactively upgrade the old ephemeral harness, and does not provide an
exact-emitter-only WASAPI mode.

## Regression and independent export

Retained RED evidence first shows the missing option, then the missing scope
field. Four new tests cover exact parsing, same-PID/different-creation refusal,
zero backend preparations/starts on refusal, and report identity/scope. Existing
compatibility and help tests additionally assert omission, schema3 and null
requested stamp. The focused CLI suite passes71tests. A test compilation error
from `unwrap_err` requiring a Debug implementation is corrected in the test
before the behavioral RED; it is not counted as proof of missing functionality.

Independent review finds no important issue in the production six-file change.
It identifies that fake-driver tests alone cannot enforce ordering in the real
entry point, so the remote check below exercises the production rejection path
with a real held process identity. This does not require audio initialization.

Only the six intended Rust files are exported;18preexisting dirty files are
excluded and hash checked unchanged. Source tree
`9835f3fbef582ad0d7003e2561658a9d43e16220` builds the remote binary. Additional
schema/null assertions added afterwards are test-only, not a new runtime binary.
Scoped build/test/log receipts use `s1-process-identity-*-1102` in the evidence
root.
The independent export completes `cargo test --locked --workspace` with
2171passed,0failed,8ignored across70test targets; the separate gpui-test
`measured_ui_tests` run passes20tests. Workspace all-target strict Clippy and
`cargo fmt --all -- --check` both exit0. These checks include the six-file
runtime change, not the later eight assertion-only lines. After copying only
that test file into the same export and recording its hash, the final focused
CLI71tests and formatting check pass again. No later runtime source differs
from the remotely checked binary.

## Remote pre-native refusal smoke

No native audio, device setup, source signal or OBS recording starts. A single
owned metadata-only PowerShell sleeperPID22712 supplies a held process handle;
`GetProcessTimes` obtains exact creationFILETIME `134380497028162203`.
The new binary is placed at
`D:\MoirenTest\20261102-process-identity-20261102-073417689\moiren-app.exe`,
71989248bytes, SHA256
`CC41E82803EE5A4046778E7B3446DD3422D0D3D059344B2ED694365D57AC561A`.
It is built from the independently exported source tree, not the dirty worktree.

At07:35:02..07:35:04+08:00 on November2,2026:

- Expected stamp1: process15268 exits1 with the exact observed stamp in the
  creation-identity error; requested cursor directory is not created.
- Expected stamp18446744073709551615: process19452 exits1 with the exact integer
  retained in its error and the same observed stamp; no cursor directory.
- `monitor --help`: process11648 exits0 and describes the option and tree scope.

Both refusals deliberately include the PCM tap and a nonexistent output label.
The actual error is identity mismatch, not endpoint preparation, proving that
refusal precedes archive/native setup. A correct identity's live audio route is
not exercised or claimed. APPDATA is redirected to a new isolated test profile
and RUST_LOG=warn for these owned processes only. User settings are not changed.

The wrapper accesses and holds each process handle before waiting, retaining
numeric exit codes rather than the prior offline wrapper's null-exit limitation.
All four owned processes are terminal at cleanup. User OBS12648/start1793505454949
and QQMusic15096/start1793504036422 remain unchanged. Receipt:
`s1-process-identity-remote-result-1102.json`; exact launcher source and binary
settings are retained beside it. No native or physical latency PASS follows.
The final read-only remote inspection at07:40:53+08:00 reconfirms binary hash,
all numeric refusal/help exits and cleanup receipts, no remaining app under
this owned test root, and unchanged protected OBS/QQMusic identities. It starts
no process or audio route; receipt `s1-process-identity-remote-final-1102.json`.

## Remaining S1-S3 work

The old eight gap-bound violations remain. Historical parent/child evidence
does not create sample-exact Engine/render epoch alignment or physical
calibration. First-packet investigation stays deferred. This provenance fix is
not justification for a new repeated stability scan, weakened quality gates or
lower defaults. Continue the remaining independent source/owner/render-boundary
integration and native route verification without claiming S1-S3 complete.
