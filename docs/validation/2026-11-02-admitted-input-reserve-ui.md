# Desktop admitted input reserve - 2026-11-02

## Bounded application change

Native Diagnostics already shows effective prime/capacity and live delivery
faults. It now also projects the actual prepared queue's modeled minimum,
low-water and high-water values, explicitly in input frames and labelled
`modeled reserve, not latency`. It does not recalculate admission from live
occupancy, infer physical delay or change the running owner.

The Compatible queue has no modeled minimum; it stays `Unknown`, not0 or an
invented timing estimate. Absent queue data adds no row. The existing Running/
session-generation/observation-running filter still hides retired or stopped
observations. English and Simplified Chinese preserve units, placeholders and
the measurement limit. Existing diagnostic accessibility nodes expose the row.
No DSP/RT/transport algorithm, buffer default, admission gate or settings
workflow changes.

## Fresh software checks

Three regressions first fail because the row/translations are absent, then
pass after implementation. The Compatible expected water marks are corrected
to the existing authoritative2/8191 values, not production changes. Final tests
also cover stale generation, Idle, running=false and missing queue explicitly.

| Check | Result |
| --- | --- |
| Focused native-live row tests | 11passed |
| Final app library with `gpui-test`, `-j1` | 620passed,3ignored |
| `cargo test --locked --workspace -j1` | 2092passed,8ignored |
| Workspace/all-target strict Clippy | exit0 |
| Release app build | exit0 |
| rustfmt and diff check | exit0 |

Logs in `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`:
`s3-input-reserve-red-0212.log`, `s3-input-reserve-green-0213.log`,
`s3-input-reserve-final-app-0216.log`, `s3-input-reserve-workspace-0216.log`,
`s3-input-reserve-final-clippy-0216.log`, `s3-input-reserve-release-0216.log`.
Read-only review of the4source/locale files finds no substantive defect;
execution results are checked independently.

The [1344-frame candidate](2026-11-02-process-input1344.md) runs remotely in
parallel with development and headless checks. Its unchanged binary and
operational-only result are not claimed as verification of this new UI row.

## Pinned remote UI verification

Executable: `D:\MoirenTest\20261102-s3-input-reserve-0222\bin\moiren-app.exe`,
33932288bytes, SHA256
`611D172B25BBF51C49B8720277AA218C334FA5797D78FD615FB1B31DF315C138`.
Manifest `s3-input-reserve-source-manifest-0222.json` retains HEAD8075482
plus exact dirty sources, including all unrelated retained files. It does not
claim a clean-commit binary. Only the relevant source/docs are staged locally.

A bounded remote-only Limited Interactive task uses isolated APPDATA/TEMP,
existing deterministic source, Process -> unity Bus -> DAWN and explicit
Shared336/render512/Engine64/measured1536. It checks the accessible diagnostic
row after native Apply, then attempts native Stop/Quit before evaluating any
display mismatch. Original OBS/QQMusic, defaults and render controls are
preserved. Terminal evidence is appended after the task completes; no native
pass is claimed from launch or from the headless projection tests.

### Terminal native receipt

Task `Moiren-S3InputReserve-20261102-0222` completes02:23:59+08:00. Exact
requested/active measured1536, Shared336, native738, render512 and Engine64
are confirmed after Apply and a30-second active hold. UIA reads:

`2 Application input: admitted input reserve: Minimum 1334; low 706 / high 15904 input frames (modeled reserve, not latency)`.

This is a native accessible-text confirmation, not inferred from source or
headless tests. The retained screenshot shows the first diagnostic viewport;
the new row lies below that viewport. Although UIA reports Offscreen=false,
its bounds extend below the window, so screenshot visibility/viewport placement
is **not** claimed. Existing accessible diagnostic rows have the same retained
tree behavior; no screenshot claim substitutes for actual visible pixels.

Native Stop/Quit and app/source exit0, no forced termination, error, diagnostic
error, cleanup error or remaining owners. Task Ready/result0; original OBS/
QQMusic, defaults, render volume/mute and boot identity remain unchanged.
The accessible sampled delivery row shows0gap violations at that observation;
this is not a whole-run trace, continuity pass or physical measurement.

Downloaded result247112bytes/SHA256
`A31D4FE06BC26EB4729B21443CBA3644820E6586A552454080588579771B8681`:
`s3-input-reserve-result-0225.json`. Screenshot279823bytes/SHA256
`B59C2CD8E3C28E8EBDEEB3FB91F56C98CDFE457000BDAF53628357FC90C2853D`:
`s3-input-reserve-screen-0225.png`. Both match remote closed-file manifests;
the screenshot is visually reviewed. Accessible tree is separately retained
as `s3-input-reserve-tree-0226.json`. Final02:25:05target audit confirms both
this task and1344task terminal with no owners;205.48GiB free.

All18unrelated retained files remain hash-equal and excluded, per
`s3-input-reserve-preserved-0224.json`. No push, host audio execution, driver
installation, default promotion or memory update. S1-S3 remain incomplete.
