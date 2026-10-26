# Minimal I/O latency implementation and qualification evidence

## Source and scope

- Date: 2026-10-26.
- Branch: `feat/minimal-io-latency`.
- Source base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
- Current reviewed probe-quality implementation: `a57abde`; latest target
  probe: `d1df8fa`. Newer service/UI slices are tracked separately below.
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
  production VAD integration path are implemented. A dual-machine production
  `live-vad` run now reaches `VAD MMDevice -> native PCM -> VirtualPlayback ->
  Engine -> Bus -> physical render` with nonzero output and zero runtime epoch
  resets/recovery events. The complete event/timer matrix is still not passing;
  boundary xrun cleanup, rename smoke, Process comparison, and calibrated
  physical measurement remain pending.
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
  `0.36.20.788`. The initial matrix did not capture package hashes. The
  2026-10-27 follow-up matched `oem30.inf` to the service's exact DriverStore
  package and recorded all three installed file hashes below.
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
- The current branch's production `live-vad` path was run on the target with
  source commit `df3f76c`, binary SHA-256 recorded in the local archive, and a
  unique stable endpoint. The 10-second run completed 998 VAD packets with
  zero driver, carrier, stale, silence, epoch-reset, recovery, and delivery
  bound faults; physical render observed 5,172 nonzero samples with no render
  failure. The virtual playback boundary still reported 7 xrun events, so the
  run is a stability diagnostic, not a qualification pass.
- The retained report and manifest are archived at
  `target/vad-target-archive/20261027-live-vad-df3f76c/live-vad-df3f76c-final-10s/`.
  After the 2026-10-27 storage cleanup, the guest retains only the current
  release probe, `e2e_latency_probe-df3f76c.exe`.

### Target VM storage policy and cleanup

The target has a 64 GB system disk. Build on the development host and copy only
the executable and driver package required for the next target operation. Keep
the installed working driver; remove superseded test binaries and staging
copies once they are no longer needed. Do not accumulate build trees, duplicate
packages, completed run directories, PCM captures, or traces in the guest.

After each run, including failed runs, copy its evidence to the development
host under `target/vad-target-archive/`. Verify the copied file lengths and
SHA-256 hashes against the guest before deleting the guest copies. Retain
failed evidence on the host. Validate every deletion path inside the explicit
test directory, reject reparse points, and verify all candidates before the
first deletion. Keep only the current probe, active-run files, and any package
needed for the next install or rollback in `C:\MoirenVadTest`.

The 2026-10-27 cleanup verified ten report/manifest files from five completed
runs, totaling 80,876 bytes, before deleting those guest directories. The
verified copies, remote hash manifest, and cleanup inventory are under
`target/vad-target-archive/20261027-vm-retention/`. One earlier diagnostic
directory, `live-vad-044b819-10s`, was deleted before archive verification.
Its runtime/cleanup/counter summary was recovered from the local session log
as `recovered-live-vad-044b819-10s-summary.json`; its original report and manifest
were not recovered. This partial record is not a replacement for original
qualification artifacts.

Final target inventory: one 3,267,072-byte executable (SHA-256
`795040D89670389C4CDCA36A0F29D8E94848B412C2A946DF8D347982C34087D1`),
30.11 GiB free, no active Moiren/VAD probe process, and no present dynamic
`MOIREN\VAD_RAW` child. Historical non-present PnP entries were not counted as
active endpoints. This is storage/lifecycle verification, not audio
qualification.

### Production `live-vad` startup diagnostics

The current release probe was pushed and exercised on the same target in three
small steps. These runs are startup diagnostics only and do not qualify PCM or
physical latency:

- `2b4a3b5` first retained the final `HostSnapshot` and backend native report.
  The target showed `Blocked` with `native outputs require exactly one explicit
  physical master clock`. The VAD project builder had left its physical output
  as the default follower.
- `6182064` set the VAD physical output to `ClockRole::Master` and added a
  regression test. The target then passed that preparation boundary and Engine
  reached its startup condition; the next failure was native source startup:
  `expected one VAD playback MMDevice ... found 0`.
- `fb71888` makes native playback wait and retry MMDevice publication for the
  same bounded 15-second startup window used by the native VAD probes, while
  preserving the last enumeration error. It was pushed but could not be
  rerun because SSH to `192.168.111.128` began resetting during key exchange.

- `0cb4d2f` prepared the native WASAPI client before opening the VAD gate and
  started it only after Engine admission. The first production run then
  delivered 203 submissions and 199 completions, but its report mixed the
  cleanup epoch into the runtime snapshot.
- `8d4be23` separated runtime and cleanup observations, kept the native source
  alive for the control-side measurement interval, and surfaced virtual-boundary
  xrun events in qualification quality. The target still showed one startup
  recovery and 7 xrun events in a 2-second run.
- `044b819`, `8bae8ce`, and `df3f76c` distinguished initial gate/stream
  admission from recovery only until the first PCM owner exists. The final
  10-second target run has `epoch_resets=0` and `recovery_events=0`; this is a
  lifecycle correctness fix, not permission to ignore later stream/gate changes.

SSH recovered and the target cleanup was verified on 2026-10-27, as recorded
above. The `df3f76c` production rerun remains unqualified because of its seven
boundary xruns and missing calibrated physical evidence.

### Startup xrun investigation on 2026-10-27

Probe commit `d1df8fa` adds up to 256 startup samples, collected at 10 ms
control-side intervals. Storage is bounded independently of run duration.
The existing native render trace also retains only its first 256 callbacks.
Neither trace resets, subtracts, or replaces the full-run quality counters.
The target executable SHA-256 was
`841E741DD9646447D012065C9068EA6D334F85F658A037784820D7FBBB5AE0F8`.

Two ten-second production runs used Compatible input and full-capacity render
settings on the same target output:

| Run | Provider completions | Boundary xruns | Provider drops / stale / silence / resets / recovery |
| --- | --- | --- | --- |
| `live-vad-d1df8fa-startup-10s` | 998 | 11 | All zero |
| `live-vad-d1df8fa-trace-10s` | 998 | 7 | All zero |

Both are failed quality diagnostics, not qualification passes. The code path
that waits for initial carrier prime returns before updating this probe's xrun
counter. The nonzero probe xruns therefore represent actual carrier shortage;
they are not the earlier initial-admission recovery-counting bug.

In the trace-mode run, the first observed xrun appeared by 0.405 seconds and
the seventh by 1.125 seconds. The count remained seven in the final runtime
snapshot. Each traced native render wake demanded 480 frames, but the first
20 wake intervals elapsed in only 0.1750872 seconds. Across 240 intervals,
the device demanded 115,200 frames in 2.2306278 seconds; a nominal 48 kHz clock
would advance 107,070.1344 frames in that interval. This comparison excludes
the first callback's demand, so it does not count the starting buffer as
clock drift. The excess was about 8,130 frames, far outside a 2,000 ppm drift
correction budget.

The pinned output is the guest's `High Definition Audio Device`, hardware
identity `HDAUDIO\FUNC_01&VEN_15AD&DEV_1975`. The evidence points to a transient
rate mismatch between this VMware output and the VAD packet clock during
startup. It does not prove behavior on a physical DAC. Keep these quality
failures visible; do not enlarge buffers silently or reinterpret internal
timestamps as calibrated physical latency.

Reports, manifests, binary hashes, startup samples, and cleanup receipts were
copied to the host and verified before guest deletion:

- `target/vad-target-archive/20261027-live-vad-d1df8fa/`
- `target/vad-target-archive/20261027-live-vad-d1df8fa-trace/`

The second archive also contains `render-pacing-analysis.json`,
`hardware-and-driver-inventory.json`, and `installed-package-hashes.json`.
The installed `oem30.inf` hash matched
`moirenvad.inf_amd64_83f7fb4733f00562`, which is also the running service's
image directory:

| Installed file | SHA-256 |
| --- | --- |
| `moirenvad.inf` | `B3F2C04552C76DF5689D5131FAA17A2D33078A29001EB90B2A1A5344E88CA58D` |
| `MoirenVad.sys` | `0E8AB77CD06D81E14EF2227E817CD3146327B9436FF36B7819B2449E4F1CA04B` |
| `MoirenVad.cat` | `917F32ADC36CB9B7D398B8581020B2A15A6804623F90759F39E8381B7E685EAE` |

After both runs, the guest retained only `e2e_latency_probe-d1df8fa.exe`
(3,269,120 bytes), had 30.10 GiB free, and had no active test process or present
dynamic VAD child. No driver package, device default, or buffer policy was
changed by this investigation.

### Product observation presentation

Commit `e19b834` retains native virtual-boundary prepared capacity and optional
content target in the service's existing native snapshot. Diagnostics now shows
those values alongside post-transfer occupancy, signed correction ppm, and
cumulative boundary xrun/soft-event counts, with explicit frame units and
scope. Stream association uses both node and kind; stopped/stale snapshots
are hidden, and an absent target remains unknown.

Focused RED/GREEN regressions cover the real native snapshot and UI text
projection. Final affected library tests passed: 450 Windows-audio tests and
291 app tests, with four existing opt-in checks ignored. Strict library/test
Clippy and touched-file formatting passed. This closes one presentation
omission. Subsequent provider and native observation work is recorded below;
the full Task 7 hardware and presentation acceptance gate is still open.

#### Provider observation and QUERY provenance

Commits `9b97c08` and `ee73c8e` preserve provider telemetry through the service
and show it in Settings -> Diagnostics. Rows distinguish stable identity,
requested/actual names, pending native recreation, native preparation, exact
endpoint errors, packet/completion timing, and cumulative drop, stale, silence,
recovery and reset counters. Full `u64` values remain integers; missing or
unsampled timing is unknown. These rows describe the latest native export
observation, not Engine-confirmed live audio or measured physical latency.

Independent review found that a failed native QUERY could return retained
measurements inside a successful batch. Fix `fe88ad3` adds explicit provider
provenance and clears cached measurement fields before assigning the accepted
service observation generation. Endpoint errors, fresh preparation-refusal
evidence, healthy endpoints and existing gate/Engine authority stay separate.
The fix and UI slices passed their independent reviews. Focused provider tests
passed 66/66 and exports tests 32/32; the UI's committed-baseline checks passed
306 app-library tests with two existing ignored and eight acceptance tests.
Combined-tree verification remains separate from these pinned slice results.

#### Live native observations

Commit `e431f38` publishes bounded coherent native delivery and qualified
oldest-pending-frame age through the capture bridge. Native QPC observation is
prepared even with trace disabled. Unknown, invalid/future timestamp, empty,
reset and underrun paths cannot retain a stale successful age. This is an
explicit timestamped pre-transfer sample, not a continuously advancing age or
an estimate from queue fill. Delivery and age are independently coherent,
not a transaction with each other or with every scalar counter.

Commit `392fab3` adds the fixed-size render padding observer; `ce136a2` wires
it to real WASAPI padding queries and backend output-node observations.
Samples are unknown before the first wake and after owner stop/error/unwind.
Publication does not allocate or wait; control retains the storage through
join, and replacement-owner cleanup cannot clear a newer generation. Zero
padding is a valid raw observation, not proof of a hardware xrun or missing
audio. The scalar primitive passed independent review. Bridge and native
integration review and their final UI projection are still in progress.

After native integration, the affected Windows-audio library passed 465 tests
with two existing ignored, the routing probe passed 14 tests, and strict
all-target crate Clippy passed. No new device or physical measurement was run
for these software observation changes.

### Process quality reporting correction

Commit `a57abde` fixes a reporting error in the Process harness: it previously
sampled the capture bridge before activation and serialized that startup
snapshot after the run. The observer now survives both native-owner joins and
is sampled only after both have completed.

The common quality producer now retains Process overflow loss, running
backlog/age discards, underrun frames, bridge discontinuities/resets, and
graph-to-render bridge shortfalls. It selects one canonical render report
when standalone/backend reports overlap, preserves independent VAD faults,
and saturates cumulative sums. Source SILENT flags, initial prime trim, and
empty-padding wakes remain separate diagnostics; none alone proves missing
transport audio.

Four new regressions demonstrated the omitted counters and integer overflow
before the fix. All 22 example tests and strict example Clippy passed.
Independent task review passed with no findings. The corrected Process
producer has not yet been rerun on the native target; these are software
regression and call-order results, not a new runtime or physical measurement.

### Continuous collection limits

Ordinary live CLI and finite native session duration accept at most 600
seconds. Ordinary Process capture allocates its PCM evidence for the entire
requested duration before streaming. Extending that allocation unchanged to
two hours would require approximately 2.76 GB for stereo Float32 PCM alone,
which is unsuitable for the target VM.

Commit `4be32c4` adds an explicit diagnostic-only soak mode up to 7,200 seconds
without whole-run Process PCM storage. It uses bounded periodic JSONL samples
(5-60 second intervals, at most 1,442 records of 32 KiB each), preserving raw
counters while keeping the same source and Engine owners running. This is
not bounded continuous waveform capture or physical qualification. Its first
independent review identified cancellation dependence on synchronous journal
I/O, incomplete best-effort VAD cleanup after collection failures, and reserved
artifact-path collisions. Those findings are being fixed; do not use the
initial implementation as evidence of a qualified two-hour run.

The VM storage policy was rechecked on 2026-10-27: `C:\MoirenVadTest` still
contained only the current 3,269,120-byte probe, no running probe was found,
and 30.10 GiB was free. No additional obsolete test files needed deletion.

Before collecting the required continuous windows, the live harness needs
bounded capture/window evidence, supervised controlled-load coverage, an
observed steady window, orderly stop/join, and artifact provenance. The
existing evaluator already requires 30 continuous minutes for an experimental
candidate or two independent two-hour runs for default promotion; changing
those thresholds, joining short runs, or setting report booleans does not
complete collection. The separate application integration soak has bounded
long-run windows, but its `VirtualPlayback -> Gain -> VirtualMicrophone` graph
does not substitute for the physical `Input -> Bus -> Output` reference.

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
  production `live-vad` path has also activated the complete Moiren Engine
  graph, as recorded above. A clean native matrix, live rename smoke and
  calibrated VAD-vs-Process comparison remain open.

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
