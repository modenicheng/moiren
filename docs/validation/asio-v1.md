# ASIO v1 acceptance

As of 2026-10-18, the production application uses ProfessionalBackend for
explicit professional settings, including ASIO. This is not a new alternative
host or a replacement of the existing engine graph/control architecture.

## Implemented scope

- One native 64-bit Windows ASIO driver session, with capture/render sharing
  one driver, one rate and one buffer. Channels 0/1 are used; mono is adapted.
- Driver-owned PCM encoding; f32/f64 engine processing is independent of the
  native carrier. Explicit driver/rate/buffer requests do not silently fall back.
- Query-only owned capability snapshots: registry identity, driver name/version,
  channel names/types, native buffer rules, four candidate rates, current rate
  and optional driver-reported input/output latency. Query activates and
  initializes the driver, but does not set rate, create buffers or start IO.
- Separate engine-bridge loss counters, callback capture-block drops and native
  render-silence frames (including startup starvation). First invalid ASIO input
  buffer discard is not counted as overflow. Callback counters reset on start.
- Optional latency-change refresh and atomic callback notification handling.
  Callback faults never reopen drivers, allocate, lock or log.
- Reset, resync, rate and buffer notifications use bounded control-side recovery.
  Exact settings remain authoritative; unsupported reconfiguration stays failed.

## Buffer admission

Driver-preferred (`buffer_frames=0`, or no explicit service period) uses the
reported preferred count, including native non-power-of-two periods within
64..4096. Explicit periods use the existing power-of-two menu and remain exact.
Negative-one granularity doubles from minimum; positive granularity steps from
minimum. With zero granularity only the in-range reported preferred size is
admitted. This conservatively supports the observed FL metadata
`min=256, max=4096, preferred=512, granularity=0` without inventing a stepping
rule or substituting a different explicit request.

## Recovery and diagnostics

One automatic reopen allowance is shared by unchanged ASIO node configurations
and engine sample rate for the backend lifetime. New owner generations do not
replenish it. Configuration/rate changes create a new allowance; old tickets and
pending events are fenced. Repeated faults stop visibly rather than looping.
All final native teardown and error aggregation occur outside realtime work.
The reopen event is authorized only after the whole transport (including other
native streams and process-capture auxiliaries) has completed teardown without
a terminal fault. Any caught panic dominates ordinary transfer/cleanup errors.
Failure-state locks are released before auxiliary cancellation callbacks, so a
caught cancellation panic can safely update that aggregate.

The STA owner services queued window messages before the capture-ready fast
path and after any native wait wake, including audio-event wakes. Both points
share a maximum of 32 dispatched messages per wait call. This bounds dispatch
count, not third-party window-procedure execution time, and is not realtime
callback work or a hard shutdown-time guarantee.

The public observation is owner-generation scoped. Driver latency is in native
frames and excludes application ring backlog and DSP latency. Callback silence
or drops are not physical hardware XRUN counts, and driver-reported latency is
not a measured analog round-trip result.

## Repeatable commands

Ordinary tests never activate drivers:

```powershell
cargo test --locked -p moiren-windows-audio --lib professional
cargo test --locked -p moiren-windows-audio --example asio_probe
cargo test --locked --manifest-path crates/moiren-windows-audio/src/professional/rt-check/Cargo.toml
```

The probe requires the exact authorized FL Studio ASIO CLSID. Query-only is the
default. It never records capture or sends captured data to render; optional IO
writes only silence. No OS default-device/volume or driver settings are changed.

```powershell
cargo run --locked -p moiren-windows-audio --example asio_probe -- --driver '{188135E1-7171-3434-854F-01A3C71F3DF9}' --json --trace
cargo run --locked -p moiren-windows-audio --example asio_probe -- --driver '{188135E1-7171-3434-854F-01A3C71F3DF9}' --run-seconds 2 --json --trace
$env:MOIREN_ASIO_HARDWARE_TEST_DRIVER = '{188135E1-7171-3434-854F-01A3C71F3DF9}'
cargo test --locked -p moiren-windows-audio --test asio_acceptance -- --ignored --nocapture --test-threads=1
Remove-Item Env:MOIREN_ASIO_HARDWARE_TEST_DRIVER
```

Native calls are synchronous third-party code. A finite transfer deadline cannot
preempt hung activation/start/stop/dispose/Release; use an external exact-process
watchdog for acceptance. Never abandon a timed-out probe or start duplicate
driver sessions without first confirming the original process has exited.

## Local native evidence: 2026-10-18

Only the installed FL Studio ASIO CLSID above was activated. No Realtek ASIO
session, OS device default/volume change, driver installation or PCM recording
was performed. Every externally timed-out probe was terminated by its exact
task-scoped process handle and checked before launching another.

The full query succeeded: driver version 1; `In 1`/`In 2`, `Out 1`/`Out 2`, all
Float32LSB; current rate 44100 Hz; 44100/48000/96000 supported, 88200 rejected;
buffer min 256, max 4096, preferred 512, granularity 0. Before stream creation it
reported input latency 2790 and output latency 2898 frames. These are query
results, not running-stream timing or physical measurements.

**Audio IO acceptance did not pass.** Initial 30-second query watchdogs expired
inside CoCreateInstance; a later longer query returned. Read-only thread-stack
inspection showed the activation owner waiting in the Windows audio RPC path
(AUDIOSES/RPCRT4); that observation does not establish its root cause.

The real EngineHost test opened the FL driver and attempted silent duplex
processing, but reported `native device stopped transferring frames` with zero
callbacks and no active timeline. It failed, and therefore did not proceed to
the f64 phase. No compatibility success is inferred from successful start.

An owner-only bounded STA message pump was then added and covered by a
deterministic posted-window-message regression (fails before, passes after).
The subsequent direct probe reached setSampleRate, createBuffers (512 frames)
and successful start, but its final stop did not return before the external
180-second overall watchdog; that exact probe was terminated. This did not
establish successful IO or graceful final cleanup. The STA regression fix is
not presented as the solution to the unresolved FL runtime problem.

The final regression also covers capture backlog, repeatedly ready audio
events, and 65 messages dispatched in bounded batches of 32, 32 and 1. These
later fairness refinements were not followed by another native activation
attempt.

Evidence is retained locally under `.superpowers/sdd/`: `asio-v1-fl-*.log`,
`asio-v1-fl-*.stderr*`, and the EngineHost stdout/stderr logs. Empty JSON output
from a timed-out probe must not be interpreted as a successful result. The
remaining acceptance blocker is sustained FL callback/capture/render progress
and graceful stop; native hardware qualification remains separate.
The final process inventory contained no surviving task probe or acceptance
process. No default devices, volume, audio services or installed drivers were
changed to try to force a passing result.

## Final software verification: 2026-10-18

- Final workspace rerun: 1071 passed, 0 failed, 6 ignored, 3 explicitly filtered.
  The three long clock simulations had passed in the earlier unfiltered
  workspace run; their implementation was not changed in this ASIO work. The
  final rerun is not described as a new unfiltered full-suite pass.
- Probe example: 14 passed, including no-callback, startup-only, missing capture
  and missing native-render-progress rejection plus unconditional cleanup.
- Nested professional RT harness: 61 passed. Tests cover real callback
  allocation/deallocation checks, transport-wide teardown ordering, bounded
  recovery, auxiliary cancellation deadlock and panic-dominant aggregation.
- Workspace all-target check and strict Clippy passed. The Windows-audio
  all-target strict Clippy check for `x86_64-unknown-linux-gnu` also passed;
  this is a cross-target compile/lint check, not Linux test execution.
- Workspace rustfmt and `git diff --check` passed. The CI workflow now includes
  the nested harness on both existing OS jobs, but no remote CI or Miri run is
  claimed for this uncommitted change. GPUI-feature acceptance was not rerun.
- A final workspace rerun exposed a pre-existing UI lifecycle test race:
  `try_snapshot()` may legitimately return None while its nonblocking lock is
  contended. The test now uses a bounded two-second retry, matching the existing
  UI acceptance helper; production snapshot/service behavior is unchanged.
  Its focused four-test suite and the subsequent workspace rerun passed.

Final workspace rerun command:

```powershell
cargo test --locked --workspace -- --skip clock_control_prepared_low_inventory_handles_rate_phase_order_and_lateness --skip clock_faults_joint_queue_handles_bursts_stalls_and_pause_recovery --skip clock_startup_joint_queue_matrix_handles_staggered_owners
```

The independent source review's five findings were addressed and re-reviewed;
that inspection is separate from the execution results above. Final logs are
`asio-v1-final-workspace-tests.log`, `asio-v1-probe-final-tests.log`,
`asio-v1-rt-final.log`, `asio-v1-final-check.log`, `asio-v1-final-clippy.log`,
`asio-v1-rt-clippy-final.log` and `asio-v1-linux-clippy-final.log`, all under
`.superpowers/sdd/`. The user-created `driver/moiren/` template was preserved
and excluded from this work. No staging, commit or publishing was performed.

## Qualification limits

FL Studio ASIO testing is compatibility evidence for that installed driver, not
qualification of a physical native-ASIO interface. Arbitrary channel routing,
multiple simultaneous ASIO drivers, measured physical latency, long-duration
XRUN-free operation and device-unplug recovery on actual ASIO hardware remain
separate acceptance work. Plugin hosting and virtual devices are not ASIO v1.
