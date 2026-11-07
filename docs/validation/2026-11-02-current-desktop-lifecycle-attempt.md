# Current desktop lifecycle attempt and stopping boundary

Date: 2026-11-02, Asia/Shanghai. Tested source: clean `78967f4`.
This increment preserves two failed harness attempts. Neither establishes a
current native desktop Start/Stop/post-audio Quit PASS or a product regression.

## Software checkpoint and deployment

An independent `git archive HEAD` excludes all eighteen preexisting dirty-path
changes; tracked files remain at their committed versions. Host execution is
limited to compilation and headless tests, never native audio or the desktop.

| Command | Result |
| --- | --- |
| `cargo build --locked --release -p moiren-app -j1` | Exit 0, 2m15s |
| `cargo test --locked -p moiren-app --features gpui-test --lib measured_ui_tests` | 20 passed, 0 failed |
| `cargo test --locked -p moiren-app --lib cleanup` | 9 passed, 0 failed |

An earlier `rejects_failed_measured_baseline` filter selects zero tests. Its log
is retained but is not coverage. The preceding full
[integration checkpoint](2026-11-02-s1-s3-integration-checkpoint.md) is historical,
not rerun by this increment.

The exact new executable is deployed, not demonstrated operational by these
attempts:

```text
D:\MoirenTest\20261102-desktop-lifecycle-1222\bin\moiren-app.exe
SHA256 4C96B3BB7284AC3721616863FE0D83BC13798925E00E85F996BBE5A30E9D51C4
```

The existing running SYS remains
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
No driver install, device-control change, new recording or OBS action occurs.

## Fixed native question

The missing acceptance surface is the current professional desktop, separately
from the working production CLI: Start, confirm the exact native/source/active
configuration, Stop with joined owner, then Quit with normal process exit.
Earlier [stopped-only Quit](2026-11-02-desktop-quit-viewport.md) and
[Process CLI](2026-11-02-process-cli-packet-capture.md) evidence cannot substitute
for it.

The predeclared single retry requests a finite 12-second Process Compatible
smoke: exact source process creation identity, unity Bus, DAWN PRO2, 48kHz/F32,
Engine block64, Shared requested336/actual capacity738, render512/wake64/
callback64, Process prime2048/capacity8192. Its isolated project has no exports;
source and app identities are held. It records no waveform and cannot establish
quality, latency, custom-owner replacement or lower-setting qualification.

## Failed attempts remain failed

Both execute only on authorized remote `LAPTOP-NP49RG6H`.

| Remote suffix under `D:\MoirenTest` | Retained observation |
| --- | --- |
| `20261102-desktop-lifecycle-1158` | Task launches at 12:10:21. The worker appears about 235 seconds later, after the supervisor's intended launch interval. Task reaches its four-minute cap. The late worker fails before source/app launch: PowerShell 5 marshals the null backup passed to `File.Replace` as an invalid path. `result.json` stays stale; `result-next.json` and stderr preserve the failure. No source/app owners or audio actions are recorded. Worker later exits without forced cleanup. |
| `20261102-desktop-lifecycle-1222` | Corrected retry launches at 12:29:46.975. Polls show only owned supervisor PID26048, creation Unix-ms1793593786986; no worker receipt, run directory, source or app. By 12:34:03 it is Ready/result267014 after its four-minute cap. Final audit still finds no first supervisor receipt, result or owned process. No child/audio launch is established. |

The retry failure is localized only to before the first supervisor receipt and
child launch, not definitively before PowerShell script entry. Its exact startup
delay cause is unknown. The supervisor's internal 150-second worker deadline
is not established as entered; the scheduled-task four-minute cap is terminal.
Observation timeouts alone are never treated as task completion or permission
to relaunch. There is no automatic third native attempt.

Before the retry, the harness uses an explicit nonempty atomic-replace backup,
held `Diagnostics.Process` launches rather than `Start-Process`, redirected
asynchronous live logs, actual production fault strings, and bounded cleanup
without UIA calls in `finally`. Remote no-audio checks cover three atomic JSON
generations, a direct probe `--help` child exiting0 in232ms, and concurrent live
log visibility. PowerShell 5 parses all scripts and deployment hashes match.
Independent review finds no concrete retry blocker. These checks do not convert
the failed native attempts into passes, nor prove the interactive startup cause.

## Closure and evidence

The 12:35:12 read-only terminal audit confirms no native/analysis/owned shell
leftovers. OBS PID12648/creation1793505454949 and QQMusic
PID15096/creation1793504036422 are unchanged. Default endpoint roles and all
catalogued render volumes/mutes match the earlier before-catalog; driver remains
Running with the pinned hash. No image-name process cleanup is used. An earlier
read-only inspection helper is stopped only by its exact owned identity; its
separate receipt does not represent audio cleanup.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-desktop-lifecycle-20261102-1158`.

- `build.log`, `measured-ui.log`, `cleanup.log`, source archive and deployment
  manifests retain the clean build and headless checks.
- `failed-native/` and `failed-native-manifest.json` preserve the first attempt's
  original files with verified lengths/hashes, including stale and next results.
- `retry/` retains immutable deployed scripts, input pins, no-audio self-checks,
  launch/polls, terminal observation and final read-only audit.
- `preserved-before.json` and `build-and-preservation-verified.json` retain all
  eighteen unchanged dirty-file hashes. Both original Python cache directories
  are left untouched.

## Development decision

A bounded source/original-requirement review identifies no additional mandatory
S1-S3 software implementation gap. This failed harness does not justify changing
production audio, shutdown or defaults. Current desktop lifecycle acceptance
remains open; retained waveform refusals remain Process18/19 and VAD17/19, and
calibrated physical qualification remains unavailable. Historical native faults
are not erased. S1-S3 overall are therefore not complete.

Do not loop on unchanged soaks, detector fits or startup instrumentation, invent
S4 work as S1-S3 completion, or delay independent product development solely for
missing measurement hardware. First-packet work stays deferred and retired-PCM
fail-fast stays withdrawn. Future remote long tests may overlap development only
for a concrete new regression/acceptance question, with pinned binaries, isolated
state, bounded load/duration and exact-owner cleanup. Offline analysis must not
contend with a native acquisition. No default promotion, push or memory update.
