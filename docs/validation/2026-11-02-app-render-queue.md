# Application render queue configuration - 2026-11-02

## Scope and development policy

Continue S1-S3 application work while finite native tests run on the remote
target. The host performs builds, static inspection and headless tests only.
First-packet investigation remains deferred by user direction. Physical
calibration is unavailable and does not block this configuration slice.
Compatible input and FullCapacity output remain the defaults.

## Implementation

- Persist optional `transport_latency.render` with independent target, wake
  margin and callback budget in output-rate frames. Portable validation checks
  bounded shape; native preparation still requires the actual period, capacity
  and complete deadline. Explicit requests never fall back to FullCapacity.
- Both Windows f32/f64 application factories pass the existing
  `RenderQueuePolicy` into the professional backend. No render algorithm or
  whole-wake consumer-demand guard is changed.
- CLI accepts the complete `--render-target-frames`,
  `--render-wake-margin-frames`, `--render-callback-budget-frames` tuple.
  Measured Process collection retains this output policy while its input
  baseline remains Compatible.
- Desktop settings expose the independent render fields and requested/active
  summaries. Save does not restart a running owner; explicit restart waits for
  matching active confirmation before committing preferences. Draft rebase
  treats render intent as an independent conflict domain.
- As with existing input transport edits, saving a render-only change clears
  the requested measured Process selection. The running owner remains active
  until restart. The footer now explicitly warns that render changes need
  fresh admission; it must not reuse a finite-window proof bound to an earlier
  consumer envelope. Set render intent first, then measure/admit input again.
- Regression coverage includes persistence, invalid/native refusal, CLI field
  independence, draft merge/conflict, stopped save/reset and running
  save/restart/active confirmation.

## Pinned native short test

Remote `LAPTOP-NP49RG6H`, task lifecycle, isolated APPDATA/TEMP. Exact route:
`Process Loopback -> unity Bus -> DAWN PRO2`, 48 kHz Float32. The source also
renders directly to DAWN; this is mixed-output operational telemetry, not an
isolated waveform acquisition or physical latency measurement.

- Remote executable:
  `D:\MoirenTest\20261102-s3-render-short-0013\bin\moiren-app.exe`.
- App SHA256 `5EA44D9A19C4FEB9156C73AB54842E8193AC7F17F7E9E948C6AD7D88AAAF45AB`,
  33932288 bytes; HEAD4278914 plus the exact dirty-source manifest.
- Source probe SHA256
  `3FB5330D790767F68B0114162F137E685AE472D17717CF5C4967D8EFA81A2E28`.
- Loaded611 driver remains Running, SYS SHA256
  `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
- Client3 requested/actual period336, native capacity738, render target512,
  deadline464 = period336 + wake64 + callback64. Engine64 is independently
  confirmed; no fallback or Windows conversion is reported.
- Prepared Process demand704 = render512 + allowance64 + 2*Engine64.
  Measured delivery gap531 gives modeled minimum1333 = demand704 + gap531 +
  jitter96 + lookahead2. Target1536/capacity16384 is admitted exactly.
  This equation is reserve admission, not an additive latency measurement.
- Baseline session1 is replaced by session2. Native owner generation1 is
  scoped to its individual session, not a globally increasing owner counter.
  The active measured profile preserves origin session1/owner1/revision2 and
  the exact source PID/creation identity.
- Running20001 ms, 2000 packets/960000 captured frames. Zero dropped/silent
  frames, underruns, discontinuities, timestamp errors, resets and backlog
  recovery; render writes960800 frames with zero native dropped/silent frames.
  Startup priming2560/prime-discard384 are retained separately.
- Three delivery-gap bound violations remain visible (maximum110807*100ns,
  versus measured531-frame bound). This is operational completion, **not a
  clean sustained-delivery or qualified low-latency PASS**. A short measured
  prefix is not a future-service guarantee. No smaller default is selected.
- App/source exit0, joined-owner cleanup complete, no forced termination,
  cleanup error or remaining test owner. Original OBS12648 and QQMusic15096
  identities are unchanged.

Root evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Manifest: `s3-render-source-manifest-0013.json`. The raw report and result are
downloaded and length/SHA256 checked against remote closed-file manifests:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-render-short-0021-monitor.json` | 1580130 | `85D5855EDD4D75F813969127BE2DEFA20D0E9877121BA532640A45D23DC04FEE` |
| `s3-render-short-0021-result.json` | 14733 | `D0D48DCCF2E6C1165F2C8035AEE8DA8970445DF4A1A112E91125B6BAEB60E4C5` |

The first launcher refuses a hash calculated before Windows line-ending
serialization; no test child starts. The corrected launcher pins the actual
uploaded bytes. JSON extraction now bounds the first line-start opening brace
through the last line-start closing brace and validates schema plus process
PID/creation identity, preserving surrounding logs separately.

## Parallel longer run and software integration

A nine-minute remote task uses the same pinned app/configuration and a
ten-second measurement prefix while local workspace checks, review and
development continue. Remote directory:
`D:\MoirenTest\20261102-s3-render-long-0018`.
Started00:13:50 and finished00:23:02+08:00 on2026-11-02. Actual Running540000ms,
requested/active contract exact, app/source exit0, normal joined-owner cleanup,
no remaining owners or cleanup errors and unchanged original OBS/QQMusic.

- 54000 packets/25920000 captured and published frames. Zero dropped/silent,
  nonfinite, underrun, discontinuity, timestamp-error, reset, backlog/age/ingress
  recovery and low-water-guard frames/events.
- Render writes25921568frames; zero native dropped/silent frames and zero
  native wait timeouts. No retained native fault.
- Delivery gap bound533frames, modeled minimum1335frames and target1536.
  The subsequent run has35 gap-bound violations; maximum120258*100ns =
  12.0258ms host publication interval. Therefore operational/configuration
  completion is true but `telemetry_clean` is false. Do not call this a
  qualified stable smaller preset or attribute gaps to lost audio without PCM.
- Startup priming2560/prime-discard384 remain separate. Input occupancy
  min898/max1987 is telemetry, not measured physical residence or latency.
- The final256 queue observations span only0.3359816s and the final256 producer
  events span2.5495327s. Their earlier histories were overwritten404769 and
  53744 times respectively; the retained tail has no gap violation but cannot
  erase the35 whole-run aggregate violations.

Closed result15289bytes/SHA256
`3E96FB680EE14C8D2190EB32A2A6A50498587CC06935060277CD33E480F85934`
is downloaded and verified. The full1591491-byte report remains remote, pinned
SHA256 `A9853455E5DA5D471124A0023011903DFAE1347941D0C5DE41C8F1247CED457B`.
Analysis runs on the remote isolated OBS-analysis venv without transferring the
full report or PCM. The initial attempt uses a venv without matplotlib and
fails before generating outputs; the existing plotting venv succeeds without
changing the global environment.

Remote `operational-telemetry.png` and `.svg` visualize bounded queue/packet
tails and whole-run aggregate faults, explicitly labelled as nonphysical
telemetry. Downloaded/visually checked PNG423019bytes/SHA256
`1092D54B5821BDFA6E04D89BAFF9D71598C81360114792C1E44C1FDDF02381A5`:
`s3-render-long-0029-operational-telemetry.png` in the root evidence directory.

## Final executable and F64 control

The final app includes the fresh-admission warning/review regressions, SHA256
`F28E503C46233FF9D5CEBDB55815721AA3E376AFD7A84DA8FDED87ECA2F9ADA8`,
33937408bytes. Manifest: `s3-render-final-source-manifest-0030.json`.
Usable remote executable:
`D:\MoirenTest\20261102-s3-render-final-0031\bin\moiren-app.exe`.

A separate20-second final native control explicitly selects F64. Active
precisionF64, Client3 requested/actual336, render512/deadline464, Engine64 and
measured Process1536/capacity16384 are all confirmed. Measured delivery gap534
gives modeled minimum1336. Captured960000frames, zero dropped/silent/underrun,
discontinuity, timestamp-error, reset or recovery counters; one gap-bound
violation remains (maximum11.467ms). This verifies F64 factory propagation and
operational lifecycle, not clean sustained service or a new latency result.

App/source exit0 and joined-owner cleanup complete. Closed result14737bytes,
SHA256 `F8DE8269B526EC57F9AE789D75D512A45B7D062DD5059D581687EDF3775483A4`,
is downloaded/verified. Remote full report1580126bytes, SHA256
`B6D70E61D6575D73D44BA4685BFE246A58F617E5CCB529270892121349B4AC75`.
Final00:29:40 target audit finds all three exact task names Ready/result0,
no remaining app/probe owners, unchanged original OBS/QQMusic and unchanged
Running611 driver/SYS hash. No host audio/device/process operation or push.

## Fresh software checks

| Check | Result |
| --- | --- |
| Workspace tests after configuration changes | 2087 passed, 8 ignored; exit0 |
| Final workspace tests after warning review fix | 2088 passed, 8 ignored; exit0 |
| Final app library with `gpui-test` after warning review fix | 612 passed, 3 ignored; exit0 |
| Render configuration/transaction focused tests | 6 passed; exit0 |
| Render-only invalidation/warning | 2 passed; exit0, UI RED/GREEN retained |
| Final workspace/all-target strict Clippy | exit0 |
| Final Release app build | exit0 |
| rustfmt and diff check | exit0 |

Workspace logs `s3-render-workspace-0019.log` and
`s3-render-final-workspace-0034.log`; final app log
`s3-render-final-app-0027.log`. Clippy and Release logs:
`s3-render-final-clippy-0027.log`, `s3-render-final-release-0027.log`.
The final workspace and612-test app suite both include the review-fix source;
the visible warning test runs only with `gpui-test`.

Read-only review identifies the render-only measured-intent invalidation as a
potential surprise. The resolution retains the conservative existing
invalidation contract, explicitly warns about render changes/fresh admission
and adds both draft-confirmation and visible headless UI regressions. No
native profile is silently reused for a changed consumer envelope.
Final read-only assessment: no remaining concrete findings in the reviewed
slice. Review receipt: `s3-render-review-0035.json`. All18 preexisting unrelated
dirty-file hashes remain equal to their initial source manifest, including
the retained `gate.log`; they are excluded from this local commit.

S1-S3 remain incomplete: no calibrated
physical threshold, defensible hardware lower bound, isolated continuous
waveform or matched VAD/Process comparison is established by this slice.
