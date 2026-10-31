# Probe-only virtual-route retirement ordering - 2026-10-31

## Software checkpoint

The [still-open confirmation receipt](2026-10-31-close-confirmation.md)
supports a scoped sequencing experiment, not a relaxed gate check.
`ccf226b` (`ba867e1` independent source) changes only the explicit E2E probe's
`retire_with_route` scheduler and tests in `vad_collection.rs`.

For the exact virtual Playback/Input -> Bus -> MIC/Output route:

```text
Observe -> ObserveFinal -> Invalidate -> CloseGate
-> StopSource -> StopMeasurement -> ShutdownHost -> JoinSource
-> FinishMeasurement -> Retire
```

The two exports are attempted by the existing per-endpoint cleanup helper.
Closing gates before external native Stop/reset avoids intentionally racing
that epoch-changing teardown with an open-gate close request. Both external
Stop broadcasts still precede every join, including potentially blocking host
shutdown. The recorder releases its MIC client before endpoint removal.
Physical-route ordering is unchanged.

No driver, production provider, app routing, PCM transport, buffer default,
quality classification or `finish_close` guard changes. All stages still
accumulate returned errors/caught panics rather than skipping later cleanup.
Actual blocking IO/host joins are not newly bounded by these scheduler tests;
a hung operation must not be reported as completed cleanup.

## Verification

- RED: the actual retirement scheduler with a controlled native-stop/reset
  interleave retains two gate2 -> 3/still-open failures in the previous order.
  This is a scheduling model, not a kernel concurrency proof.
- GREEN:546 example tests pass on current experimental source, independently
  rerun by root; strict example/tests Clippy and formatting pass.
- Regression coverage retains all-stage failure/panic handling, both Stop
  broadcasts before joins, partial-setup unwind, blocked sinks, and blocked
  host shutdown after successful/failed/panicking early CloseGate.
- Eleven existing close-confirmation/error/guard tests also pass unchanged.
  Independent review finds no blocking issue and confirms route-only scope.
- Isolated clean source `7f7287982403468ec55aea1e4f909f03785f3d7c` is the
  previous `960b3c9` probe plus this sequencing patch only. Its534 example
  tests pass; the release build uses host jobs2/shared target.

Host regression logs:
`D:\coding\moiren-evidence\20261031-close-confirmation\mic-retirement-order`.
No later successful software result erases the previous native cleanup or
early-Running MIC silence failures.

## Staged target and open native gate

The isolated release EXE is7044096 bytes, SHA256
`410BE9F5419489B4EC7AC651F1E5435D13982D3D94316D9B5A46BD6FFEA44304`.
It is staged side-by-side at
`D:\MoirenTest\20261031-mic-retirement-order\bin\e2e_latency_probe.exe`.
The derived runner SHA256 is
`B5E039D62CD975A5365BDDB26C072A600ACEC105C4D0C6A4694C145B0FA68137`;
58/58 root PS5.1 fixtures pass. Native settings/input/driver pins are unchanged
from the previous diagnostic collection.

As of UTC02:45:37, an unrelated remote `moiren-app.exe` PID13256 is present.
The probe has not started; that application is preserved. Target evidence
and staging receipts live under
`D:\coding\moiren-evidence\20261031-mic-retirement-order`.

Native collection/cleanup improvement is unverified. Startup silence,
continuous changing-waveform delivery, S1-S3 and calibrated physical
sub-10-ms acceptance remain open. No experimental source is merged wholesale
into main or described as an accepted driver/latency fix.
