# Minimal I/O latency implementation and qualification evidence

## Source and scope

- Date: 2026-10-26.
- Branch: `feat/minimal-io-latency`.
- Source base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
- Current implementation commit: `805e4e1`.
- Worktree: `D:\coding\moiren\.worktrees\minimal-io-latency`.
- Reference graph: Input -> Bus -> Output, 48 kHz.
- Target: measured steady p95 <= 10 ms with stable continuous waveform.
- Plan: `../superpowers/plans/2026-10-26-minimal-io-latency.md`.

## User objective

- Reference graph: `Input -> Bus -> Output`, with no other processing nodes.
- Desired result: stable physical output with calibrated end-to-end p95 as close
  to the limit as possible, and no more than 10 ms; p50 <= 8 ms is an
  exploration target.
- A passing window must have zero unexpected drops, silent frames, underruns,
  discontinuities, marker gaps, or epoch/recovery resets.
- Current state: software transport, VAD source-neutral evidence, and the
  production VAD integration path are implemented. The dual-machine native VAD
  smoke has run, but its complete event/timer matrix is not passing; production
  `live-vad`, rename smoke, and calibrated physical measurement remain pending.
- At 48 kHz, 10 ms is 480 frames and 80 ms is 3,840 frames. Audible doubling
  while direct application output and Moiren output are both active is not
  evidence by itself that the delay exceeds 80 ms.

## Current gate state

| Stage | State | Evidence required |
| --- | --- | --- |
| 0 | Software baseline passed | Archived plan d863d0f; tests/check/driver results below |
| 1 | Software foundation passed | 19 marker tests; no physical observation yet |
| 2 | Software foundation passed | 5 transport, 8 budget and delivery-timing tests |
| 3 | Software propagation passed; target native evidence partial | Real preparation/prime and 64-frame demand regressions; target native packet probe ran but is not a complete pass |
| 4 | Software budget/boundary passed; target native matrix failed | Explicit VAD target/capacity tests passed; target event/timer PCM matrix hit `0x8007051A` revision mismatches |
| 5 | Software graph/RT regressions passed; physical timing pending | Compiled Input -> Bus -> Output delay regression; physical output timing pending |
| 6 | App settings/rename tests passed; production/rename live validation pending | Persisted transport settings, stale active state, safe rename and held-client tests; current target native smoke is recorded below |
| 7 | Not qualified | Calibrated physical p95, 30-minute and 2-hour PCM/load runs |

No software estimate, VM result or silent counter run qualifies physical E2E.
Missing hardware/calibration is a missing gate and must not be reported as pass.

## Baseline evidence

- `cargo check --locked --workspace --all-targets`: passed before implementation.
- `cargo test --locked -p moiren-engine -p moiren-windows-audio`: passed,
  including clock control, fault and startup matrices. The Windows library
  baseline was 422 passed / 2 ignored; hardware tests remained ignored.
- `pwsh -NoProfile -File driver/vad/test.ps1`: all six host suites passed.
- `pwsh -NoProfile -File driver/vad/build.ps1 -Analyze`: passed with zero
  warnings/errors, INF valid, WDK 10.0.28000.0.
- Development host: Windows 11 Pro 10.0.26300; FreeDSP, Realtek and CABLE
  present. No present Moiren VAD on this host. Device defaults were not changed.
- Target VM: `driver-test` / `DRIVER-TEST`, Windows 11 build `10.0.26200.9711`,
  reached over SSH as `wdkremoteuser@192.168.111.128` using the dedicated
  `moiren-test` key. AC/DC sleep timeouts are zero and hibernation is disabled.
  The target currently reports `ROOT\MEDIA\0000`, `oem30.inf`, driver version
  `0.36.20.788`. No target driver package file hash was captured in this run.
- No calibrated physical recording arrangement has been established.

## Current target-machine evidence

The current branch was rebuilt before target execution. The target executable
was `vad_native-805e4e1.exe`, source commit `805e4e1`, SHA-256
`2E8EA9C90CC956D5D40E3CA00B9CD36E08B6ADC227704B7D5489716A4C3469AF`.
Evidence was copied to the development host before the target directory was
cleaned; the local archive is
`target/vad-target-archive/20261026-232435/`. The guest now retains only the
current probe executable to conserve its 64 GB disk.

- Target endpoint creation and MMDevice identity mapping reached all four
  expected playback/microphone mono/stereo raw children. The timer run opened
  all eight native cases, including 480-frame playback and 512-frame stereo
  Float32 microphone buffers; native position advanced.
- Event mode exited with `0x8007051A` during playback mono Float32 after the
  playback mono PCM16 case. Timer mode exited with the same revision mismatch
  during microphone stereo Float32 after seven cases. The current complete
  native matrix is therefore **not qualified**; this is target failure evidence,
  not a pass.
- After both failures, the new probe cleanup left no `vad*`/`moiren*` process
  and no dynamic `MOIREN\VAD_RAW` child; only the control bus and physical
  audio device remained. This validates failure cleanup, not PCM stability.
- The target has not yet run the current branch's production `live-vad`
  `VirtualPlayback -> Engine -> Bus -> physical Output` path, a held-client
  rename through the app, or a calibrated physical marker measurement.

## Software changes and focused evidence

- Marker generation uses caller buffers; bounded offline correlation rejects
  silence, clipping, truncation, ambiguous matches and invalid limits.
  `latency_marker`: 19 passed; combined with existing `latency`: 29 passed.
  Module-focused strict Clippy passed. Offsets are sample alignment only.
- Transport input budgets independently resolve capacity, prime/servo target,
  watermarks and modeled minimum from complete consumption burst, declared
  delivery gap, jitter and lookahead. Unknown delivery cannot admit a target.
  `transport_budget`: 5 passed; existing `latency_budget`: 8 passed.
- Configured Process Loopback preparation now attaches its requested queue
  before activation. The regression primes after three 480-frame packets at
  target 1024 (below the old 2048 threshold), preserves zero-position FIFO
  semantics and reports no invented native period. One focused test passed.
- Professional process regression reproduced 256-frame prefetch on a 64-frame
  render (RED). Removing the duplicate FIR/prefetch made it consume exactly
  64 frames and preserve every expected sample (GREEN).
- These are software regressions, not measured Windows callback or E2E results.

## Current implementation conclusions

### VAD driver and runtime

- The driver has a working control ABI for capabilities, stable endpoint IDs,
  CREATE/QUERY/REMOVE, gate admission, PCM16/Float32 mono/stereo streams,
  session/epoch fencing, bounded overlapped requests and asynchronous removal.
- The Moiren provider has a production integration path: lifecycle discovery,
  endpoint creation, native PCM workers, playback/microphone virtual
  boundaries, Engine activation gates, cancellation and stale-epoch recovery
  are implemented and covered by host tests.
- The target machine can create and map the VAD endpoints, but the current
  native event/timer matrix fails on `0x8007051A` revision mismatches. The
  software production path is present; live activation through the complete
  Moiren Engine graph and calibrated VAD-vs-Process comparison remain open.

### VAD rename

- User rename is supported by the app UI and service.
- The driver has no independent in-place rename IOCTL. The safe implementation
  closes admission, drains the owner, removes the old native child, waits for
  observed `Removed`, then recreates it with the same stable endpoint ID and
  graph binding but the new name.
- During this sequence the UI reports pending/actual/native/Engine-confirmed
  state. Held-client, create-failure and repeated-rename cases are tested;
  desired name is retained if native recreation fails.

### Buffer and latency interpretation

- Process Loopback buffer policy is configurable through persisted app
  settings. Capacity, target, jitter, maximum packet and maximum gap are
  validated before activation and are distinct from physical latency.
- VAD has the same explicit target/capacity model, but low-latency admission
  requires a real packet/gap bound. The driver capability `4800` is a maximum
  packet bound, not proof that every packet is 4800 frames or that the endpoint
  has a 50 ms normal residence.
- Software changes removed the old Process Loopback 2048-frame prime and the
  duplicate professional prefetch. This explains a significant part of the
  observed delay risk, but it cannot establish the remaining Windows callback,
  render queue or physical acoustic/electrical delay.
- It is therefore not yet honest to declare VAD or Process Loopback faster.
  The current software structure gives Process Loopback the clearer short-path
  baseline; VAD may match or beat it only after native packet delivery and
  physical output are measured under identical conditions.

## Remaining execution gates

1. Diagnose and fix the target native `0x8007051A` revision mismatch. Repeat
   event and timer playback/microphone matrices until every case passes with
   zero drops, unexpected silence, stale packets, discontinuities, and epoch
   resets; retain the failed artifacts.
2. Run the current branch's production `live-vad` path on the target: create a
   named playback export, resolve its MMDevice, route it through
   `VirtualPlayback -> Engine -> Bus -> physical Output`, and verify orderly
   gate/owner/endpoint retirement.
3. Run held-client rename smoke through the app. Verify remove/recreate keeps
   stable ID and graph binding, reports pending/actual names correctly, and
   reopens admission only after Engine confirmation; repeat remove/create
   failure cases.
4. Add the same packet-size/gap telemetry to Process Loopback and VAD runs;
   sweep candidate targets such as 2048/1024/512/256 Process frames and
   480/240/128/64 VAD packet frames only when the native path admits them.
5. Measure `Input -> Bus -> Output` with a calibrated physical cable or a
   calibrated external dual-channel reference. Record p50/p95/p99/max,
   waveform continuity, drops, underruns, discontinuities and epoch resets.
6. Tune one complete configuration at a time. Reduce target and render queue
   until the first quality failure, then choose the smallest stable target with
   margin. Never hide an underrun by silently increasing the target after the
   run.
7. Run the 30-minute controlled-load experiment for each viable VAD and
   Process configuration. A candidate passes the experimental gate only when
   calibrated p95 plus uncertainty is at most 10 ms and all quality counters
   are zero.
8. Repeat the proposed default in two independent two-hour runs, then update
   the default profile and evidence only if both runs pass. Until then, keep
   the default Compatible behavior and label expert settings experimental.

The `<10 ms` target is not passed by this branch yet. Internal markers,
software queue timestamps, VM lifecycle results and silent-counter tests are
necessary diagnostics but are not physical end-to-end evidence.
