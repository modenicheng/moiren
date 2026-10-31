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

## Target acquisition

The isolated release EXE is7044096 bytes, SHA256
`410BE9F5419489B4EC7AC651F1E5435D13982D3D94316D9B5A46BD6FFEA44304`.
It is staged side-by-side at
`D:\MoirenTest\20261031-mic-retirement-order\bin\e2e_latency_probe.exe`.
The derived runner SHA256 is
`B5E039D62CD975A5365BDDB26C072A600ACEC105C4D0C6A4694C145B0FA68137`;
58/58 root PS5.1 fixtures pass. Native settings/input/driver pins are unchanged
from the previous diagnostic collection.

An unrelated remote `moiren-app.exe` PID13256 is present at UTC02:45:37;
it is preserved, and a bounded read-only wait confirms no owners at02:47:09.
Only then does the guarded runner start. Target evidence and staging receipts
live under
`D:\coding\moiren-evidence\20261031-mic-retirement-order`.

The single acquisition starts UTC02:47:38.0388947 and retains a terminal
receipt at02:48:14.5816875. PID12332 exits0, elapsed34.1638366 seconds.
Runtime collection and cleanup complete=true, both failure fields=null;
outer `DiagnosticCollectionPassed=true`. Engine ends Closed/Dormant with
no fault/cleanup error and final native summary failure=null. This is a
bounded operational collection/cleanup result, not complete quality acceptance.

Raw logs first show both gates closed at generation1/gate2 while Running:
Playback lifecycle success at line15 and MIC transport success at17. Later
worker queries see already-closed gates (Playback gate6/Closed, MIC gate3/Paused).
No close-confirmation rejection is logged. This is consistent with the intended
ordering, but one rebuilt/logged run does not prove every possible teardown
interleave or general production cleanup correctness.

The source acknowledges1440000 scheduled frames,29696 nonzero source frames
and1451040 native submitted frames including480 silent preroll frames.
Recorder retains1456320 stereo Float32 frames/3034 complete480-frame packets,
11650560 PCM bytes,31481 nonzero frames/62962 nonzero scalars,
all finite, bit-identical channels and peak0.0474965088. Sink metadata retains
zero incomplete/missing packets, sequence gaps and nonfinite samples; native
capture flag/gap/drop/discontinuity counters are zero. These are storage and
sample observations, not marker recovery or continuous waveform qualification.

**Quality remains incomplete.** The report's `quality_passed=true` means no
fault was latched by its existing active checks; `quality_complete=false`
is explicitly retained by the existing schema/code, not a new result invented
from the successful cleanup. Playback classification reaches final-after-join
with32 initial-prime muted frames,zero classified steady delta/xruns, but
retains15 unknown observations. No field is changed to make this look complete.

Moreover, MIC worker-tail logs85-86 and the final `vad_microphone_cleanup`
Removed snapshot retain **silence_packets=1**, while the last active provider
snapshot and successful MIC Running/gate2 close snapshot have0. Thus the
increment is observed after successful gate closure and before the later
retirement query, not assigned to a specific callback or recorded PCM sample.
The existing quality checks do not revalidate this final cleanup snapshot.
Both cleanup telemetry snapshots retain epoch_resets=1 and
recovery_events=1. They are not cleared or excluded from the raw evidence.
The exact time/cause of the tail silence increment is not established; an
active zero snapshot or the scalar `quality_passed=true` cannot establish
zero faults over the whole acquisition/retirement interval. This run does not
prove the prior early-Running silence problem is fixed.

Catalog observations retain equal volume/mute/defaults; no audio owner or
present VAD child remains, root PnP OK and driver Running. Boot remains
`2026-10-30T09:50:49.6598450+08:00`, also checked after archive at UTC02:50:02.
There is no driver install, physical render, volume/default change or reboot.

## Evidence and remaining gates

ZIP `20261031-mic-retirement-order-target.zip`:519821 bytes,21 payload files
plus manifest, SHA256
`974A96EB3624B3243A727C3B18259BE5FA64049B161311E4508B0094E9113F41`.
Root verifies all ZIP CRCs, exact member set, manifest lengths/SHA256 and
extracted bytes. Independent audit verifies the same archive, pins, metadata,
sample statistics, successful cleanup and retained tail faults. Its receipt
`independent-evidence-audit-20261031.json` has SHA256
`2D9283DFAF3396438774BA17E198FC5E60999F82530418767B8A69946B6B0EFB`.
No ETL decode/loss qualification is claimed. Original target evidence remains
retained.

Continuous changing-waveform delivery, full quality coverage, S1-S3 and
calibrated physical sub-10-ms acceptance remain open. No experimental source
is merged wholesale into main or described as an accepted driver/latency fix.
