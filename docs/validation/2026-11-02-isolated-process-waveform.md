# Isolated Process waveform diagnostic - 2026-11-02

## Scope and parallel execution

Continue S1-S3 application development alongside bounded native tests on
`LAPTOP-NP49RG6H`, never on the development host. First-packet investigation
remains deferred. Physical recording/calibration equipment is unavailable;
that limits qualification, not feature development. No budgets, defaults,
admission rules or quality thresholds are relaxed by this slice.

The desktop now accepts exact measured input bounds (local `9a19aa8`), following
independent render queue settings (`a571467`) and exact shared native periods
(`4278914`). This slice additionally verifies all three activation paths reject
invalid current text rather than reuse a last-valid draft.

## Acquisition and provenance

Remote base: `D:\MoirenTest\20261102-s3-isolated-waveform-0128`.
Task: `Moiren-S3Isolated-20261102-0128`, Limited Interactive, task lifecycle,
six-minute scheduler limit. It finishes normally by01:27:21+08:00; it is not
left running. Status-query SSH timeouts are preserved, not treated as permission
to relaunch. A later compact filesystem query confirms the completed receipt.

Application: `D:\MoirenTest\20261102-s3-exact-value-0115\bin\moiren-app.exe`,
33930752bytes, SHA256
`604B598519405FB7E8C1B70B47E680063E3BBBFF6965EFCE44F06D0F9313AC4E`.
`s3-exact-value-source-manifest-0115.json` records source HEADa571467 plus exact
retained dirty sources; this is not a clean commit-to-binary claim.

Route and recorder:

- Python float32 stereo48k renders to the existing separate
  `Voicemeeter In 2 (VB-Audio Voicemeeter VAIO)` endpoint,
  `{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}`.
- Moiren captures that Python process tree, passes a unity Bus, and renders to
  DAWN PRO2 `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`.
- A task-owned portable OBS records two separate lossless PCM24 stereo48k
  tracks: Python Process reference and DAWN output. Monitoring is off, sync
  offsets0, source gains1; user OBS is not reconfigured.
- Before Moiren starts, a distinct deterministic two-second broadband control
  is recorded to test unintended direct routing. The main32-second source has
  five seconds silence, sixteen seconds changing bandpassed noise, two seconds
  silence, four seconds log sweep and five seconds silence. Silence is explicit
  stimulus, not automatically counted as an audio failure.
- Main source writes1536000frames/3200blocks; reported source underflows are0
  both before and during the main signal. Maximum blocking write is24.9945ms.
  Source stderr retains PortAudio JACK/ASIO enumeration and subnormal warnings;
  the selected active stream is WASAPI, not JACK/ASIO.

Monitor requests70seconds, measurement10seconds, Engine64, Shared period336,
render target512/wake64/callback64, input capacity16384/target1536/jitter96.
The native report confirms period336, buffer/render capacity738, render
target512 and input prime1536/capacity16384. It uses IAudioClient3 without
silently substituting a different request.

### Outcomes, including faults

- Monitor/source/owned OBS all exit0 without forced termination; run and cleanup
  complete. No owned processes remain. Original OBS12648/QQMusic15096 identity,
  default roles and device master controls are unchanged.
- Final Process counters retain7001 gap observations, maximum11.0912ms,
  **one gap-bound violation**, no packet-bound violations, dropped frames,
  bridge silent frames, underruns or resets. Startup prime discard384 is
  recorded separately; it is not erased or investigated here.
- OBS logs an initial Process window lookup retry, source invalidation/retry
  and42ms of dynamically added audio buffering. These recorder-side events
  remain part of measurement uncertainty, not calibrated Moiren latency.
- MKV: `records\2026-11-02 01-25-53.mkv`,48952506bytes, SHA256
  `1BC5A58C7833A24D645B88545EF27D34FAC7D7C20A0479653FFC463DA87D3894`.
  Both tracks decode to2640896stereo frames, no nonfinite/clipped samples.
  Identical stereo source channels are not independent physical references.

## Analysis and visualization

`scripts/obs-isolated.py` adds a separate one-path diagnostic rather than
misusing the existing direct-plus-return comb analyzer. It reuses the validated
bounded signed NCC in `scripts/obs-two-path.py`; it opens no audio devices or
processes. It requires exactly two PCM24 stereo48k tracks and preserves refused
windows. It does not resample implicitly or shift tracks to manufacture zero.

Fixed gates: source/reference NCC0.99, return NCC0.95, competitor ratio1.8,
raw/highpass/stereo agreement0.25ms. A control can only clear when its exact
changing reference is established, its bounded return interval is complete,
there is no supported correlated leakage and its return RMS/peak stay below
1e-6/1e-5. This bounds only the retained control interval and amplitude floor,
not all future routing.

Actual result:

- Main noise/reference correspondence: NCC0.999999999998 at frame1161034.
- Control/reference candidate: NCC0.981944156251 at frame133834, below0.99.
  Therefore **control correspondence and isolation remain refused**. The plot
  shows no visible DAWN signal during that interval, but visual quietness is
  not substituted for the failed exact-control gate.
- Only6of28main-noise windows meet all support gates. Their diagnostic OBS
  relative offsets are47.4583..47.5208ms (p50=47.4792ms,p95=47.5156ms).
  These selected windows are not a whole-run distribution, not qualified
  isolated latency, and emphatically not physical DAC/ADC latency.
- Most windows retain weak correlation and varying RMS. This is a genuine
  unresolved waveform/clock/SRC correspondence issue; no cause, whole-waveform
  continuity pass or lossless verdict follows from zero software drop counters.

The four-panel overview retains full level, control log-level, raw/highpass
correlation and supported offsets. Analysis runs beside the closed MKV on the
remote target; only compact JSON/PNG/receipts are downloaded. Raw MKV/PCM stays
remote, avoiding a roughly49MB media transfer.

First analysis `analysis-0143` is preserved. Read-only review discovers a report
crash when a track is truncated: an incomplete row has empty correlation lists.
A failing render regression reproduces `IndexError`; the fix renders the row
as refused instead of indexing a missing channel. Final analysis reruns on the
same immutable recording under `analysis-final-0209`, without new audio clients.

Earlier separate attempt0122 uses already-muted Realtek, whose PortAudio open
fails `Invalid device [-9996]`; no Moiren owner starts. Source exits1, owned OBS
exits0, no remaining owners or changed controls. It is preserved rather than
treated as a route result or repeatedly retried.

## Software regression and review

New analyzer suite:11missing-implementation failures observed first;11pass
after implementation. Review's truncated-report test fails before the fix;
all12pass afterward on host and target. Controls cover exact/signed/negative
offsets, quiet/correlated/unrelated controls, missing/truncated PCM, ambiguity,
stereo disagreement, silence/tone, drops/repeats, drift and epoch-like silence.
Even supported windows always retain physical/continuity qualification=false.
Existing two-path analyzer's14regressions also pass.

Two new headless desktop tests cover live-window and evaluated activation.
All three exact fields are tested invalid, followed by valid activation as a
positive control. Live policy is selected before typing; direct dispatch then
occurs without an intervening explicit frame. Evaluated activation starts with
a matching test-only admitted predecessor token before the invalid edit.
The methods' visibility is restricted to Settings solely for direct event tests;
production admission and dispatch bodies are unchanged.

Removing the shared text-conflict return makes all three invalid-text tests
fail; the guard is then restored byte-for-byte. The first mutation attempt
fails compilation due unused validation and is preserved, not counted as a
behavioral RED. A second compiling mutant produces the actual failing tests.
Read-only review also corrects the initial live test's unnecessary rendering
between edit and direct activation. No production bypass is found.

Final focused/full verification and artifact pins are recorded in the closing
receipt. Workspace Rust results from the preceding exact-input feature belong
to that revision; this analysis/test slice does not relabel them as a new full
workspace run. No native quality or calibrated physical pass, lower-default
promotion, driver change, push or unrelated-file staging follows.

### Closing receipt

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Final app library with `gpui-test`:617passed/3ignored, strict workspace/all-target
Clippy, rustfmt and diff checks pass. All eight Python analyzer suites pass
131tests (including the new12 and existing two-path14). Final target analyzer
suite passes12. These are software tests, not131audio-device runs.

Logs: `s3-exact-alternate-app-reviewed-0211.log`,
`s3-isolated-clippy-reviewed-0211.log`,
`s3-isolated-preserved-and-python-tests-0216.json`,
`s3-isolated-final-analysis-0209.stdout.log` and
`s3-exact-alternate-mutant-red-0203.log`. The18unrelated retained files all
match their preceding source-manifest hashes and are excluded from staging.
Final analysis JSON/PNG match the initially downloaded and visually reviewed
artifacts byte-for-byte; the final analysis does not require another media
transfer.

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| Remote `analysis-final-0209/summary.json`, local `s3-isolated-summary-0145.json` | 53902 | `08DC7AD75CE3BFBA920CAF9FEFC3EF4874EE3693E18F60A6C041C8BF831E2A52` |
| Remote `analysis-final-0209/overview.png`, local `s3-isolated-plot-0145.png` | 326814 | `80A79E2DF8688CEEE5EF771A0BA888F1CD46F61D7955868081FBA0008542A6DD` |
| Remote `capture-result.json`, local `s3-isolated-result-0145.json` | 8677 | `D13129915E5CF5BFD3C7CEAA95A8D7A0E262D394EBA6B006368D0361B44F99B5` |

S1-S3 remain incomplete. The next measurement improvement is a recorder-ready,
steady-state control and explicit drift/SRC model comparison on retained PCM;
it is not a reason to block independent application development or resume the
deferred first-packet investigation.
