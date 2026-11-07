# Desktop Quit viewport control

Date: 2026-11-02, Asia/Shanghai. Branch: `feat/minimal-io-latency`.

## Decision and scope

The [exact336 VAD/DAWN recording](2026-11-02-vad-dawn-exact336-functional.md)
retains `Completed=false` and final owned-app exit `-1`. Its Quit automation
does not establish that the application Quit handler ran: the button was
partly covered by the fixed settings footer. A separate stopped-app control
now scrolls the whole action above the footer and exits normally, code 0.
There is no evidence here requiring a production Quit change or another
native audio/stability scan. First-packet investigation remains deferred.

This increment adds headless layout coverage and updates stale integration
expectations. It changes no production
audio, shutdown, driver, recovery, default or qualification policy. A stopped
control is not proof of every post-audio shutdown path, waveform quality,
matched-route latency or calibrated physical latency. Those remain open and
do not block independent development without physical calibration hardware.

## Mechanism and retained setup failures

Pinned `gpui-pre-0.3.9/src/window.rs`, `handle_a11y_action`, handles AccessKit
Click by synthesizing mouse down/up at the accessibility bounds center.
UIA Invoke returning and Offscreen=false therefore do not prove that the
handler receives a click when that center is covered by another element.
Production settings already use a `flex_1/min_h_0/overflow_y_scroll` content
area followed by a fixed `flex_none` footer; Quit already calls `cx.quit()`.

Finite remote attempts are retained, not overwritten as passes:

| Remote suffix under `D:\MoirenTest` | Observation |
| --- | --- |
| `20261102-quit-noaudio-0822` | Harness mistake: a fresh profile enables automatic startup. Actual native default audio owner started, despite the raw hardcoded `native_audio_started=false` field. Quit was never invoked; only the held owned app was terminated. Corrective receipt explicitly marks the field untrustworthy. |
| `20261102-quit-noaudio-restored-0827` | An isolated restored project correctly stayed stopped. The harness global UIA FromPoint probe failed before scrolling; Quit was never invoked. Owned cleanup only. |
| `20261102-quit-owned-viewport-0830` | The footer guard was too conservative and refused a fully exposed button. Screenshot retained; no Quit invocation, owned cleanup only. |
| `20261102-quit-visible-0831` | Correct stopped startup, owned-window-local scrolling, complete button exposure and native Invoke; normal exit 0, no forced cleanup. |

The first attempt's actual audio startup remains recorded in
`s3-quit-first-attempt-correction-1102.json`; its intended no-audio scope is
not the observed result. All device/UI attempts ran on `LAPTOP-NP49RG6H` only.
No developer-host audio, devices or user processes were opened or stopped.

## Successful control

Actual runner interval: 08:30:12.995-08:30:22.413, not the directory suffix.
Task: `Moiren-QuitVisible-20261102-0831`, lifecycle `task`.
App PID 16420, creation FILETIME `134380530135327050`; clean source `1267f8d`.
Executable remains:

```text
D:\MoirenTest\20261102-vad-dawn-exact336-0812\bin\moiren-app.exe
SHA256 A836C1CF83E977650C7B8ECC4D4A99B24C47569ACF62C42317864F260953B043
```

The isolated profile restores a project with no exports and never invokes
Start, starts a source or recorder, or requests an audio owner. The owned
window is resized nonactivating to 1100x700; foreground HWND remains 66882.
After four scrolls, Quit bounds `[218,908,1488,49]` fit wholly above the
conservative footer cutoff 961. The retained PNG visibly confirms this.
Quit Invoke returns at 08:30:22.315, process exit is 0, `completed=true`,
cleanup errors are empty. The recorded 2070ms wait includes the existing
Invoke helper's two-second observation, not measured shutdown latency.

The application log contains no native-host/owner start; final retirement is
`generation=0 owner_present=false cleaning=0`. A fresh read-only final audit
finds task Ready/result 0 and no Moiren/probe processes. User OBS PID12648 /
start1793505454949 and QQMusic PID15096 / start1793504036422 remain unchanged.

## Software checks

New `stopped_diagnostics_quit_scrolls_fully_above_the_fixed_footer` coverage
uses the real stopped shell and Settings/Diagnostics view. Comfortable and
Compact density each run at 1100x700 and 1440x900. The whole Quit button must
fit in the scrollport above the visible footer, Save/feedback stay visible,
the effective draft retains the requested density, and the service stays Idle.
This tests existing behavior, not a claimed new production fix.

Fresh checks against the clean source export plus only the two owned test
module files and the integration expectation file, excluding all preexisting
probe/VAD edits:

```text
cargo test --locked -p moiren-app --features gpui-test stopped_diagnostics_quit_scrolls_fully_above_the_fixed_footer
  1 passed
cargo test --locked -p moiren-app --features gpui-test --lib
  657 passed, 3 ignored, 0 failed
cargo test --locked --quiet -p moiren-app --features gpui-test
  719 passed, 4 ignored, 0 failed across library and integration suites
cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings
  exit 0
cargo fmt --all -- --check
  exit 0
```

Separate fresh `app_quit_drains` and `close_releases_window_view` filters
also pass one test each. This is scoped software validation, not a new
workspace-wide native or physical qualification result.

The first full application command exposes three preexisting `native_live`
acceptance failures. A separate baseline run without the new Quit module
reproduces all three: the literal 19-row expectation omits six already-shipped
virtual-clock diagnostic rows and expects physical latency at row18 instead.
The production UI is correct. The test now explicitly checks all six Unknown
rows and physical latency at row24, retaining all age, padding, phase and
generation checks. Actual RED baseline and GREEN corrected runs are retained;
the three focused tests then pass, followed by the full719/4 result above.
No production output is removed, no test is skipped, and no gate is lowered.

## Receipts

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Successful artifacts use prefix `s3-quit-visible-`; tests use `s3-quit-`.
Final remote/local lengths and SHA256 match, and the PNG is visually reviewed:

| Payload | Bytes | SHA256 |
| --- | ---: | --- |
| `result.json` | 38510 | `4C33EB3D94D2712A755D1A02657E69BED50EEAEAD0DA107A4E0A6BC7F8C83944` |
| `02-quit-fully-revealed.png` | 104182 | `27129EF328D096C6CC88D5A8A413B8888FC32B03EEC06AA9FBEF7783F8A66D18` |
| `02-quit-fully-revealed.json` | 13336 | `ED8A63CE634B5E5A2D63E77B62263DAA27FF2C9AEC1721C60605E33A673DFB43` |
| `app.stdout.log` | 2233 | `240B50812EF849E0BD031F7B2963AB1C2F0A549237134E597635CBD9E840A759` |

Runner SHA256:
`126509A943900C8FD0D6F81265526F298685162838D19BB3E6B4C158035BCBFA`.
The final audit is `s3-quit-visible-final-audit-1102.json`. Original 18
preserved-path hashes still match the preexisting baseline; no cache directory,
unrelated draft, remote user process, driver or boot configuration is changed.
No push or memory update. S1-S3 qualification remains incomplete.
