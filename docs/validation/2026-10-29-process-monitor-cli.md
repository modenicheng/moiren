# Actual Process monitor application entry

Source: `a4a21d6` on `feat/minimal-io-latency`, 2026-10-29 Asia/Shanghai.
This completes the bounded application-CLI propagation slice of S3; it does
not complete S1-S3 or qualify native audio performance.

## Application behavior

The current binary supports:

```text
moiren-app monitor --process PID --output ENDPOINT --seconds 30
moiren-app monitor --help
```

The native executable name is `moiren-app.exe` on Windows. Replace PID and
ENDPOINT with the source process and the explicit WASAPI output identity.
No output endpoint or source is substituted implicitly. The Process creation
time is resolved before startup and checked again by the existing native owner.
GUI startup and the offline `--gain` path remain separate commands.

An explicit Process input budget is a complete tuple:

```text
--target-frames 1024 --capacity-frames 8192 --jitter-frames 96
--max-packet-frames 480 --max-gap-frames 480
```

These are declared experiment assumptions, not measured delivery evidence.
The 1024-frame example is not a preset or a target pass. Preparation rebinds
the complete actual consumer demand through the existing professional backend
and may refuse it. `--output-buffer-frames` is an independent native output
request; its period never becomes a Process Loopback delivery period.

`--precision f32|f64`, supported `--engine-block-frames`, and
`--output-mode shared|exclusive` use the same audio settings as desktop.
An explicit `--age-limit-100ns` fails visibly before native startup; unknown
Process sample age is not replaced with queue duration or arrival interval.
Omitting all queue options preserves Compatible. Partial/duplicate/unknown
options and unbounded run/startup settings are rejected.

The route is exactly `Source -> Bus -> Sink`, with no Gain/Pan/extra delay
node. An isolated service context is fully prepared before polling starts;
no saved project, device preference or export intent is imported or overwritten.
The actual `ManagedDriver` lifecycle is shared with desktop.

The requested 1..600-second run begins only after matching active generation,
graph revision, audio settings and transport settings are confirmed. Startup
has a separate configurable 1..30000-ms deadline, default 15000 ms. Native
errors, owner/revision changes and unexpected termination retain a failure.
Quality counters alone are not monitor qualification gates. Every exit path
requests service cleanup and joins it; a control deadline cannot interrupt a
blocked native call or join.

The command prints a JSON operational report alongside the existing
application diagnostics on stdout. It retains requested versus confirmed
active intent, process identity, generation/revision, run duration, completion,
cleanup and failure. `qualified` is always false. `last_running_native` is
explicitly a Running snapshot, not a final joined-native counter receipt.

## Cleanup correctness fix

The new stop-failure regression initially showed `cleanup_complete: true`
and no failure after the fake native owner returned an injected stop error.
Previously, `ManagedDriver` reduced cleanup completion to a generation number.

It now reads the surviving Engine observer after dropping/joining the Host
on the existing cleanup worker. `last_cleanup` retains that original service
generation and final Host state separately from live observations. The CLI
checks Closed, cleanup errors, final faults and generation again after service
join, so it cannot lose a final publication between polling and thread exit.
A service thread join is not by itself successful native cleanup.

## Verification and limitations

All checks below run without opening an audio device, driver or VM:

- Real application integration tests: 2 passed. Both command-help regressions
  failed before implementation and pass with the real application binary.
- App library with `gpui-test`: 426 passed, 3 ignored in the final complete
  run with `--test-threads=4`. This includes both precisions, stable identity,
  pure IO/Bus topology, queue tuple/refusal, startup deadline, owner failure,
  generation-bound final cleanup and injected native stop failure.
- App strict all-target Clippy with `gpui-test`, app all-target check with
  `gpui-test`, workspace formatting and diff whitespace check pass.
- One earlier default-parallel full run timed out in the pre-existing
  `repeated_same_candidate_apply_keeps_its_own_future_target_ack_barrier` test.
  Its isolated rerun passed; the subsequent complete bounded-concurrency run
  passed without skipping it or changing any timeout/ACK assertion. This does
  not establish that the earlier timing failure has been diagnosed or fixed.
- Linux cross-check was attempted but stopped in `glib-sys`/`gobject-sys`
  before the application: this Windows environment lacks the Linux
  sysroot/pkg-config setup. Do not record Linux CI as passed. Non-Windows
  native-only imports/helpers are cfg-gated; native Linux CI remains required
  before integration.
- A separate session was running the shared debug `moiren-app.exe`. It was
  preserved. The binary integration tests instead used the explicit Windows
  target output directory, after a shared-output build was refused by Windows.

Logs: `target/s1-s3-20261028/s3-monitor-*.log`. Earlier failures are retained
alongside final successful logs. This slice changes no driver/package or
production buffer defaults. There is no new native-operation, continuity,
VAD/Process comparison or sub-10-ms result, and no new deployment or merge.

## Remaining S3 software requirement

Actual desktop policy propagation and actual app CLI propagation now both
have software evidence. Measured Process delivery admission remains open:
`InputDeliverySnapshot` records packet/gap/burst counters, but a caller's
`Empirical` label is not a generation-bound measured profile.

The next slice must associate observations with stable Process identity and
their originating owner/window, retain clock faults and unknown phase, and
feed them into preparation with complete consumer demand. Capacity must
account for a whole callback drain burst, rather than treating every packet
in a drain as an independently consumed publication. Measure consumer demand
once at the full native render wake, not once per Engine segment. Correlate
producer/consumer timing off RT using bounded observations; missing phase must
retain conservative admission. Apply through owner replacement, with explicit
source/generation revalidation and visible refusal.

Physical and matched-route evidence remains deferred under the user's
2026-10-29 instruction. A measured arrival profile will still not establish
Process sample age, a hardware clock, a steady-state guarantee or physical E2E.
