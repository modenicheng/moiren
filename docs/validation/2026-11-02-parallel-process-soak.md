# Parallel Process2048 application soak

This bounded remote run executes concurrently with local headless development.
It supports continuing software work without another native stability scan.
It does not qualify physical latency, whole-waveform continuity or a lower
default. First-packet investigation remains deferred.

## Exact environment and lifecycle

Remote target: `LAPTOP-NP49RG6H`, limited interactive session1.
Deployment/evidence: `D:\MoirenTest\20261102-s3-parallel-soak-0440`.
Scheduled task: `Moiren-S3ParallelSoak-20261102-0440`, task lifecycle with a
15-minute hard ceiling. All its native/source/recorder children have exited.

Pinned application:
`D:\MoirenTest\20261102-s2-clock-diagnostics-0342\bin\moiren-app.exe`,
SHA256 `7C40E828DB247589132E480E5D26E451D81C11C542B23921B64C24B42DEC7E35`.
This is the existing source-equivalent cumulative-diagnostics binary, not a
clean-commit build of the new owner tests. Its original exact dirty-source
manifest remains `s2-clock-source-manifest-0342.json` in the host evidence root.

Route and observation boundaries:

- Owned Python Float32/48k source -> separate
  `Voicemeeter In 2 (VB-Audio Voicemeeter VAIO)`.
- Moiren captures that source process -> unity Bus -> DAWN PRO2 render.
- Separate task-owned portable OBS records Process source reference and DAWN
  output capture as two PCM24/stereo/48k tracks. Monitor off, gain1, sync0.
- Engine64, Shared336, negotiated render buffer738, render target512,
  wake margin64 and callback budget64.
- Measured input capacity16384 / target2048 / jitter96; observed calibration
  packet480/gap528; admitted minimum1330 and low-water706 frames. No smaller
  target is attempted and no reserve/default is enlarged.

The source includes a distinct6s pilot,2s silence and entire fixed4s no-Moiren
control before the main sequence. The main sequence is exactly480s:
5s silence +464s changing band-limited noise +2s silence +4s sweep +5s silence.
Signal generation, OBS capture and PCM analysis all execute remotely.

## Retained startup failures and isolation

An earlier attempt remains in
`D:\MoirenTest\20261102-s3-parallel-soak-0410`:

- The limited PowerShell wrapper never reached Python or produced audio
  children. After inspecting the exact owned process/task, only that wrapper
  was stopped; `wrapper-startup-refusal.json` retains the failure.
- Direct Python launch then reached the CLI, which correctly refused the
  harness's660-second request: `--seconds must be 1..600`. The failed capture
  receipt stays failed. Its owned OBS required forced cleanup; this is not a
  successful native/audio result.
- The fresh0440 run uses the existing600-second limit and shorter480s source;
  no application limit or quality gate was relaxed.
- Three previously owned SSH diagnostic PowerShell readers were identified by
  exact decoded command, PID and creation time, then stopped remotely. They
  had remained CPU-bound during PowerShell metadata serialization. No audio
  child, user OBS or QQMusic was stopped, and no development-host process was
  manipulated. Original diagnostic receipts are preserved.

User OBS12648 and QQMusic15096 retain their original start times. Default role
and volume controls are unchanged. Final audit finds no task-owned audio or
analysis process, with176.13GiB free on the remote D volume.

## Native result

The main source submits23,040,000 frames, reports zero signal underflows, and
finishes normally. The application observes600,000ms of running time; run and
cleanup complete, failureNone. Monitor, source and owned OBS all exit0 normally;
scheduled task returns0. The two retained recording tracks each contain
515.3066667s, including preroll/control/teardown.

Final counters after owner join, not only the last running snapshot:

| Observation | Result |
| --- | ---: |
| Native packets / accepted frames | 60,000 / 28,800,000 |
| Packet-bound violations | 0 |
| Delivery-gap-bound violations | 72 |
| Largest observed gap | 11.2579ms |
| Dropped / silent / nonfinite frames | 0 / 0 / 0 |
| Underrun / discontinuity / timestamp errors | 0 / 0 / 0 |
| Resets / backlog or age recovery | 0 / 0 |
| Startup prime-discarded frames | 352, kept separate |

This is an operationally successful application run with **nonzero timing
bound violations**, not a zero-quality-fault PASS. It neither justifies reducing
target2048 nor establishes physical p95. Unknown source age remainsUnknown.

## Remote waveform analysis and visualization

`analyze-0515.py` uses the unchanged, pinned `obs-isolated.py`,
`obs-two-path.py` and repaired `obs-linear-clock.py` from the previous retained
tool deployment. Their SHA256 values match this branch byte-for-byte. Runtime
8.437s; no new native clients. Whole source/control/reference correspondence
searches are limited to a fixed first-minute prefix; the full4s control is never
cropped to select a better interior. RMS working memory is chunk-bounded.

- Entire control/reference correspondence NCC1.0 on both channels, peak
  ratio77.5488; DAWN return is exactly zero over the checked control interval.
  `bounded_control_clear` establishes only that retained pre-Moiren interval,
  not all-time routing isolation. This new result does not rewrite prior failed
  controls from other runs.
- Main source/reference initial-window NCC1.0 on both channels, peak ratio34.6388.
- Eight prespecified source-noise positions1/60/120/180/240/300/360/462s are all
  retained. Three satisfy the existing stereo held-out affine model gates;
  five remain refused. Some high-NCC points still fail residual/stereo gates.
- Selected supported OBS-relative offsets are85.5521,86.4883 and95.8124ms.
  These are three uncalibrated model-window offsets, **not** whole-run latency
  percentiles, a physical latency distribution, actual servo reconstruction or
  evidence of lossless intervening PCM.
- Full retained50ms RMS envelopes plus all eight selected-window NCCs and
  compatible offsets are plotted in `analysis-overview-0515.png`. Refused
  windows stay red. The plot is visually reviewed on the host.

## Artifact pins

All raw media remains remote. Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Compact report/plot downloads are verified against remote byte lengths and
SHA256. The monitor JSON is also retained locally; no raw PCM/MKV is transferred.

| Artifact in remote deployment | Bytes | SHA256 |
| --- | ---: | --- |
| `capture-result.json` | 8938 | `5F823BDDC36E32922572222E6569D976161E22063EAFBC7E7A1D7AFDC37EEF41` |
| `monitor.json` | 1637246 | `F3E12BBE4B1D206C5938338994CE6E15B8ED4280A834E7C7BADEE05443430D9A` |
| `records/2026-11-02 04-11-44.mkv` | 458268120 | `A52D7FB28DFC4BA98816F70B24DB3960D692E86FD09ED7B862E14C97B367C986` |
| `analysis-summary-0515.json` | 19012 | `73AB601E69AC9A7B29FE0E626B79DEFD5D7CC04B1832D02CAF87E2FB83193F67` |
| `analysis-overview-0515.png` | 197060 | `205C91D9C955D9B4478234522455CEF0F86F6D5B291608DC32DD89CB01768EC1` |
| `analyze-0515.py` | 6984 | `37C336EA993960D9F3944A3F53975242DB4E7C2A355C866C5F00C1AE5FAE761E` |

Host receipts `s3-soak-final-audit-0530.stdout.log` and
`s3-soak-pins-0532.stdout.log` retain final process state, native counters,
recording/decoded-track/tool hashes and original user-process identities.
The separate [owner burst tests](2026-11-02-owner-burst-integration.md) are
developed and fully checked while this run executes. No push/memory update or
S1-S3 completion claim follows from this checkpoint.
