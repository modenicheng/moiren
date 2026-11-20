# Exact Shared device periods and parallel smoke - 2026-11-01

## Development scope

Continue configuration work while bounded native smoke runs on the remote
target. The user's roughly ten-minute practical use without a severe issue
supports deferring first-packet investigation; it does not certify a physical
latency threshold or justify hiding steady-state faults. Physical equipment
availability and exhaustive long-duration qualification do not block this slice.

## Implementation

- WASAPI Shared requests accept integer periods in 64..=4096 frames. Engine
  blocks, Exclusive and ASIO retain their original preset validation.
- CLI, persisted global/device overrides and desktop input preserve exact
  periods, including 336/441/480. Empty input requests the driver default.
- Explicit periods require Client3 capabilities, matching mix format, exact
  initialization and observed-period readback. Unsupported requests fail;
  there is no compatibility fallback or rounding for an explicit request.
  Unspecified periods retain compatibility initialization.
- Desktop invalid text blocks Save; discard/default reset clears it. Device
  text fields retain keyboard focus. Modal Escape still closes the dialog;
  ordinary text events can reach GPUI's platform input handler.

## Fresh software checks

All commands ran in `.worktrees/minimal-io-latency` without starting native
host audio or manipulating existing host processes.

| Check | Result |
| --- | --- |
| App library with `gpui-test` | 604 passed, 3 ignored |
| Windows audio library | 746 passed, 4 ignored |
| Engine `audio_config` | 5 passed |
| Workspace tests | 2083 passed, 8 ignored; exit 0 |
| Workspace/all-target strict Clippy | exit 0 |
| rustfmt check and diff check | exit 0 |
| Release app | exit 0 |

Logs: `s3-period-*-green2-2343.log`, `s3-exact-period-workspace-2348.log`,
`s3-exact-period-clippy-2348.log`, `s3-exact-period-release-2345.log`, under
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

## Parallel nine-minute native operational smoke

Remote `LAPTOP-NP49RG6H`, 23:41:05 to 23:50:05+08:00, exact graph
`Process Loopback -> unity Bus -> DAWN PRO2`, 48 kHz Float32, Engine256.
Compatible input target2048/capacity8192; actual render period480/capacity1056.
This uses the previous budget-diagnostics app, SHA256
`FCF01755B7122ACB82FDC32011D5D7672505348A75F2891AF0358A22AE40D485`,
not the in-development exact-period binary. Build/test work proceeded locally
while the limited interactive remote task ran with isolated APPDATA/TEMP.

- Confirmed Running window: 540000 ms; run and joined-owner cleanup complete.
- Process: 54000 packets, 25920000 captured frames; zero dropped/silent frames,
  underruns, discontinuities, timestamp errors, resets and backlog recovery.
- Render: 25922496 written frames, zero native dropped/silent frames.
  Native wait timeouts20883 are retained as observations, not counted as XRUNs.
- App/source exit0; no force termination, cleanup error or remaining test owner.
  Original OBS12648 and QQMusic15096 identities remain unchanged.
- Startup priming4096 and prime-discard352 remain separately visible; these
  are not a new first-packet investigation or continuous-waveform proof.

The collector initially treated stdout logs plus JSON as pure JSON. Its task
exit1 and error receipt are preserved. Postprocessing the original report,
checking its source PID/creation time and actual duration, establishes the
operational PASS without repeating the audio run. Remote `analysis.json`:
`D:\MoirenTest\20261101-s3-parallel-smoke-2339`. Root receipt:
`s3-parallel-analyze-2354.stdout.log`.

The source also renders directly to DAWN, so this is mixed-device operational
evidence, not clean waveform analysis or calibrated physical latency.

## Exact336 native preparation and measured1024 refusal

Release app SHA256
`28E72C23C7460DA6CC3CEEA7F6673F17E5DCBAA954A2EF70EA554375DC3A1B15`,
33891840 bytes; source HEAD44abd21 plus the manifest-listed dirty edits.
Manifest: `s3-exact-period-source-manifest-2351.json`. Remote executable:
`D:\MoirenTest\20261101-s3-exact-period-2349\bin\moiren-app.exe`.

At23:53:04 the actual app prepares Client3 requested336/observed336, 48 kHz,
capacity738. Capabilities remain default480/fundamental1/minimum336/maximum480.
No Windows conversion or initialization fallback is reported.

Full prepared Process consumer demand1314 = render738 + allowance64 +
2*Engine256. After three seconds of observed delivery, target1024 is refused:

```text
minimum1950 = demand1314 + delivery gap538 + jitter96 + lookahead2
capacity16384 is storage, not the operating target
```

This is reserve admission, not measured latency. App exit1 is the expected
target refusal; source exit0, joined-owner cleanup complete, no native fault,
unchanged original OBS/QQMusic and no remaining test owners. The collector
also encountered a trailing log line after JSON; the original error and raw
stdout remain preserved and bounded JSON postprocessing confirms the contract.
Remote `analysis.json`: `D:\MoirenTest\20261101-s3-exact-period-2349`.
Receipt: `s3-exact-period-analysis-2359.stdout.log`.

Loaded611 driver remains Running with SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
No driver, default role, endpoint volume or external process was changed.

## Next configuration boundary

Exact native period support is verified. Next expose the existing admitted
render-queue policy to application/CLI/desktop users, independently of native
capacity and Process target. Retain actual-period/deadline and whole-wake
checks; do not weaken admission to make1024 pass. No S1-S3 physical completion
or sub10ms result is claimed.
