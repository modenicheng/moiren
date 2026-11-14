# Process requested/active diagnostics accessibility

Date: 2026-11-02, Asia/Shanghai. Baseline: clean `6bd0a9c` plus only the
two application source changes described below. No audio, shutdown, driver,
default, admission or signal-analysis policy changes.

## Native reproduction and root cause

The current release `AD7DA050F1FAF8961D304E6236C880225DB42453020513A92822F1B1B287173F`
is built from a clean archive of `6bd0a9c`, not the eighteen preserved dirty
PCM/SRC/driver paths. One remote Limited Interactive session1 attempt uses an
isolated project/preferences directory, exact source PID/creation identity,
Process -> unity Bus -> DAWN PRO2, Engine48k/F32/block64,
Shared336/capacity738, render512/wake64/callback64 and Compatible2048/8192.

The `20261102-desktop-uia-1400` run first confirms the previously repaired
disabled Apply action: `IsEnabled=false`, no InvokePattern, not offscreen,
exact app PID24972, expected stopped preflight refusal. The complete button
and refusal are screenshot-verified. The source endpoint is prepared but the
source is not activated and no application graph owner exists at this point;
this is not a claim that no native endpoint was opened.

Start then confirms the exact native output and Compatible bridge tuple.
However, Diagnostics exposes neither requested nor active Process summary to
Windows UIA. Both selector arrays are empty in the retained snapshot. The
two production `detail` rows set an ID and ARIA label but omit an accessibility
role. GPUI's observed headless labels can still see those attributes without
proving that a native node exists. The existing `native_detail` helper already
sets a Label role plus matching text value for actual native diagnostic rows.

This attempt remains **failed**. It never activates the changing source or
reaches its twelve-second smoke, explicit Stop, joined-owner assertion or Quit.
Failure cleanup posts owned WM_CLOSE while Settings is open; it does not exit
within ten seconds and the held app is forcibly ended (exit-1). Source exits0.
The log retains `0x80040102` and `0x80070578` at cleanup, not a demonstrated
normal-Quit failure. No successful lifecycle or waveform verdict is inferred.
Final task Ready/result1 has no owned native/analysis process. Protected user
OBS/QQMusic, default endpoints, all catalogued render volume/mute controls and
the running SYS hash are unchanged.

## Minimal repair and regression

Only the requested and active Process diagnostics rows reuse `native_detail`.
Their stable IDs, visual children, order, requested/active values and conditional
active-row rendering remain unchanged. No dependency or registry source changes.

The existing real headless ingress-confirmation test now asserts Label role and
value=label for both rows while requested and active configurations differ,
and rechecks active role/value after actual service confirmation. The same test
still verifies requested200000 versus activeDisabled, pending-not-success state,
confirmed200000 and persistence. No test-only accessibility state is fabricated.

Clean export evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-process-request-a11y-20261102-1405`.
`red.log` is a genuine assertion failure: roleNone versus Some(Label), not a
compilation or fixture error. After the two-row repair, `green.log` reports
one passed, zero failed. `cargo test --locked --workspace --features
moiren-app/gpui-test -j1` then exits0:2307 passed,9 ignored,0 failed across71
surfaces, including13 UI acceptance cases. These Windows software checks do
not rerun Linux/CI/Miri or native opt-in hardware tests. Repaired native
acceptance is a separate result, not implied by these counts.
`cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test
-j1 -- -D warnings` and `cargo fmt --all -- --check` also exit0.
`source-verification.json` confirms that these two source files are the only
tracked differences from the clean export. Independent static review finds no
concrete defect in the repair or the retained failure/cleanup attribution.

## Changed-binary native acceptance

One new finite remote attempt uses the same worker logic, changing only the
isolated base directory and application pin. Release build exits0; repaired
application SHA256 is
`7282994A001122C520CF4386A334F61076D321EED165A85AA664A872AEB6144F`.
The user-testable binary is
`D:\MoirenTest\20261102-desktop-uia-1415\bin\moiren-app.exe` on the remote
machine, not the development host. No driver install is needed for this
Process route. The loaded driver remains the existing pinned Running SYS.

`Moiren-CurrentDesktopUIA-20261102-1415` finishes Ready/result0 at14:22:56.
Supervisor, worker, app and source all exit0 normally; no forced termination or
cleanup error occurs. The original thirty-second launch admission,150-second
worker watchdog and240-second outer task cap are unchanged.

- Current native disabled Apply remains `IsEnabled=false`, no InvokePattern,
  not offscreen and owned by app23344. The stopped refusal is unchanged.
- Both repaired Diagnostics native summaries are present exactly once:
  requested and active each report `Compatible; host ingress elapsed limit:
  Disabled`. Native stream336/capacity738, prepared render512/capacity738 and
  Application bridge2048/8192 are confirmed independently from their labels.
- Source22956/creation134380741359295177 activates only after those checks.
  Its ten-second changing schedule is fully submitted480000 source frames;
  total active source duration14.5108462s includes zero tail. No waveform is
  recorded, so submission does not establish audible/captured continuity.
- Explicit Stop finishes owner1 cleanup/reap with `cleaning=0` before source
  stop. Stopped diagnostics retain the exact source PID/creation and native1
  predecessor, not just an application image-name match.
- Quit is revealed entirely above the fixed footer: actual UIA bounds
  y1071..1120, safe bottom1125. Owned-window screenshot confirms exposure;
  Invoke returns and app exits0. A requested1100x700 resize is not claimed as
  an observed final viewport size.
- The bounded Process trace collection reports `EventLimit`:1090 packets /
  8192 reads retained. That explicit refusal remains a refusal, not a trace,
  lower-budget, whole-run timing or waveform PASS. Focus-role warnings from
  unrelated controls remain in the original log; they do not erase the exact
  repaired-row/native lifecycle observations.

Final audit finds no owned native or analysis process, unchanged protected user
OBS12648 and QQMusic15096 creation identities, default roles, catalogued render
controls and running SYS pin. Raw receipts/snapshots/logs remain remote. All25
compact archived files are length/SHA256-verified after download;372133-byte
archive SHA256 is
`06039D7E44F2C7A8CBD382DB3CE9F818A77EC634FDBB791BD48D65B00A3D9B21`.
Host evidence directory:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-current-desktop-uia-20261102-1415`.

## Retained bootstrap and environment boundaries

The preceding `20261102-desktop-uia-1345` GUI supervisor attempt enters but
refuses its worker at the unchanged thirty-second launch-age guard. Task result1;
no worker, source, app or audio launch is established. The no-audio GUI control
had completed normally, which proves that bootstrap route can work, not the
cause of earlier delayed launches.

A read-only audit finds the exact owned historical packet-format inspection
PowerShell PID4264 using26639048704 bytes working set with963044KiB free RAM;
the exact old stdin inspection PID18224 also remains. Their creation times,
session0 paths and decoded command hashes are verified before cleanup; they
are not user OBS/QQMusic. The cleanup script pins both handles, calls Kill only
on18224, then its five-second wait times out before reaching4264. A later
read-only audit finds both absent and28474288KiB free RAM. It does not establish
how4264 exited or prove memory pressure was the unique startup cause. No host
process or image-name-wide cleanup is used. The failed cleanup receipt remains
failed; the follow-up absence audit is separate.

Only after that observed memory recovery does the isolated1400 attempt run;
its worker enters promptly with the original30s admission and150s watchdog.
This is not an unchanged soak or gate relaxation. Both old bootstrap/lifecycle
failures stay retained.

Host evidence directories under
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`:

- `s3-current-desktop-uia-20261102-1345`: clean release source/build pins,
  no-audio control, failed launch, decoded exact-owner audit, failed cleanup
  and independent memory/absence readback.
- `s3-current-desktop-uia-20261102-1400`: original native results, snapshots,
  screenshots, stdout/file logs, catalogues and manifests. All21 archived files
  are length/SHA256-verified after download; the259304-byte compact archive is
  SHA256 `33FB4CE3805B8ADA6FCFA01C5A0A918C1CAD6577A4EB86A66221E5D143E3D0BF`.

## Acceptance boundary

Both accessibility repairs and the finite Compatible Start/Stop/join/Quit
sequence now have their own changed-binary native evidence. The earlier failed
attempts stay failed, and software requested/active divergence tests remain
distinct from this equal-Compatible native observation. Neither resolves
retained waveform refusals, custom measured desktop native
replacement, physical calibration or the original overall S1-S3 qualification.
First-packet investigation remains deferred. Useful isolated remote tests may
overlap local headless development; unchanged soaks and unavailable physical
equipment are not prerequisites for independent product work. No push or memory
update occurs.
