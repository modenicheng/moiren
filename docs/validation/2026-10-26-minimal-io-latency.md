# Minimal I/O latency implementation and qualification evidence

## Source and scope

- Date: 2026-10-26.
- Branch: `feat/minimal-io-latency`.
- Source base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
- Latest collector/control diagnostics: `d64250f`, with runner repair
  `59e781b`. VAD remains unqualified; an output-only control reproduces native
  demand above nominal 48 kHz. See the final 2026-10-27 section for the current
  target inventory and raw archive locations.
- Latest native target period diagnostic: `405f6ec` adds an opt-in request for
  the queried exclusive minimum. The matching test-signed package remains
  installed as `oem32.inf`;
  bounded event/timer retests still fail the zero-quality gate. Earlier
  implementation, review and service/UI checkpoints are tracked below.
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
| 4 | Software budget/boundary passed; native quality remains unqualified | `3289ded` fixes timer Playback first-buffer erasure in the one-second matrix; event Microphone drops and a five-second timer epoch mismatch still fail quality |
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

After that cleanup, target inventory was one 3,267,072-byte executable (SHA-256
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
omission. Subsequent provider and native observation work is recorded below.
Observation publication and text projection are implemented and independently
reviewed; full rendered UI acceptance and hardware qualification remain open.

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
audio. The scalar primitive, bridge and native integration passed their
independent reviews. Commit `7e235a9` adds the final live-native UI text
projection, also independently reviewed. Eight focused row regressions and
315 app-library tests passed with two existing ignored; strict app Clippy
passed. These slice results are not the full rendered UI acceptance matrix.

After native integration, the affected Windows-audio library passed 465 tests
with two existing ignored, the routing probe passed 14 tests, and strict
all-target crate Clippy passed. No new device or physical measurement was run
for these software observation changes.

#### Render diagnostics owner-layout repair

Expanded live bridge DTOs exposed an existing stack-layout guard in the
combined workspace test: the inline eight-entry diagnostic history made
monitor cleanup handoffs too large. Commit `3a95588` moves that fixed history
to preparation-time storage and retains it until control-side reclamation,
including after native callback unwind. The existing monitor bounds are
unchanged; no RT allocation/deallocation or event retention guard is relaxed.
Four diagnostics tests, 35 native-render tests and seven monitor tests passed,
along with strict library/tests Clippy. Independent repair review passed.
Full combined-tree verification remains separate from these focused results.

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
artifact-path collisions. Repair `2c61452` passed independent re-review with
all three findings resolved. One non-RT journal worker and one pending-record
slot keep disk I/O out of the finite native cancellation loop; all VAD cleanup
stages are attempted and their errors retained. Reserved output aliases are
rejected before audio starts, and Process samples retain real render padding
or explicit unknown. Thirty-four example regressions and strict example/tests
Clippy passed. The JSONL maximum is 47,252,898 bytes, not whole-run PCM.

The owned writer is joined only after native shutdown. A stalled OS write can
still delay final process exit, but not the native cancellation request; no
worker is detached or forcibly terminated. This is not crash-atomic journaling
or a qualified two-hour run. Continuous physical marker windows and controlled
load remain missing.

Before the new diagnostic run on 2026-10-27, `C:\MoirenVadTest` contained only
the 3,269,120-byte `d1df8fa` probe, no running probe was found, and 30.10 GiB
was free. It was replaced and cleaned as recorded below.

### Reviewed diagnostic-soak target smoke

The host built release probe `2c61452`, length 3,359,744 bytes, SHA-256
`2F25C5BAADE7C0D2350A41C9FB00F7CA26D5ACB1C84258D3EC8BB436BAF2A1D0`.
The guest verified its length/hash before deleting the superseded `d1df8fa`
binary. No build tree, driver install, device default or buffer policy was
changed on the guest.

The 10-second `live-vad --diagnostic-soak` run observed 10.0041523 seconds and
wrote two periodic records at approximately 0 and 5.0102 seconds. Both contain
real render-padding observations: 576 frames, capacity 1,056, period 480,
sample rate 48 kHz. The final running snapshot contains the same raw padding;
these values are not physical E2E measurements.

The harness returned its expected unqualified exit code 1, with no native
collection/cleanup error. Render produced 491,136 frames and 5,172 nonzero
samples; render failure, graph-bridge shortfall and empty-padding wakes were
zero. Provider completed 990 packets, maximum completion gap 53.0038 ms;
drop/stale/silence counters were zero, but runtime recovery and epoch reset
were both **2**, and the virtual playback boundary had **4 xruns**. This run
therefore fails stable-quality qualification. These nonzero counters must not
be replaced by the older runs' zero-provider-counter result. No calibrated
physical evidence, waveform continuity or sustained-load window was collected.

The requested export name was retained, but the native MMDevice observed a
generic Windows virtual-endpoint name rather than that requested name. This
smoke does not establish native friendly-name propagation or live rename.
The host confirmed Closed and the driver reported Removed after retirement.

All three report/manifest/JSONL files (589,290 bytes total) were copied to
`target/vad-target-archive/20261027-live-vad-2c61452-soak/`; each length and
SHA-256 matched the guest manifest before deletion. Remote evidence was
rechecked against that manifest, then the run directory and helpers were
deleted. The cleanup receipt confirms only the current 3,359,744-byte probe
remains, no active test process or present dynamic VAD child, and 30.10 GiB
free. Failed quality evidence remains archived on the host.

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

### Final combined Windows host verification

The combined production-source tree at `2c61452` passed workspace build,
all-target check, and workspace tests: 1,602 passed, zero failed, eight ignored
across 70 test binaries/doc-test suites. Native hardware opt-in tests stayed
ignored. The full test run includes the previously failing, unchanged monitor
cleanup-size guard and the lengthy clock fault/startup simulations.

Whole-workspace rustfmt found one capture-regression assertion wrap; the
format-only `41e8d33` repair passed seven focused capture tests and the complete
format check. Strict workspace all-target Clippy also passed.

Full `cargo test --locked -p moiren-app --features gpui-test` initially failed
the running-settings restart-confirmation fixture. Exact reproduction with a
backtrace showed it waited after confirmation because its fake driver never
published `active_transport_latency`, now required by the real transaction.
The independently reviewed test-only repair `9b91efb` publishes transport
state at initial activation and explicit confirmation, matching the
production owner; no timeout, production
confirmation rule or assertion was relaxed. The exact test then passed and
the complete gpui-feature run passed 484 tests, zero failed, four ignored
across 20 suites (including eight UI acceptance tests). Feature-enabled strict
app all-target Clippy and the final format/diff checks passed.

Logs are retained in `.superpowers/sdd/task7-final-*.log`, with the initial
failure, exact RED/GREEN and fixture-review evidence kept separately. These
are local Windows software results, not Linux CI, Miri, a full rendered UI
state matrix, continuous device waveform, or physical qualification.

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

### 2026-10-27 compact native UI and target retention update

Commit `b6fbbd2` adds three 1024x720 real GPUI rendered acceptance tests for
native packet/gap/qualified pre-transfer age/padding provenance, absent/stopped
or stale generations, fresh replacement and vertical scroll reachability.
The focused three tests, all eleven `ui_acceptance` tests, touched-file format
check and feature-enabled all-target app strict Clippy pass. Independent pinned
review `78c85a3..b6fbbd2` passes spec compliance and code quality. Tests inject
snapshots at the UI boundary: they do not prove native collection, pixels,
horizontal clipping, OS screen readers or physical qualification. Verified
physical-result import/qualified states and full service rename matrix remain
outstanding. No accepted physical result was manufactured.

Guest inventory rechecked over SSH on 2026-10-27: only the current
`e2e_latency_probe-2c61452.exe` (3,359,744 bytes) remains in
`C:\MoirenVadTest`; no test processes or present dynamic raw VAD children;
30.12 GiB free. There is nothing obsolete to delete. Build on host; future
guest evidence is still archived with per-file length/SHA-256 verification
before deletion, preserving system and DriverStore files.

### 2026-10-27 bounded capture precursor

Commit `e16ed25` adds a prepared mono/stereo Float32 SPSC evidence tap capped
at 8 MiB of frame storage including metadata. The native lease copies before
ReleaseBuffer, retaining sample bits, packet sequence/offset/count, raw flags,
device position and QPC; capacity refusal drops a whole packet visibly.
Position-gap inference requires explicit compatible units. Deferred physical
attachment uses negotiated format and retains the writer in the report across
native error/unwind/cancellation through control join. Capture report schema is
9. Existing defaults/options/legacy artifacts remain unchanged.

Focused capture library 29 tests and evidence integration eight tests pass;
full Windows-audio crate 694 passed/5 ignored across 29 result suites, strict
all-target crate Clippy and touched format pass. Independent pinned review
`3140c04..e16ed25` passes spec and quality. Synthetic owner and fake-COM tests
establish report retention and leased copying, not hardware panic safety or
allocation-free OS unwinding. Packet copies and normal close with an abandoned
reader have zero observed allocations/deallocations; storage is reclaimed on
control. Direct Process deferred attachment, bounded disk sink, genuine
measurement/load collection and physical qualification remain subsequent work.

### 2026-10-27 bounded native measurement collection

Commit `ab48a9a` adds `e2e_latency_probe record-measurement` and `--list` for
an independent negotiated WASAPI capture. It uses the deferred tap plus an
owned non-RT sink, fixed 64 KiB PCM/16 KiB metadata buffers and one bounded
packet JSON record, not duration-sized memory. Duration (1..7200 seconds),
ring capacity, PCM bytes (at most 3 GiB), metadata bytes (at most 256 MiB) and
packet count (at most 1,000,000) are explicit. New directories/files prevent
overwriting runs; loss, partial IO and failed artifacts remain visible.
Native stop/join precedes sink join even on error/unwind. Sink join may still
wait for blocked OS IO after native shutdown; flush does not guarantee fsync
or survival across process/power loss.

Independent pinned review `2805dff..ab48a9a` passes spec and quality; 52 example
tests (including 15 sink tests), strict example/test Clippy and format pass.
Host-only CABLE two-second smoke negotiated 48 kHz/stereo from a 96 kHz mix
and correctly disabled incompatible position-gap inference. It retains 199
packets/95,489 frames, 763,912 PCM bytes and 26,014 metadata bytes, with zero
tap drops/overflow/abandonment or sink missing/incomplete/nonfinite samples.
One native discontinuity remains a failure; exit 1 and `qualified=false` are
intentional. Packet offsets, file lengths and SHA-256 were verified on host.
Invalid endpoint startup retained its `GetDevice` `0x80070057` report.
A separate eight-byte-cap failure stopped native capture after 0.0258694
seconds, retained the failure report and zero-byte PCM/metadata files, and
never exceeded its cap. No microphone was captured, no VM files were staged,
and no defaults or driver installation changed.

This standalone collector does not launch/supervise a source, graph or load,
infer a physical boundary/calibration, automatically authenticate artifacts,
or qualify physical E2E. Same-condition physical setup/marker capture,
controlled load, 30-minute and two independent two-hour gates remain open.

### 2026-10-27 rendered lifecycle and current target evidence

Commit `4b0deba` adds two compact rendered UI acceptance tests covering held
old name during rename, refused recreation, and fresh replacement for Playback
and Microphone. Independent pinned review passes spec compliance and quality.
All thirteen rendered acceptance tests pass; the full feature-enabled app suite
passes 489 tests/4 ignored across 20 result suites. Workspace tests pass
1,621/8 ignored across 71 result suites. These are software checks, not native
rename acceptance, calibrated latency results or Stage 7 completion.

A fresh host-built native probe (source `f6f2718`, 380,928 bytes, SHA-256
`68396c12a6f7ece963b7292ce3e18e29b4e4b09fb8869948d22c968f5bd9d2a2`)
completed all eight timer case reports with unchanged defaults and zero reported
native discontinuities/microphone drops. Observed silence remains, so this is
not zero-silence or physical qualification. Event mode failed in Microphone
Mono Pcm16 at asynchronous `GetOverlappedResult(VAD PCM)`, `0x8007051A`, after
36,960 signal frames, 9,600 silent frames and two dropped microphone writes.
The failure is no longer ambiguously labeled control IO; exact epoch/shape
cause remains to be diagnosed. The wrapper's process exit fields are null,
not verified success/failure exit codes; stdout/stderr are preserved as-is.

Archive `target/vad-target-archive/20261027-native-f6f2718-matrix` retains six
payloads and a manifest (12,577 bytes including that manifest). Host payload
length/SHA-256 checks and a second guest check preceded deletion. Temporary
run, native probe and runner were removed immediately; only the current
3,359,744-byte diagnostic probe remains on the VM, with no dynamic endpoints
or probe processes and 30.12 GiB free. Build artifacts remain host-side; no
DriverStore/system/default-device changes were made.

### 2026-10-27 pre-cleanup failure discriminator and repeat limits

Commit `f098fa9` adds native-probe-only admitted/opened and PCM-shape snapshots,
plus one best-effort QUERY on signal-phase failure before gate revocation can
change its epoch. Explicit scalar identity/epoch/state/shape fields retain the
decoded current snapshot even if progress validation fails. Diagnostic QUERY
failure remains separate from the original PCM error, and cleanup still runs.
No driver, protocol, timing, retry or admission policy changes. Seven added
tests show genuine RED/GREEN; all 33 native example tests, strict example/test
Clippy and host release build pass. Independent pinned spec/quality review passes.

The new 391,168-byte binary SHA-256 is
`168a9c77645267076b365c4494dd6483a38cb39ed3bea8cbd1ee832a3953effd`.
One-second event and timer matrices each returned recorded exit 0 and all eight
case reports, with zero reported native discontinuities/microphone drops.
Microphone event cases still contain 480 silent frames each. The event repeat
at five seconds per case also exited 0, but Float32 microphone writes dropped
once in mono and three times in stereo; those cases contain 1,440 and 3,840
silent frames. The existing probe signal threshold is not a zero-drop quality
gate. `0x8007051A` did not reproduce, so the new failure observation has not
yet discriminated the cause. It is not honest to call the intermittent issue
fixed, the candidate stable, or physical latency qualified.

Artifacts are retained under host archives
`target/vad-target-archive/20261027-native-f098fa9-snapshot` (19,091 bytes) and
`target/vad-target-archive/20261027-native-f098fa9-event5` (8,500 bytes), counting
payloads and manifests. All nine payload lengths/SHA-256 values and remote
manifest bytes were checked before deleting the guest runs and temporary probe/
scripts. Only the 3,359,744-byte current diagnostic probe remains, zero dynamic
endpoints or probe processes, 30.12 GiB free. A read-only refresh confirms the
running installed INF/SYS/CAT hashes still match the recorded package.

A final bounded event repeat used 15 seconds per case and exited 0 with all
eight reports. PCM16 Microphone dropped three mono and one stereo write, with
3,360 and 1,440 silent frames; native discontinuities remain zero. No revision
mismatch/new failure snapshot occurred. The diagnostic exit criterion is still
not the stability gate. Its three payloads and manifest (8,508 bytes) are
archived in `target/vad-target-archive/20261027-native-f098fa9-event15`, verified
on host and rechecked on guest before immediate deletion of run/probe/script.
Guest again retains only the current diagnostic probe, no dynamic endpoints/
probe processes and 30.12 GiB free. No long qualification run is warranted.

### 2026-10-27 composable recorder lifecycle

`6dcb9e8` extracts example-local measurement preparation, activation, native
Start acknowledgement, nonblocking health/stop and native-before-sink finish.
Explicit partial reports and capped failure accumulation survive cleanup.
Review found a discarded session deadline in the adapter; `ce91b05` fixes it
using the earlier activation/session/adapter bound. Independent re-review
passes spec/quality and closes the original P2. All 69 example tests pass,
including 13 owner/four adapter regressions and 15 unchanged sink tests; strict
example/test Clippy, scoped formatting and host example build pass.

The corrected host CABLE two-second recorder returns native started/stopped,
198 packets/95,009 frames, 760,072 PCM and 25,882 metadata bytes. All three file
lengths/SHA-256 values and packet sequence/offset/PCM size consistency are
verified. Zero tap drops and sink missing/incomplete/nonfinite samples do not
erase its one native discontinuity: exit 1, failed/unqualified remain correct.
Earlier owner-state validation also exercised an eight-byte cap, retaining
zero-byte waveform/metadata and stopping native in 0.0201606 s. No microphone,
defaults or VM changes. Source/graph/load supervision, physical calibration,
stable candidates and qualification repetitions are still open.

### 2026-10-27 independently scheduled VAD source

Source plan `45268bd` has independent spec/quality review PASS, 85 example
tests PASS, strict example/test Clippy, scoped format and host release build
PASS. Source marker positions no longer come from preloaded measurement
analysis. Exact source JSON is retained; silent native preroll and acknowledged
post-Start source submissions have separate checked counters. These facts do
not establish played/captured marker arrival or map source to recorder clocks.

A bounded three-second target smoke confirms 96,000 source frames submitted,
480 silent native preroll frames, origin at native submission frame 480 and
144,960 total submitted native frames. Native error is null, event timeouts
zero and provider dropped/silent/stale/recovery/epoch counters zero. The
graph boundary still records seven xruns: quality FAIL. Engine closes and the
export reaches Removed. Exit 1 and unqualified remain correct; no independent
measurement PCM, calibration or physical latency was collected.

Host archive
`target/vad-target-archive/20261027-source-45268bd-3s/source-45268bd-3s`
contains seven payloads and manifest, 518,194 bytes total. Every length/SHA-256
and exact admitted source bytes were checked on host and rechecked on guest
before deleting guest evidence, temporary 3,570,176-byte probe, plan and runner.
Probe SHA-256:
`32e6540f17f06c3f6ce11e826770b4db9fbea84cde24beccca674c8d244d3519`.
Only the current 3,359,744-byte diagnostic probe remains on guest, no test
processes/dynamic endpoints and 30.11 GiB free. No driver/system deletion.
Concrete source/graph/independent capture collection is the next implementation;
controlled load and physical qualification are still unfinished.

### 2026-10-27 Windows name evidence gap

The same source smoke records requested/provider name
`Moiren-source-marker-probe`, but Windows MMDevice FriendlyName remains a
localized speaker label containing `23- Moiren virtual audio endpoint`.
This was a CREATE smoke, not a held-client rename run. Driver policy already
attempts requested-name publication through PDO text, interface/endpoint policy
and the ACX pin callback; this mismatch alone does not identify a failed path
or prove Windows caching.

Read-only code tracing confirms a separate application reporting gap:
`WindowsVadProvider` observations return stored `ExportSpec.name`, the service
maps that into `actual_name`, and the UI labels it "Actual endpoint name".
There is no Windows property readback in that chain. Provider-created-name
equality can therefore clear rename_pending without proving the displayed
Windows name matches. Existing fake service and injected UI tests cannot close
this native acceptance gap.

Keep provider creation, lifecycle completion and observed Windows naming
distinct. The pending real-service held-client rename test must read the exact
MMDevice property before/after recreation, retain stable ID/binding and fresh
native generation/Engine admission, and preserve direct old-Removed evidence.
The existing app integration example does not exercise rename, and the active
native lifecycle example uses the same name and a separate control owner.
No new driver rename ABI, cache workaround, default change or passing native
name result is justified by this investigation.

### 2026-10-27 real VAD collection path

The reviewed implementation now has an opt-in collection mode that runs the
independent recorder concurrently with the existing VAD playback and production
`VirtualPlayback -> Bus -> physical Output` graph. It records exact source and
collection configs, negotiated recorder format, PCM/packet artifacts, source
submission progress, provider/Engine/native health, cleanup outcomes and raw
partial failures. It does not reuse measurement analysis as a source clock and
does not construct a nominal-format qualification result.

The collection lifecycle was independently reviewed. The review initially
found two finalization gaps: a source timeout after the last healthy poll could
be missed, and first raw failure evidence found during retirement could be
overwritten by a subsequent query. Both are fixed in `43c572f`; the repair
review passes. Software tests are green, but no native run of this combined
collector has occurred yet. VM microphone/output evidence remains diagnostic;
calibrated physical input wiring, marker discovery, controlled load and
physical p95 uncertainty are still absent.

The first combined VM smoke entered the collector and failed during independent
microphone recorder preparation: WASAPI shared-period negotiation returned
`0x80070057` (`choose_shared_period`). The failed report correctly retained
`collection_complete=false`, `cleanup_complete=false`, `qualified=false` and
the primary recorder/cleanup errors; it does not establish a VAD, graph or
driver-quality failure. Ten payloads totaling 17,993 bytes were archived and
length/hash-checked on the host, then rechecked on the guest before cleanup.
The guest again retains only the 3,359,744-byte current probe, with no test
processes/dynamic endpoints and 30.11 GiB free. This VM has no independent
calibrated reference endpoint, so no physical latency conclusion follows.

1. Diagnose and fix the target native `0x8007051A` revision mismatch. Repeat
   event and timer playback/microphone matrices until every case passes with
   zero drops, unexpected silence, stale packets, discontinuities, and epoch
   resets; retain the failed artifacts.
2. Repeat the latest corrected production `live-vad` path for a clean target
   stability window. Prior `df3f76c`/`d1df8fa` smokes reached
   `VirtualPlayback -> Engine -> Bus -> physical Output`, but boundary xruns
   failed the quality gate. Verify orderly gate/owner/endpoint retirement and
   preserve failed evidence.
3. Run held-client rename smoke through the app. Verify remove/recreate keeps
   stable ID and graph binding, reports pending/actual names correctly, and
   reopens admission only after Engine confirmation; repeat remove/create
   failure cases.
4. Use the implemented native delivery/provider observations and corrected
   Process quality producer for comparable continuous recordings and
   same-condition target sweeps. Sweep candidates such as 2048/1024/512/256
   Process frames and 480/240/128/64 VAD packet frames only when admitted.
   Positionless input age remains unknown; do not derive it from fill/rate
   or invent a hardware period.
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

### 2026-10-27 valid VM shared-period negotiation and quality failure

Commit `c3fbc8d` removes an incorrect local requirement that the Client3
default period itself be a multiple of the fundamental period. The target VM
reported `default=488`, `fundamental=35`, `min=139`, `max=488`; the 5 ms
request selected 245 frames and Client3 initialized successfully. This is a
valid capability set, so the earlier `0x80070057` at `choose_shared_period`
was a host-side validation defect, not a driver or endpoint rejection.

The host release probe from `c3fbc8d` was verified on the target as 3,748,352
bytes with SHA-256
`7490E742A9297982431AA3DBD28C24D50264D8C5E196CE6F33C4A3F6306B71B`. The same
15-second VAD collection inputs were used. The run exited 1 and remains
unqualified, but it progressed through independent recorder activation and
produced 1,327,509 bytes of evidence. The recorder reported requested period
245 and actual period 279 frames, with no period-selection failure.

The quality gate still fails: VAD completed 37 packets with 36 completion
gaps; the production virtual boundary recorded one xrun and 33 soft events;
the physical capture recorded 74 gap events, 1,835 gap frames and two
discontinuities. The source acknowledged only 18,720 of 48,000 planned
frames before the collection stopped. The run is archived at
`target/vad-target-archive/20261027-collection-c3fbc8d-15s/`; every payload
was checked against the guest manifest by length and SHA-256. The guest was
cleaned back to the current probe only, with 30.11 GiB free. No driver,
DriverStore or system files were removed.

### 2026-10-27 collection failure evidence after retention repair

The two atomic commits `4850ccf` and `bc90dbd` were pushed to
`feat/minimal-io-latency`. The first retains native shared-period capability
and preparation reports; the second propagates those reports through the
standalone measurement and VAD collection artifacts. Windows-audio validation
after the repair passed: 478 library tests plus 2 ignored, 109 probe tests,
strict all-target Clippy, all-target check, format and diff checks.

A host release probe from `bc90dbd0a7eef37d9c9b21b164ab810cf70365f6` was
copied to the target and verified as 3,748,352 bytes with SHA-256
`BB42D63E1AF1AEC7F5D8E668DD54B0BBE058EBF68EC83277E7FCA820361648E1` on both
host and guest. The same 48 kHz, 48,000-frame source schedule, endpoint and
collection limits were used for a bounded 15-second run. The probe exited 1;
the run was unqualified and did not produce source, graph or measurement
success.

The retained native capture report now includes the diagnostic that was
previously lost: 48 kHz stereo caps were `fundamental=35`, `min=139`,
`max=488`, `default=488`, with requested period `5.0 ms`; period selection
failed as `invalid_capabilities`, mapped to `E_INVALIDARG` at
`choose_shared_period`. The host archive is
`target/vad-target-archive/20261027-collection-bc90dbd-15s/`; five payloads
were checked against the guest manifest by length and SHA-256. This is a
shared-period negotiation failure, not evidence about VAD quality, physical
latency, or the driver output path.

After verification, the guest retained only the current probe binary, with no
run directory, input files, runner, old probe, active test process or dynamic
VAD endpoint; free space was 30.12 GiB. No driver INF/SYS/CAT or system files
were removed.

### 2026-10-27 supervised controlled-load collection

Commit `3c94765` adds a real supervised CPU-load worker to the VAD collection
path. The worker has a unique load ID, PID, checksum, stop handshake, bounded
result schema and cleanup evidence. Collection starts it only after the
independent recorder is `Running`, polls for early exit, requires coverage of
the source plus postroll window, and fails closed when the worker result is
missing, invalid or too short. The implementation was pushed before the VM
run; 113 example tests, strict example Clippy, format and diff checks passed.

The release probe from `3c94765` was copied to the VM as
`e2e_latency_probe-3c94765.exe`, verified at 3,841,024 bytes with SHA-256
`F1A72D6EB3BD658D8054498904754F06241950CCB6682DF36F615BC5C5AAF29E` on both
host and guest. A bounded 15-second VAD collection used load ID
`load-3c94765-15s`. The run exited 1 and remains unqualified. The worker did
receive the stop request and exited zero, but the collection ended after
0.5462443 seconds while 1.2 seconds were required, so
`controlled_load.valid=false` and `window_complete=false`.

The run remains unqualified: VAD completed 42 packets with 41 monotonic
completion-interval timing observations (not packet drops); the virtual
boundary recorded one xrun and 33 soft events, and the independent capture
recorded 87 device-position mismatches totaling 2,128 absolute frame
differences plus two discontinuities. The complete seven-file run archive, including manifest,
report, PCM and packet metadata, is retained at
`target/vad-target-archive/20261027-controlled-load-3c94765-15s/`; host hashes
are in `host-sha256.txt` and the condensed result is in `summary.json`. The VM
was cleaned to only the current probe binary, with 30.12 GiB free. No driver,
DriverStore or system files were removed. This is controlled-load failure
evidence, not latency qualification.

### 2026-10-27 `a085841` target retest and production lifecycle smoke

The `a085841` release probe was built on the development host and verified on
the target at 3,843,584 bytes with SHA-256
`C5CC1A01A98952BD3F75CF2AE8BC6E2F144B9336E9703D7B0E0933A739739B3F`.
The matching `vad_native` probe was 391,168 bytes with SHA-256
`413004345D0B004AE6BB3A10702F2EA197479FE5D8AE5CB4A49CE388505FD8FA`.
This operation deployed the probe binaries only; the installed `MoirenVad.sys`
package was intentionally not rebuilt or replaced. The observations below
therefore do not yet validate the `a085841` `Pcm.cpp` change on the guest
driver. A driver-package build, install, package-hash record and repeat matrix
are still required.
After the run, the target retained only these two current probes in
`C:\MoirenVadTest`; old probes and logs were removed only after evidence was
copied to the host and checked. `C:\DriverTest`, the installed driver package,
and system files were not touched.

The one-second native event matrix completed with exit code 0. All four
playback cases reported 47,040 signal frames, zero silent frames and zero
native discontinuities. Microphone cases still contained 480 silent frames;
microphone stereo Float32 also reported one dropped packet and 1,440 silent
frames. The one-second timer matrix completed with exit code 0 and no dropped
packets or native discontinuities, but it still reported initial silent frames:
2,048 for playback Pcm16, 1,024 for playback Float32/stereo Pcm16, 512 for
playback stereo Float32, 768 for the microphone mono cases and 370 for
microphone stereo Float32. These matrices are retained as failed quality
diagnostics, not as a zero-silence driver pass:

- `target/vad-target-archive/20261027-native-a085841-event1/`
- `target/vad-target-archive/20261027-native-a085841-timer1/`

A three-second production `live-vad` smoke then exercised the target path
`VAD MMDevice -> native PCM -> VirtualPlayback -> Engine -> Bus -> physical
Output`. It exited 0, observed 3.011021 seconds, and retired the endpoint
cleanly to `Removed`. The requested name was `Moiren-VAD-a085841`; Windows
MMDevice readback was `扬声器 (21- Moiren virtual audio endpoint)`, so the
name propagation gap remains visible and is not treated as a rename pass.
The report retained stable ID
`00000000000000000000000000000012` and the physical output endpoint
`{0.0.0.00000000}.{71789a68-9934-4137-9E1A-BA0A5730AA68}`.

This production smoke is still unqualified. The VAD provider submitted 304
packets and completed 300, yielding 299 monotonic completion-interval timing
observations (not packet drops); the virtual boundary reported 7 xruns and 135
soft events. Provider driver/carrier/stale/silence/epoch/recovery counters
were zero, but boundary xruns remain a quality failure. The run used diagnostic pulses without a
source schedule, retained no marker-bearing measurement PCM, and used the
internal boundary without calibration, so it provides no physical latency
result:

- `target/vad-target-archive/20261027-live-vad-a085841-3s-r2/`

The empty artifact-directory invocation from the first attempt was a harness
error and is not audio evidence; it was cleaned separately. The target now
holds only the two current probe binaries and no active probe, run directory,
or dynamic VAD endpoint. Stage 7, native zero-quality qualification, live
rename acceptance, calibrated Process/VAD comparison, and the `<=10 ms`
physical gate remain open.

### 2026-10-27 `a085841` signed-driver package retest

The analyzed/test-signed package containing the `a085841` `Pcm.cpp` change was
built on the host and installed on the target before this retest. The target
reported `MoirenVad` as `Running` from
`C:\Windows\System32\DriverStore\FileRepository\moirenvad.inf_amd64_5253ce9aaaf75b9e\MoirenVad.sys`;
`pnputil` published the package as `oem26.inf`. The running target files
matched the host package:

| File | Length | SHA-256 |
| --- | ---: | --- |
| `MoirenVad.sys` | 109,840 | `C28A5BFE091FD5117EE728AB39DE338A27DB28C78558BCD7A252ABC087D84931` |
| `MoirenVad.inf` | 1,804 | `0F11E35038DF9D161CDE23FDE59480C7D670F0D737E76E2B7133439983E04D03` |
| `moirenvad.cat` | 8,484 | `1021FD83D4080320DF55C442973516C211CB3FF91013DFE22A5B14F65E288154` |

The new-driver one-second event matrix exited `0`. All four Playback cases
reported 47,040 signal frames, zero silent frames and zero native
discontinuities. All four Microphone cases reported zero dropped packets and
zero native discontinuities, but each still contained 480 silent frames. This
is an improvement over the earlier probe's Float32 microphone drop, but it is
not a zero-silence quality pass. The archived stdout, stderr and exit marker
were checked by length and SHA-256 on the host and rechecked on the guest:

- `target/vad-target-archive/20261027-native-a085841-event1-newdriver/`

The new-driver one-second timer matrix also exited `0`, with zero dropped
packets and zero native discontinuities in all eight cases. Initial silent
frames remained in every case: Playback Pcm16 mono `2,048`, Playback Float32
mono `1,024`, Playback Pcm16 stereo `1,024`, Playback Float32 stereo `512`,
Microphone Pcm16 mono `768`, Microphone Float32 mono `768`, Microphone Pcm16
stereo `768`, and Microphone Float32 stereo `287`. The timer artifacts were
verified the same way and retained at:

- `target/vad-target-archive/20261027-native-a085841-timer1-newdriver/`

After each result was archived and hash-checked, the corresponding guest
stdout/exit files were deleted. The VM now retains only the current
`e2e_latency_probe-a085841.exe`, `vad_native-a085841.exe`, and the required
`driver-a085841` package directory; no test process or dynamic VAD endpoint is
present, and the 64 GB guest has approximately 30.1 GiB free. No
`C:\DriverTest`, DriverStore rollback package, or system file was deleted.

This retest proves that the signed `a085841` package was actually loaded and
that the old revision-mismatch observation did not recur in these bounded
matrices. It does not qualify the driver: startup silence remains, and no
production `live-vad` rerun is warranted until the native quality gate is
improved. The application still supports user-visible rename through a
stable-ID remove/recreate protocol; the driver still has no in-place rename
IOCTL. The target remains integrated through
`VAD MMDevice -> native PCM -> VirtualPlayback -> Engine -> Bus -> physical
Output`, but the existing production smoke has xrun/completion failures and
does not establish a physical latency value.

The overall target remains: for `Input -> Bus -> Output` with no other nodes,
find the smallest stable configuration whose calibrated physical p95 is at
most 10 ms, with p50 exploration at most 8 ms, continuous output, and zero
drops, silent steady-state frames, underruns, discontinuities, or unexpected
recovery. The execution order remains: fix native quality, run a real held
client rename smoke with exact Windows MMDevice FriendlyName readback, build
matched VAD/Process Loopback evidence under the same output/engine/load
conditions, then sweep configurable buffer targets from larger to smaller and
retain the smallest stable candidate before the 30-minute and two-hour gates.
No VAD-versus-Process latency winner can be claimed before a calibrated common
physical boundary is available.

### 2026-10-27 gate-before-start probe retest

The native probe ordering repair is split into two pushed atomic commits:
`76cbf9b` admits the prepared VAD gate before native WASAPI `Start`, and
`617af1d` binds `PcmOwner` and cleanup diagnostics to the final `Running`
snapshot rather than the intermediate `Paused` snapshot returned by gate
admission. The latter was required after the first retest exposed a probe-side
stale-snapshot error; it was not driver evidence. The focused `vad_native`
suite passes 33/33, and format/diff checks pass.

The host release probe is 391,680 bytes with SHA-256
`D8B49717C9DB7C1139BAD6FC85D9E06F9B301411612A93F1B7DB21A49E03842A`. The
same hash was verified on the VM. Synchronous SSH execution avoids shell
wrapper exit-code ambiguity; both one-second matrices report exit `0`:

- Event mode: all four Playback cases report 47,040 signal frames, zero silent
  frames and zero native discontinuities. Microphone PCM16 mono reports one
  dropped packet and 480 silent frames; the other three Microphone cases report
  47,520 signal frames with zero silent frames and zero native
  discontinuities. This is an improvement for Playback startup, but not a
  zero-quality-failure pass.
- Timer mode: Playback retains 2,048/1,024/1,024/512 silent frames for mono
  PCM16, mono Float32, stereo PCM16 and stereo Float32. Microphone reports no
  silent frames or dropped packets, but stereo Float32 reports 45,600 signal
  frames rather than the expected one-second rate. This remains diagnostic,
  not a stable timer qualification.

The verified artifacts and manifest are retained at
`target/vad-target-archive/20261027-native-617af1d-event1/`,
`target/vad-target-archive/20261027-native-617af1d-timer1/` and
`target/vad-target-archive/20261027-native-617af1d-manifest.json`. The VM was
cleaned after verification and now retains only `vad_native-617af1d.exe`, the
existing `e2e_latency_probe-a085841.exe` and `driver-a085841`; no test process,
dynamic endpoint, `C:\DriverTest`, DriverStore rollback package or system file
was removed. Approximately 30.1 GiB remains free.

This probe result narrows the remaining startup issue: admitting the gate
before native Start removes the previous Playback startup-silence pattern in
event mode, while timer Playback and Microphone delivery still need work. No
production `live-vad` rerun or long qualification follows from this bounded
matrix until the native zero-quality gate passes.

### 2026-10-27 `02fd8b1` matching signed-driver retest and cleanup

The development host rebuilt source `02fd8b186bc27c14bbf7f761b6968eeed8d9fde5`
with `build.ps1 -Analyze`: no warnings/errors, valid INF and generated CAT.
All six driver host suites and all 33 `vad_native` example tests pass. The
release native probe was rebuilt and remains 392,704 bytes with SHA-256
`D3D64E87F73F450A0CDC2CDCD1CC4B6AFE4A40CDF2EB0113DD602005FED25C0B`.

The package contains `25951de`'s separate pre-Run queue admission predicate;
the transfer predicate still requires Running. Probe `02fd8b1` prequeues
Playback PCM before native Start. These changes were already atomically
committed and pushed before deployment.

The SYS was signed using the existing WDK test certificate, the catalog was
regenerated over that signed SYS, and the CAT was then signed. The development
host does not trust the self-signed root, so its trust verification failed;
the guest already trusts the exact signer
`7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D`. Guest SignTool verification of
both SYS and INF against the signed catalog passed before installation.
`pnputil /add-driver /install` published `oem31.inf`, version `10.13.1.204`.
The exact `ROOT\MEDIA\0000` device was restarted with no active probe or
present dynamic children. The resulting service is Running from
`C:\Windows\System32\DriverStore\FileRepository\moirenvad.inf_amd64_393dad2e93e4b62f\MoirenVad.sys`.
All three installed files match the host package:

| File | Length | SHA-256 |
| --- | ---: | --- |
| `MoirenVad.sys` | 103,832 | `C8D6644A7EEF6AB63EE1F713BB4BCEF6464921343AF1A467DFC39093167D705F` |
| `MoirenVad.inf` | 1,804 | `DC3529821E0C68B290D00F44809CF2733B97177A62C5891F1C05F26769895DD3` |
| `moirenvad.cat` | 2,474 | `632B125D249BFC9E3A17A56E5E0C1303B8ED280FE5DF65EAE8EC3C46B2B6773E` |

The first event run produced a complete eight-case report, but its wrapper
recorded a null exit code because it had not retained the process handle.
That run is archived as diagnostic evidence with an unknown exit code; it
is not promoted to a successful execution. Its Microphone cases contain
0-960 silent frames and three dropped packets in total. The wrapper was
repaired to acquire the owned process handle immediately after launch.
Subsequent timer and repeated event runs both recorded exit 0, no timeout,
and no present dynamic children after cleanup.

The following are the actual one-second-per-case results from
`native-02fd8b1-event1-r2` and `native-02fd8b1-timer1`. Signal frames are
reported counts, not a guaranteed 48,000-frame delivery rate. An exit of 0
reflects the probe's existing bounded acceptance threshold; it does not
override these zero-quality failures.

| Direction / channels / format | Event signal | Event silent | Event dropped writes | Timer signal | Timer silent |
| --- | ---: | ---: | ---: | ---: | ---: |
| Playback / mono / PCM16 | 47,040 | 0 | 0 | 45,056 | 2,048 |
| Playback / mono / Float32 | 47,040 | 0 | 0 | 46,080 | 1,024 |
| Playback / stereo / PCM16 | 47,040 | 0 | 0 | 46,080 | 1,024 |
| Playback / stereo / Float32 | 47,040 | 0 | 0 | 47,104 | 512 |
| Microphone / mono / PCM16 | 47,040 | 480 | 1 | 48,000 | 0 |
| Microphone / mono / Float32 | 47,520 | 0 | 0 | 47,520 | 0 |
| Microphone / stereo / PCM16 | 47,040 | 480 | 1 | 47,520 | 0 |
| Microphone / stereo / Float32 | 47,040 | 480 | 1 | 46,080 | 0 |

Every case in these two runs reports zero native discontinuities; the timer
run reports zero dropped writes in every case. All six default-device
readbacks are identical before and after each run. No revision-mismatch
failure appeared in these bounded runs, but event Microphone quality and
timer Playback startup silence remain unresolved. No production graph
rerun, held-client rename, calibrated physical latency or long-duration
qualification is claimed.

Host archives, including guest payload manifests, exit metadata, stdout and
stderr, are:

- `target/vad-target-archive/20261027-native-02fd8b1-event1/`
- `target/vad-target-archive/20261027-native-02fd8b1-timer1/`
- `target/vad-target-archive/20261027-native-02fd8b1-event1-r2/`
- `target/vad-target-archive/20261027-driver-02fd8b1/`

All run payload lengths and SHA-256 hashes were checked after transfer and
checked again on the guest before deletion. The package archive retains the
signed files, manifest, installation/restart log, previous installation logs
and final cleanup inventory. Temporary unsigned/signed staging directories,
superseded probes, the temporary signature verifier, completed run directories,
wrappers and copied manifests were removed only inside the validated
`C:\MoirenVadTest` tree, after rejecting reparse points.

At `2026-10-27T10:27:03+08:00`, the guest retains only
`C:\MoirenVadTest\vad_native-02fd8b1.exe` (392,704 bytes) in that test tree.
There are no active test processes or present dynamic VAD children; the
installed `MoirenVad` service remains Running. Free space is 30.107 GiB.
The guest is `DRIVER-TEST`, Windows `10.0.26200`; the earlier mixed-shell
observation of `10.0.26300` was the development host and is not guest evidence.
`C:\DriverTest`, installed/rollback DriverStore packages and system files
were preserved. Future builds and evidence retention remain on the host;
the guest is used only for the current bounded test.

### 2026-10-27 playback preroll after gate admission

`3289ded` moves the native probe's render preroll from `Stream::prepare` to
`Stream::start`, after `open_gate` has completed and immediately before native
Start. `VadSetGate` deliberately clears native storage before opening admission;
the old ordering erased the already submitted nonzero first buffer. The fix
keeps that revocation/silence fence and the prequeued four playback READs.
No kernel code, buffer size, freshness predicate or signal threshold changed.

The target-only regression `scripts/tests/vad-native-playback.tests.ps1`
rejected the actual archived `02fd8b1` timer report at mono PCM16
(45,056 matching frames, 2,048 silent frames), then accepted all four playback
shapes in fresh `3289ded` timer and event reports. The 33 native example tests,
strict example/test Clippy, formatting, full Windows-audio crate tests and
release build pass. Independent static spec/quality review passes.

The unchanged running `oem31.inf` SYS was verified before testing against SHA-256
`C8D6644A7EEF6AB63EE1F713BB4BCEF6464921343AF1A467DFC39093167D705F`.
The new probe is 392,704 bytes, SHA-256
`C94E20164F7E7B483B8FBD342010F24F42DA7DF9B5CD4D39119E9ED721594CB9`.
Both one-second-per-case matrices exit 0 without timeout:

| Direction / channels / format | Event signal | Event silent | Event dropped writes | Timer signal | Timer silent |
| --- | ---: | ---: | ---: | ---: | ---: |
| Playback / mono / PCM16 | 47,040 | 0 | 0 | 47,104 | 0 |
| Playback / mono / Float32 | 47,040 | 0 | 0 | 47,104 | 0 |
| Playback / stereo / PCM16 | 47,040 | 0 | 0 | 47,104 | 0 |
| Playback / stereo / Float32 | 47,040 | 0 | 0 | 47,616 | 0 |
| Microphone / mono / PCM16 | 47,520 | 0 | 0 | 47,520 | 0 |
| Microphone / mono / Float32 | 47,520 | 0 | 0 | 47,520 | 0 |
| Microphone / stereo / PCM16 | 47,520 | 0 | 0 | 47,520 | 0 |
| Microphone / stereo / Float32 | 47,040 | 480 | 1 | 46,560 | 0 |

All cases report zero native discontinuities. All six system-default identities
are unchanged in each complete matrix. Event microphone still fails zero quality.

A five-second-per-case timer repeat exits 1 in Playback mono Float32 with
`0x8007051A`. Its retained diagnostic has 80,896 matching frames, zero observed
silent frames, zero native discontinuities and zero microphone drops. Admission
was stream generation 2 / gate epoch 4; the post-failure, pre-cleanup QUERY
reports the same stream Running with gate open and epoch 5, plus one dropped
packet. This establishes an epoch change during the case, not its exact cause or
a recovered/pass result. Earlier `ScheduleTimerDrivenStep` stall rebase is a
candidate path requiring separate evidence. No long run is justified.

Host archives `target/vad-target-archive/20261027-native-3289ded-timer1`,
`20261027-native-3289ded-event1` and `20261027-native-3289ded-timer5`
retain stdout, stderr, exit metadata, manifest and cleanup inventory, including
the failed run. Every payload length/SHA-256 was checked on the host and
rechecked on the guest before bounded deletion under `C:\MoirenVadTest`;
reparse points were rejected. At `2026-10-27T10:50:05+08:00` the guest again
holds only the current 392,704-byte probe, has no active test processes or
present dynamic children, and has 30.106 GiB free. Temporary runner and obsolete
probe are deleted. The driver installation and system/rollback packages remain.

### 2026-10-27 bounded freshness diagnostics and correlated microphone evidence

`78ce2a1` retains the first collected capture completion and the first
CaptureDropped completion in each case, on both successful and failed paths.
Each fixed record names the actual completion variant, one-based collection
ordinal, original WRITE sequence/QPC and collection QPC. Ordinal overflow and
unavailable collection timing remain explicit; collection order is not a native
packet index and the original WRITE QPC is not driver consumption time.

`1d30d78` retains a fixed, at-most-256-byte diagnostic per endpoint, reset on
reservation/new stream generation. It latches the first Run identity, first
admitted capture transfer and first freshness rejection. The original
one-actual-period predicate, branch order, status, copies, silence policy and
epoch fencing remain unchanged. The return of the already existing QPC call
is retained as check time; it does not replace callback QPC in the predicate.
Only passive QUERY/gate-close paths emit WPP records after releasing locks.
No audio callback formats logs or allocates memory, and no ABI change was made.

Independent review found two diagnostic-only defects before deployment:
exhausting all four stale requests could omit the first packet, and using
current snapshot frames could relabel a historical observation after a shape
change. Both now have demonstrated RED/GREEN regressions compiling the actual
`VadPcmTransferLocked` body against host DDI stubs. A NoCurrentRequest outcome
covers budget exhaustion; actual frames are retained with each observation.
Re-review passes. All seven driver suites and the final analyzed WDK build
pass with zero warnings/errors and valid INF. The first intermediate build
failed because the new header imported user-mode stdint into the kernel;
using the existing ABI header's fixed-width types corrected it before the
successful final build.

All 36 completion-diagnostic example tests pass, including three new RED/GREEN
regressions; strict example/test Clippy and formatting pass. The full
Windows-audio crate test run passes. The remaining workspace test run
(`--workspace --exclude moiren-windows-audio`) passes 927 tests / 3 ignored
across 42 result suites, with no warnings/errors. These are host software tests.

The package was test-signed with the existing trusted certificate. Guest
SignTool verification of both SYS and INF as signed CAT members passes;
`pnputil` installed `oem32.inf` version `11.8.4.985`, then the exact
`ROOT\MEDIA\0000` root was restarted without active test clients. The service is
Running from
`C:\Windows\System32\DriverStore\FileRepository\moirenvad.inf_amd64_d385926bf68350e6\MoirenVad.sys`.
Installed lengths/hashes match the host signed package:

| File | Length | SHA-256 |
| --- | ---: | --- |
| `MoirenVad.sys` | 107,928 | `B2033C162ACA9587EB3F1D4034CAE288392485325FF80C80EFAD9A38D249DECB` |
| `MoirenVad.inf` | 1,803 | `28D9E0BEF0452E19D62941D062E777C95D522A5D443FC7DCB024388C42395579` |
| `moirenvad.cat` | 2,474 | `3A063731F5B9801E7E366A61E381C4FA7C729A00F4A6D7966A0726F2371AE893` |

Probe source `1d30d78` is 400,384 bytes, SHA-256
`B0B230970275B5B5B715AA71FE94ABA601B679F58CBB823237F8EB6EA57E7E4F`.
Its five-second full event matrix fails in Playback mono Float32 with epoch
4 -> 5 after 4,320 matching frames; its one-second repeat fails in Playback
mono PCM16 with epoch 5 -> 6 after 10,560 matching frames. Both exit 1 without
timeout, report zero observed silent frames/discontinuities in the failing
phase, and never reach microphone exercise. Their bounded ETL files are not
microphone evidence.

`10a33ae` therefore adds explicit `--intent=playback|microphone` diagnostic
selection. Default still executes all eight cases. A selected report names
`intent_filter` and contains only that direction's four cases; all four owned
endpoints are still mapped, default-checked and cleaned up. The parser test
first failed on the unsupported flag, then all 37 example tests, strict
example/test Clippy, formatting and host release build passed. Independent
spec/quality review passes. This subset cannot qualify the full native matrix.
The new probe is 400,896 bytes, SHA-256
`12C812E13E40A032432AA40E80411926B4C60A9CDA4AE29CDCBCD7B856DBBFA5`;
the running kernel package remains pinned to `1d30d78`.

With that probe, the five-second microphone subset fails in stereo PCM16
after 162,720 matching / 480 silent frames, zero collected dropped writes and
zero native discontinuities. Pre-cleanup QUERY reports epoch 1 -> 2 while the
same stream remains Running/gate open. The earlier mono cases have kernel
TooOld records at ages 11.6481 ms and 10.1933 ms; the matrix error does not
retain earlier successful CaseReports in stdout, so their user completion
identities are unavailable. No complete-subset claim follows from this run.

The one-second microphone repeat completes all four selected cases and exits 0
without timeout, with six unchanged default-device IDs:

| Channels / format | Signal frames | Silent frames | Dropped writes | Native discontinuities |
| --- | ---: | ---: | ---: | ---: |
| mono / PCM16 | 46,080 | 1,920 | 1 | 0 |
| mono / Float32 | 46,080 | 1,440 | 1 | 0 |
| stereo / PCM16 | 47,520 | 0 | 0 | 0 |
| stereo / Float32 | 47,040 | 480 | 1 | 0 |

The successful JSON now correlates the actual first dropped completion with
the kernel rejection by sequence and original QPC. Each has period=100,000
ticks (10 ms) and frames=480; all three hit TooOld, not NewerThanCallback:

| Case | Collection ordinal / WRITE sequence | Native packet | Header QPC | Callback QPC | Check QPC | Age |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| mono PCM16 | 70 / 70 | 71 | 435459859959 | 435459972572 | 435459972641 | 11.2613 ms |
| mono Float32 | 1 / 98 | 0 | 435465902784 | 435466009430 | 435466009517 | 10.6646 ms |
| stereo Float32 | 1 / 296 | 0 | 435492358878 | 435492459628 | 435492459703 | 10.0750 ms |

This proves startup freshness rejection in two cases and a later rejection in
another case in this run. It does not identify every empty packet or explain
all observed silent frames, nor prove that every historical drop has the same
cause. First-submit phase, queue admission/empty intervals and callback stall
timing still need causal investigation. No freshness tolerance, buffering,
quality threshold or epoch recovery was changed to obtain an exit of 0.

Each trace session uses a 1 MiB circular disk cap. The OS selected eight
64 KiB trace buffers despite the requested smaller minimum, still a fixed
512 KiB allocation. Both microphone traces decode with zero EventsLost,
format errors or unknown events; every wrapper reports trace stop code 0.
The complete microphone subset is a quality failure, not physical evidence
or a stable native candidate.

Host archives retain package/manifest/TMF/install/cleanup evidence under
`target/vad-target-archive/20261027-driver-1d30d78` and all four new runs:

- `20261027-native-1d30d78-event5`: 73,030 payload+manifest bytes.
- `20261027-native-1d30d78-event1`: 72,920 bytes.
- `20261027-native-10a33ae-mic-event5`: 138,922 bytes.
- `20261027-native-10a33ae-mic-event1`: 208,210 bytes.

All payload lengths/hashes were verified on the host and rechecked on the
guest before deleting each run. Installation staging, superseded probes,
copied manifests, signature/trace tools and temporary scripts were removed
only from validated, non-reparse paths below `C:\MoirenVadTest`. At
`2026-10-27T11:27:22+08:00`, that directory contains only
`vad_native-10a33ae.exe` (400,896 bytes), with no active test processes or
present dynamic children and 30.103 GiB free. Installed SYS was rechecked
against the recorded signed hash. System and rollback packages remain.

### 2026-10-27 native first-WRITE phase and mode-specific regression

`7d8de8c` anchors the first microphone WRITE deadline to native Start
acknowledgement before the Running QUERY, with half-period headroom. The
observed mono Float32 startup fixture first failed at runtime in the added
regression, then all 38 example tests, strict example/test Clippy, formatting
and the host release build passed. Independent review found no blocking
defect in the event path. Driver freshness, subsequent completion pacing,
pending-request bound, buffers and quality thresholds were unchanged.

The 401,408-byte probe has SHA-256
`F912DE1BBFCB03DE8DF9CA0EE43C26286ADF8E4C3B9B6BE0D1D46B9002A90467`.
The guest SYS still matches the recorded `1d30d78` / `oem32.inf` hash.
Its one-second event microphone subset exits 0, with all six defaults unchanged:

| Channels / format | Signal frames | Silent frames | Dropped writes | First native packet age |
| --- | ---: | ---: | ---: | ---: |
| mono / PCM16 | 47,040 | 960 | 0 | 1.6595 ms |
| mono / Float32 | 47,040 | 480 | 0 | 4.5155 ms |
| stereo / PCM16 | 47,040 | 480 | 0 | 4.1081 ms |
| stereo / Float32 | 47,520 | 0 | 0 | 4.6932 ms |

All four first native packets consume matching user WRITE identities with
unchanged 10 ms freshness; native discontinuities are zero. This removes the
previous first-packet rejection in this short repeat, not all silent intervals
or VM scheduling failures. The five-second event repeat exits 1 in mono PCM16
with 960 signal / 2,400 silent frames, one dropped WRITE and epoch 1 -> 2
while Running/gate open. The first callback at Run+10.9699 ms has no request;
the first WRITE is not stamped until Run+16.8441 ms. Native packet 5 later
rejects request 3 at age 11.0810 ms. No Start-acknowledgement QPC was retained,
so this record cannot attribute that initial delay to QUERY, setup or
preemption, nor establish the exact epoch-reset trigger.

The one-second timer subset exposed a separate probe regression in this
commit: timer capture consumes at its first logical segment, about 1.3--2.2 ms
after Run in these records, rather than at the event packet boundary. Delaying
its first WRITE misses the first logical packet: all four kernel records say
NoCurrentRequest. The subset exits 0 but retains 2,048 / 1,024 / 1,024 / 493
silent frames, so it is a quality failure. The correction must preserve
immediate first timer submission while retaining event headroom.

All three traces decode with zero lost events, format errors or unknowns.
Each session remains capped at 1 MiB. The six files in each run, including
manifest, were checked by length/SHA-256 on the host and rechecked on the guest
before immediate deletion. Host archives:

- `target/vad-target-archive/20261027-native-7d8de8c-mic-event1`: 207,652 bytes.
- `target/vad-target-archive/20261027-native-7d8de8c-mic-event5`: 138,811 bytes.
- `target/vad-target-archive/20261027-native-7d8de8c-mic-timer1`: 143,165 bytes.

These byte totals cover original payloads plus manifest, excluding host-decoded
text and cleanup metadata. The old probe was archived and hash-verified before
replacement. Temporary runs and the runner are already removed; the small
trace tool is retained only for the immediate mode-correction retest.

`1d6c85a` restricts the half-period start phase to event microphone and
restores immediate timer submission. Its timer regression first failed on
the delayed path; 39 example tests, strict example/test Clippy, formatting
and release build then passed. On the VM, three timer cases have zero silence
and a successful first packet. Mono PCM16 still has 2,048 silent frames:
the first segment runs at Run+1.5584 ms, before WRITE at Run+3.9375 ms.
The missing request is a startup ordering issue; immediate after-Start
submission alone does not guarantee it arrives before the timer callback.

`ee0cf79` therefore prepares sample buffers before Start and prequeues one
timer microphone WRITE while Paused/gate-open. Playback retains four prequeued
READs, while event microphone has no prequeued WRITE and keeps its half-period
deadline. The production-used startup helper has RED/GREEN ordering coverage
for all four direction/mode combinations and submission-failure suppression
of Start. All 41 example tests, strict example/test Clippy, formatting and
release build pass. Independent review finds no blocking defect. Early Start
errors still use existing PcmOwner RAII cancellation/drain; this slice does not
establish an independent bounded native teardown guarantee.

The final 401,920-byte probe has SHA-256
`CD885C7BAF548E91B16908A039D42C8FDEB22B4ED8D7A3DBAD49A7E195C05AB3`.
Both one-second microphone subsets exit 0, with defaults unchanged and zero
native discontinuities:

| Channels / format | Timer signal | Timer silent | Timer drops | Event signal | Event silent | Event drops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| mono / PCM16 | 48,000 | 0 | 0 | 45,120 | 1,920 | 2 |
| mono / Float32 | 48,000 | 0 | 0 | 47,040 | 480 | 0 |
| stereo / PCM16 | 47,520 | 0 | 0 | 47,520 | 0 | 0 |
| stereo / Float32 | 46,560 | 0 | 0 | 46,560 | 960 | 1 |

All first native packets now match a successful WRITE. The timer first WRITE
stamps precede Run and pass the unchanged freshness check; no timestamp was
fabricated or moved to callback time. Event still has later TooOld rejections:
mono PCM16 native packet 7 / request 7 at 13.6823 ms, and stereo Float32
native packet 53 / request 350 at 10.4927 ms. The fixed first-packet phase is
not a repair for later callback stalls or empty queue intervals. No stable
candidate, physical latency, long-run or full matrix gate passes here.

The additional archives retain payloads plus manifests at:

- `target/vad-target-archive/20261027-native-1d6c85a-mic-timer1`: 274,232 bytes.
- `target/vad-target-archive/20261027-native-ee0cf79-mic-timer1`: 208,692 bytes.
- `target/vad-target-archive/20261027-native-ee0cf79-mic-event1`: 208,013 bytes.

All three traces decode without lost events, format errors or unknowns. Host
length/hash checks and a second guest verification preceded each run's
deletion. Obsolete probes were replaced only after exact new/old hash checks;
host archives retain their binaries. At `2026-10-27T12:00:38+08:00`, temporary
runners, trace tool and logs are removed. The VM retains only
`vad_native-ee0cf79.exe`, with 30.108 GiB free, no active test processes or
present dynamic children, and the unchanged installed SYS hash. No system or
rollback package was removed.

### 2026-10-27 first real service/native rename attempt

`54029a3` adds explicit `vad_integration --exercise --rename`, mutually
exclusive with PCM/soak. It uses the existing production service/control
owner, actual initialized exclusive WASAPI clients and exact
`PKEY_Device_FriendlyName` readback, with separate Playback/Microphone cases,
retained phase/cleanup evidence and the existing overall watchdog. All 67
example tests, strict example/test Clippy, release build and independent
review pass; default/help remain non-exercising. This is an acceptance probe,
not a driver naming repair or PCM test.

The 7,552,512-byte release probe has SHA-256
`CAA93C5D2BA5696B33B0145FD4D1DBF1A567C380295A37977AE96319183E92FE`.
Its first VM run exits 1 after 2,066 ms of reported probe work, without
watchdog or storage-cap timeout. Both clients initialize and are released;
the service joins and releases its control session. Neither direction reaches
replacement initialization or the new-name gate. The held loop first sees
Removing, then Absent with absent diagnostics/provider name, causing
`missing current observation generation` in both cases. The old MMDevice
object still returns its previous ID/name; that does not prove its native
stream or PDO remains valid.

This exposes an incorrect probe assumption, not proof of unsafe driver
recreation. The driver REMOVE path does not check ActiveStreams before
revoking and calling AcxDeviceRemoveCircuitDevice. Holding an initialized
IAudioClient is not a guaranteed PDO-lifetime pin. The fake `fail_remove`
service regression covers a removal refusal; it does not establish that
every real held client must produce one. Do not change driver removal
semantics merely to satisfy this probe assumption. Keep the failed artifacts
and continue release/replacement/name diagnostics separately without promoting
the strict failed attempt into a pass.

The actual baseline Windows names are:

- Playback: `扬声器 (7- Moiren virtual audio endpoint)`.
- Microphone: `Moiren integration microphone (6- Moiren virtual audio endpoint)`.

They differ from provider spec names. No replacement-name claim follows from
these baseline strings. Exact readback must remain distinct from provider DTO
success; do not strip the parent suffix or use a substring comparison to pass.

Eight original files, including manifest and project/preferences fixtures,
total 45,106 bytes at
`target/vad-target-archive/20261027-rename-54029a3-r1`. Every payload length and
SHA-256 was verified on the host, then rechecked against the host checklist
on the guest before deletion. The wrapper bounds the run to 150 seconds,
32 files, 2 MiB per file and 8 MiB total with 500 ms cap checks; caps did not
trigger. A too-long encoded cleanup command failed before executing; the
same checked cleanup succeeded via a copied short script, then removed itself.
At `2026-10-27T12:06:43+08:00`, only the current integration probe remains,
with 30.101 GiB free and no test processes/dynamic children. System and
rollback files are untouched.

`5893bbf` adds a separate, feature-gated historical native retirement record.
Only the native adapter's fresh, error-free, gate-closed actual Removed
snapshot can supply it; ordinary Absent/Removing and missing/failed evidence
cannot. It retains the exact stamp and accepted sequence independently of
current presence, up to 64 latest-per-export entries with oldest-first
eviction. Repeated identical Removed polls do not refresh the sequence;
availability loss or a new control session clears the history. No reconcile,
dispatch, readiness or gate logic reads it. The missing-evidence regression
first failed at runtime, then 38 service/export tests, strict lib/test Clippy,
feature-off all-target check, formatting and independent review passed. The
exact oldest-eviction regression also passes. This closes a read-only evidence
projection gap, not the real rename acceptance or Windows name discrepancy.

### 2026-10-27 regression and guest-retention checkpoint

The full app run with `gpui-test,vad-integration-probe` completes with 500
passed, four ignored and no failures across 20 suites. The earlier workspace
run is not a pass: the virtual-only owner observation test failed its existing
one-second Running deadline. An exact isolated repeat passes, followed by a
complete Windows-audio library repeat with 478 passed and two ignored. These
repeats do not establish the cause of the original failure or erase it; no
deadline was relaxed. Logs remain under host `target/20261027-parent-*.log`.

At `2026-10-27T12:31:58+08:00`, a fresh SSH inventory confirms the guest test
root contains only `vad_integration-54029a3.exe` (7,552,512 bytes, the verified
hash recorded above), with 30.099 GiB free, no probe processes and no present
dynamic children. Builds and archived evidence remain on the host. Subsequent
runs must keep the same archive/length/SHA-256/recheck/delete sequence and
replace the old binary rather than accumulating probes on the 64 GB guest.

### 2026-10-27 bounded rename continuation and exact-name failure

`a660865` preserves the historical strict-held failure and adds separately
bounded retirement/replacement diagnostics. Both directions receive a share
of the work deadline; a missing direct Removed checkpoint can consume at most
500 ms or one quarter of its remaining direction budget. Replacement presence
and native initialization reserve time for actual name readback before Engine
confirmation. Budget regressions were observed RED, then all 77 example tests,
strict example/test Clippy, formatting, release build and independent review
passed. A diagnostic sequence reaching its end is not an acceptance pass.

The 7,608,320-byte probe has SHA-256
`1689819A556A0DD75FAF90C8E65EF124EB1684A09D9EFB3009326FE8661C5234`.
Its first VM repeat, still using the unchanged `1d30d78` driver, exits 1 with
3,297 ms of reported work and no watchdog/storage-cap timeout. Both Playback
and Microphone retain the original `missing current observation generation`
strict-held failure, but now complete the diagnostic continuation:

- Direct old Removed is observed at accepted sequences 58 and 127, newer than
  baseline sequences 44 and 123, with exact old PDO/session/endpoint identity.
- Fresh native initialization succeeds. PDO/endpoint generations advance from
  44 to 46 and 45 to 47 under the same control session. Engine confirmation
  succeeds at fresh observation sequences 110 and 179.
- Both old and replacement clients are released; service join and control
  session release succeed with no cleanup errors.
- Exact Windows name readback fails in both directions. Playback remains
  `扬声器 (7- Moiren virtual audio endpoint)` rather than
  `Moiren rename Playback 11460`; Microphone remains
  `Moiren integration microphone (6- Moiren virtual audio endpoint)` rather
  than `Moiren rename Microphone 11460`. Textual MMDevice/adapter IDs are reused;
  the native generations, not those strings, establish recreation.

These observations separate a real Windows naming defect from the earlier
incorrect lifetime assumption. They do not establish whether Windows cache,
pin/category selection or device/interface property composition causes it.
Neither run nor native rename acceptance qualifies; exact-name checks remain
unchanged. Eight original files (68,720 bytes) are retained under host
`target/vad-target-archive/20261027-rename-a660865-r1`. Host length/SHA-256
verification and a second guest recheck preceded deletion of all run files
and helpers. At `2026-10-27T12:43:51+08:00`, only the current integration probe
remains and the guest has 30.099 GiB free; system/rollback files are untouched.

The full workspace repeat now passes 1,625 tests with eight ignored across
71 suites (`target/20261027-parent-workspace-repeat.log`). The earlier failed
Running-deadline run remains recorded; its cause is still not established.

### 2026-10-27 bounded clock-rebase instrumentation, not yet deployed

`f53f6c7` adds a 216-byte first-rebase record per endpoint/stream generation,
capturing actual event/timer cause, callback/deadline/lateness, clock pre/post
and the atomic PCM epoch transition. The RT path records fixed data only;
QUERY/gate-close emits at PASSIVE_LEVEL after unlocking. Eight host C++ suites,
WDK Analyze, InfVerif and independent review pass; freshness, buffers,
scheduling and invalidation behavior are unchanged.

Host packaging found a new tooling failure before VM deployment: tracepdb
reports `Event too complex` for the original combined Clock rebase event and
omits its decoding data from TMF. Therefore that package is not installed and
no live clock-cause claim is made. A follow-up must split the bounded passive
payload into correlated decodable fragments, verify generated TMF, then sign
and verify the package on the already-trusted guest. Host signature trust
verification also correctly rejects the test root, which is intentionally
not added to the host trust store; guest chain/catalog verification remains
mandatory before installation.

### 2026-10-27 decodable clock diagnostics and bounded native repeats

`78e0e34` splits the passive first-rebase output into cause/before/after
fragments with 13/14/14 fields and the same endpoint/session/generation/stream/
callback-QPC tuple. The new production-body tests fail on the original large
event, then pass with matching fragments and unchanged first-only semantics.
Eight host suites, analyzed WDK build, InfVerif, independent review and actual
tracepdb/TMF inspection pass without warnings or omitted fields. No RT callback,
freshness, packet size, queue target or scheduling behavior changes in this fix.

The signed package is installed only on the VMware guest after guest
catalog-membership and trusted-chain verification of both SYS and INF:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| SYS | 114,072 | `3762BBDDD7FF3715F8231ECA31233EFCE7B2A01775CA23A2ABA40DA0C5E0D6D6` |
| INF | 1,805 | `59123AEA16634D362F1937670C3340504241DA83B69AB4EC82A264500CEA3A05` |
| CAT | 2,474 | `7E682446FE5348AF57A1DB3A683F017B4A438480D28B6E81CB8711CEB06DB9BD` |

PnP publishes `oem33.inf`; the restarted root reports the driver Running at
`moirenvad.inf_amd64_9b20f2da60729718`. Installed package hashes match the host.
The host retains package, matching TMF and install evidence in
`target/vad-target-archive/20261027-driver-78e0e34`. Six temporary installation
files are rechecked and removed from the guest; DriverStore/rollback remain.

The unchanged `ee0cf79` native probe runs three serial microphone subsets:

| Channels / format | Event 5s signal / silent / drops | Timer 5s signal / silent / drops | Event 15s signal / silent / drops |
| --- | --- | --- | --- |
| mono / PCM16 | 237,600 / 1,920 / 1 | 239,520 / 0 / 0 | 719,040 / 480 / 0 |
| mono / Float32 | 239,520 / 0 / 0 | 240,000 / 0 / 0 | 718,560 / 960 / 0 |
| stereo / PCM16 | 239,520 / 0 / 0 | 239,520 / 0 / 0 | 716,160 / 3,360 / 3 |
| stereo / Float32 | 239,520 / 0 / 0 | 231,360 / 0 / 0 | 714,240 / 5,280 / 5 |

All wrappers/probes exit zero with unchanged defaults and zero native
discontinuities. That is not the zero-silence/drop acceptance gate: event
subsets remain unqualified. In event 5s, the first TooOld rejection has
WRITE-header-to-callback age 10.8752 ms; event 15s stereo PCM16/Float32 first
rejection ages are 10.3499 ms and 10.2844 ms. These use callback QPC, as does
the actual freshness predicate, not the later check-QPC diagnostic. All three
ETLs decode with zero lost events, format errors and
unknowns, but contain **no Clock rebase fragments**. No actual first-rebase
cause is established in these windows; ordinary lateness/empty PCM queues
must not be misreported as a proven clock reset. Timer samples and 5/15-second
subsets cannot establish full-frame coverage, long-run stability or physical
latency, nor a VAD-versus-Process winner.

Each six-file payload/manifest archive is length/SHA-256 verified on the host
and rechecked on the guest before deletion. Archives are named
`20261027-native-78e0e34-mic-event5`, `...-timer5` and `...-event15` under
`target/vad-target-archive/`. The trace tool is also verified and removed.
At `2026-10-27T12:52:11+08:00`, the guest test root contains only the current
401,920-byte native probe, has 30.103 GiB free, and has no probe processes or
present dynamic children. Stage 7 remains open.

The two event-15s first rejections match the user completion sequence/stamp
exactly. Relative to the nominal RUN-plus-packet boundary, callbacks occur
1.0780 ms and 0.5445 ms late; the corresponding WRITE ages at those nominal
boundaries would be 9.2719 ms and 9.7399 ms. This explains why a sub-packet
delay can cross the unchanged 10 ms freshness limit without causing a clock
rebase. Callback QPC is sampled after obtaining stream State, so the records
do not distinguish DPC dispatch delay from lock acquisition. Later empty
queues are also not retained by the existing first-transfer/first-rejection
record. Additional fixed first-unserved and safe submission-boundary evidence
is needed before choosing any timing repair.

### 2026-10-27 actual MMDevice naming metadata

`58ed492` adds read-only MMDevice DeviceDesc and DeviceInterface FriendlyName
observations to before/held/after evidence, preserving typed HRESULT/type/null/
UTF-16 errors instead of substituting provider values. Optional metadata cannot
change the existing exact FriendlyName or topology gates. All 82 example tests,
strict example/test Clippy, formatting, release build and independent review
pass. The 7,618,560-byte probe has SHA-256
`63411B800419085428E528662C106D26EE092036899DEA01754A633AF2CB02BB`.

The repeat on installed `78e0e34` again exits 1 without watchdog/storage-cap
timeout, with 3,621 ms reported work. Both directions retain the strict-held
and exact-name failures while direct old Removed, fresh native initialization,
fresh replacement/Engine confirmation and all releases succeed. Actual
property-store values are unchanged before and after recreation:

| Direction | DeviceDesc | DeviceInterface FriendlyName | Final FriendlyName |
| --- | --- | --- | --- |
| Playback | `扬声器` | `7- Moiren virtual audio endpoint` | `扬声器 (7- Moiren virtual audio endpoint)` |
| Microphone | `Moiren integration microphone` | `6- Moiren virtual audio endpoint` | `Moiren integration microphone (6- Moiren virtual audio endpoint)` |

All three values are directly read from the actual MMDevice, not inferred
from the final-name string. They expose which descriptive values remain old,
but do not by themselves prove a registry/cache or pin-category root cause.
A fresh diagnostic identity with a distinct initial CREATE name can separate
initial name publication from same-ID re-enumeration behavior; this must not
change product stable IDs or bypass exact-name acceptance.

Eight original files (74,694 bytes) are archived at
`target/vad-target-archive/20261027-rename-58ed492-r1`; host and guest length/
SHA-256 checks precede deletion. At `2026-10-27T12:55:19+08:00`, the guest
contains only the current 7,618,560-byte integration probe, with 30.096 GiB
free and no active test processes or present dynamic children. System,
DriverStore and rollback files are unchanged by cleanup.

### 2026-10-27 explicit fresh-identity naming control

`d786851` adds opt-in `--rename-fixture-seed=positive_u64`, allowed only with
`--exercise --rename`. Default fixtures are unchanged. The explicit seed
selects distinct diagnostic endpoint identities and initial CREATE names;
within each rename case the same stable ID and graph binding are preserved.
It is not a product workaround or an instruction to rotate IDs on rename.
All 87 example tests, strict example/test Clippy, release build and independent
review pass. The 7,624,192-byte release probe has SHA-256
`44CBFBF9AA0AA673F98F2EF81A5BEA69F136357EBA01143EB409510776388D1C`.

Seed `202610271303` produces a new Playback/Microphone pair on `78e0e34`.
The actual initial microphone DeviceDesc is
`Moiren integration microphone seed 202610271303`, proving that the initial
requested pin description reaches this fresh endpoint. After same-ID
recreation it still has that initial description, not the requested rename.
Playback DeviceDesc is `扬声器` both before and after; adapter descriptions
remain `25- Moiren virtual audio endpoint` and
`26- Moiren virtual audio endpoint`. Final names remain the corresponding
composed old strings. Exact-name checks fail unchanged.

Direct old Removed, native initialization, fresh replacement/Engine
confirmation and release pass for both directions. Native PDO/endpoint
generations advance 17 to 19 and 18 to 20 under one session. The historical
strict-held failures are retained (Playback native identity/generation changed;
Microphone missing current observation generation). The run exits 1 with
4,009 ms reported work, no watchdog/storage cap and no cleanup error. This
separates initial publication from same-ID refresh behavior but does not by
itself prove the underlying Windows storage/consumption cause.

A separate read-only interface-registry observation for the preceding default
fixture sees the new requested name in both conventional FriendlyName and
EP/0 FriendlyName under Audio and the matching Render/Capture interface
Device Parameters. Actual MMDevice names remained old. The protected
Properties subtree rejects traversal; no ACL or property is modified. These
post-removal persistent interface values do not prove which interface/name
EndpointBuilder actually consumed while the endpoint was live. The successful
read is retained in the host `rename-58ed492-r1` archive, separate from its
original payload manifest.

The fresh-ID run's eight original files total 77,870 bytes at
`target/vad-target-archive/20261027-rename-d786851-r1`. Host/guest length and
SHA-256 checks precede immediate deletion. At `2026-10-27T13:09:34+08:00`, only
that current integration probe remains, with 30.095 GiB free. No system,
DriverStore or rollback files are removed. Native naming and Stage 7 remain
unqualified.

### 2026-10-27 bounded native submission and unserved correlation

`6cc7713` adds a fixed 248-byte per-endpoint first-unserved record for Empty,
NoCurrent and TooOld. It captures DPC entry and after-State QPC, the nominal
event packet boundary, bounded retrieval outcome, and the latest matching
WRITE reservation-side pre-forward boundary. That boundary is not actual
queue insertion or proof that forwarding succeeds, and it is not necessarily
the request rejected by a later callback. Passive correlated WPP fragments
retain the existing State-to-Shared lock order, admission/freshness/epoch
policy and PCM behavior. Nine host C++ suites, analyzed driver build,
InfVerif, generated TMF checks and independent review pass.

`edb5ab8` adds a fixed 64-entry user WRITE submission ring, first completion,
first drop and up to four neighbors on each side. The pending FIFO retains
the opaque request ticket until its actual completion supplies header
sequence/QPC. Submit-before/after QPC and Instant-based target/lateness are
distinct observations, not substitutes for the internally stamped header.
All 46 native-example tests, strict example/test Clippy and release build
pass; scheduling, queue depth and freshness remain unchanged.

Driver `6cc7713` is installed as `oem34.inf`; its signed SYS is 119,192 bytes,
SHA-256 `6031B0E13F7EB3F7172F2E6173FC13542521823444651EBBF0095C5CFF5CD30E`.
The current `edb5ab8` native probe is 430,592 bytes, SHA-256
`59FE949ABA24B1717CA12E6A21DEBE0D873CD0AAD26103BA4649D68F58C00799`.
Package/PDB/TMF/install evidence is retained on the host in
`target/vad-target-archive/20261027-driver-6cc7713`, and the probe is separately
archived in `20261027-probe-edb5ab8`.

The requested four 15-second event-driven microphone cases abort during
mono Float32, stream generation 6. The wrapper exits 1 after 23.5 seconds,
without timeout, with `GetOverlappedResult(VAD PCM)` revision mismatch
`0x8007051A`. No complete four-case report exists: stdout is empty, and stderr
retains the structured failure plus the original error chain. Failed-case
observations are 65,280 signal frames, 10,080 silent frames, seven WRITE
drops and zero native discontinuities; these are not a quality pass.

The ETL decodes 20 events with zero lost events, format errors or unknown
formats. Unlike the preceding short windows it retains an actual event
Clock rebase for endpoint 3, session 504639348358, stream generation 6:

| Observation | Actual value |
| --- | --- |
| Callback QPC / next deadline QPC | 504829286678 / 504829245627 |
| Lateness past that next deadline | 41,051 ticks at 10 MHz = 4.1051 ms |
| Packet frames / packets / completed | 480 / 2 / 158 |
| Clock before epoch / base / limit | 504813345627 / 0 / 75,840 |
| Clock after epoch / base / limit | 504829286678 / 75,840 / 76,320 |
| Atomic gate change | 8 to 9, both open and Running |

The current serviced packet's nominal boundary is one packet earlier than
that next deadline: packet 157 is due at QPC 504829145627, so its actual
completion is 14.1051 ms late. At 10 MHz the 480-frame period is exactly
100,000 ticks; both deadlines match the absolute RUN epoch without a
rounding/phase drift. This record proves a rebase and gate change, not whether
DPC dispatch or lock acquisition caused the lateness. The later pre-cleanup
QUERY sees the same stream, gate 9, Present/Running/open, cumulative drops 15
and silence 68. It is not an atomic observation of the failed request.

Earlier correlated first-unserved records in that same stream show:

| Observation | First Empty | First TooOld/final unserved |
| --- | --- | --- |
| DPC entry QPC | 504819549394 | 504819655540 |
| After-State QPC | 504819549414 | 504819655561 |
| Nominal packet boundary QPC | 504819545627 | 504819645627 |
| Latest reservation request | 1552 | 1553 |
| Latest header QPC | 504819424882 | 504819550081 |
| Latest pre-forward QPC | 504819427838 | 504819551707 |
| Retrieval calls / final status | 1 / NO_MORE_ENTRIES | 2 / NO_MORE_ENTRIES |

For the Empty callback, DPC-entry-to-State is 2 microseconds and the State
sample is 0.3787 ms after its nominal boundary. Request 1553 is submitted
shortly after that empty callback. For TooOld, the actual PCM callback QPC
504819655563 gives header age 10.5482 ms, crossing the unchanged 10 ms limit;
State is 0.9934 ms after the nominal boundary. The matching user ticket 62
retains header sequence 1553 and header QPC 504819550081, submit-before/after
QPC 504819550048/504819550702, and target/entry elapsed times
618,125,700/620,520,700 ns (2.395 ms submission lateness). The driver
reservation can happen after the asynchronous user call returns. These
earlier records cannot establish DPC-to-State delay at the later rebase.

All six original run files are length/SHA-256 checked on the host and
rechecked before deletion from the guest. The archive is
`target/vad-target-archive/20261027-native-6cc7713-mic-event15`.
At `2026-10-27T13:24:26+08:00`, a fresh guest inventory confirms only the
430,592-byte current native probe remains in `C:\MoirenVadTest`, with
30.101 GiB free, no test process and no present dynamic child. Temporary
installation/trace/runner/run files have been removed; Windows, DriverStore
and rollback packages are preserved. Builds and historical artifacts stay
on the host. Stage 7, zero-quality native stability, exact Windows naming
and calibrated physical latency remain unqualified.

### 2026-10-27 typed interface publication experiment

`2121277` retains ordinary FriendlyName/EP0 staging and adds typed
`DEVPKEY_DeviceInterface_FriendlyName` publication on each actual registered
symbolic link, before ACX exposure. The passive system worker sets one
persistent plain UTF-16 string and reads it back into a fixed-size buffer;
status, exact type/length/content/terminator mismatches fail policy closed.
Link ownership/rollback is retained. No MMDevice property write, category or
stable-ID change, retry loop, RT allocation or user-name trace is added.

The new production-extracted test first fails against the old staging path.
All ten host C++ suites, three negative mutations, independent review, parent
host rerun, analyzed WDK build, InfVerif and warning-free TMF decoding pass.
The typed-name WPP record has exactly six fields. These checks prove bounded
publication/readback, not EndpointBuilder consumption or a final name fix.

The signed package is installed on the VM as `oem35.inf`; installed SYS is
121,240 bytes with SHA-256
`F517517781C8A41D47AAC259665CB39EF22BE127261EA66C54B2FC0D2BD0A376`.
The host package/PDB/TMF/install archive is
`target/vad-target-archive/20261027-driver-2121277`.

The unchanged `d786851` integration probe uses unused diagnostic seed
`202610271333`, retaining each stable ID within its rename. The run exits 1,
reports 3,771 ms work, and has no watchdog/storage-cap/cleanup failure.
Direct old Removed, fresh replacement, exclusive native initialization,
Engine confirmation and client/service/control release pass in both
directions. Historical strict-held failures remain sticky, as before.

All twelve typed publication/readback records (three categories for each
direction, initial and recreated PDO) report success, type 18, exact returned
bytes and match=1. Initial names occupy 92/96 bytes and requested rename
names 56/60 bytes. Nevertheless actual MMDevice values remain unchanged:

| Direction | Actual DeviceDesc before and after | Actual interface description before and after |
| --- | --- | --- |
| Playback | `扬声器` | `28- Moiren virtual audio endpoint` |
| Microphone | `Moiren integration microphone seed 202610271333` | `27- Moiren virtual audio endpoint` |

Final FriendlyName remains the corresponding composed old name, not
`Moiren rename Playback 1652` or `Moiren rename Microphone 1652`.
Exact-name acceptance fails unchanged. This experiment rules out a failed
typed set/readback on the staged owned links in this run; it does not prove
which interface EndpointBuilder consumed. Stop adding name setters and
resolve the actual MMDevice-to-KS-filter/interface mapping before selecting
another naming change.

The 168-event ETL has zero lost events, format errors or unknown formats.
Ten original run files total 408,986 bytes; host length/SHA-256 verification
and guest recheck precede deletion. The archive is
`target/vad-target-archive/20261027-rename-2121277-r1`. Installation staging,
runner, run files and trace tool are removed immediately after their checks.
At `2026-10-27T13:34:37+08:00` the guest holds only the current 7,624,192-byte
integration probe with 30.093 GiB free. System, DriverStore and rollback
packages are preserved. Exact Windows rename and Stage 7 remain open.

### 2026-10-27 probe-only microphone cadence experiment

The correlated request-1553 window supports a probe scheduling defect:
each completion is collected before a new half-period delay is started,
so collection/wakeup lateness can move a WRITE beyond an empty native
callback. Its later TooOld is distinct from the subsequent clock rebase.
The production application worker already advances a successful packet's
intended phase, and is not changed by this probe repair.

`e4aef04` advances successful completion cadence from the preceding intended
submission phase plus one whole packet period. An overdue deadline permits
one immediate WRITE, never a catch-up burst; pending remains at most one.
First/unanchored and dropped completions retain the existing half-period
re-arm. Submission at least one period late discards its stale phase.
Startup, driver policy, freshness, queue capacity, PCM and epoch failures
remain unchanged. Shared submission/cadence Instant is explicitly sampled
before the helper, distinct from the unchanged QPC submit/header records.

The actual scheduling helpers fail three new regressions before the fix.
All 52 example tests, strict example/test Clippy, scoped formatting, release
build, independent review and parent test/Clippy rerun pass. The release
probe is 431,104 bytes, SHA-256
`5AA92F468FF9C58E093CFB24041959D49202949CDF3CA522F66552F32EDA4EDB`;
host archive `target/vad-target-archive/20261027-probe-e4aef04` preserves it
before replacing the guest's integration probe.

On unchanged installed driver `2121277`, the four 15-second event microphone
cases now finish with exit 0 and no watchdog, silence, WRITE drops or native
discontinuities:

| Channels / format | Signal frames | Silent frames | WRITE drops | Native discontinuities |
| --- | --- | --- | --- | --- |
| mono / PCM16 | 720,000 | 0 | 0 | 0 |
| mono / Float32 | 719,520 | 0 | 0 | 0 |
| stereo / PCM16 | 719,520 | 0 | 0 | 0 |
| stereo / Float32 | 719,520 | 0 | 0 | 0 |

All use 480 buffer/driver packet frames and 1,500 WRITE submissions. The
eight-event ETL decodes without loss/error/unknown formats and contains only
the four first-transfer diagnostics, no unserved or clock rebase fragments.
This is an improved short diagnostic window, not proof of exact full-frame
coverage, long-run stability, minimum-period support or physical latency.
Six original files are host/guest length/SHA-256 verified and removed from
the guest at `2026-10-27T13:44:28+08:00`; host archive
`target/vad-target-archive/20261027-native-e4aef04-mic-event15` retains them.
No default or qualification gate is promoted by this result.

The separate four-case 15-second timer microphone subset also exits 0 with
zero silence, WRITE drops and native discontinuities. Its actual aligned
buffer and driver packet frames are 2,048 (mono PCM16), 1,024 (mono Float32
and stereo PCM16), and 512 (stereo Float32). These are actual observations
for this run, not assumptions from page capacity or the initial 10 ms
request. Signal frames are 719,520 for the first three cases and 694,080 for
stereo Float32, below 720,000; zero counters alone cannot establish complete
waveform/frame coverage. All timer native positions advance. The ETL again
has eight events, no unserved/rebase fragments and zero lost/error/unknown
events. The original six files are checked on host and guest and removed;
host archive `20261027-native-e4aef04-mic-timer15` retains the incomplete
signal observation without upgrading it to qualified.

At `2026-10-27T13:46:17+08:00`, after checked removal of the runner and trace
tool, only the current 431,104-byte native probe remains in the guest test
root, with 30.099 GiB free and no present dynamic child/test process. No
guest build tree or historical run archive is retained. Exact full coverage,
small-period negotiation, long-run stability and physical latency stay open.

The subsequent requested four-case 60-second event subset does not complete:
it exits 1 after 185.9 seconds during stereo Float32 with the original
`0x8007051A` revision mismatch. There is no complete stdout report. Failed
case counters retain 181,440 signal frames, 1,440 silent frames, one WRITE
drop and zero native discontinuities. A prior stereo PCM16 case also has
retained first-empty/TooOld events. The improved 15-second windows therefore
do not demonstrate stability at longer durations.

For endpoint 16, stream generation 2, the actual rebase callback is QPC
524377028991. The next deadline is 524377003575 (2.5416 ms exceeded), while
the packet being serviced was due at 524376903575 (12.5416 ms late).
Atomic open/Running gate changes 4 to 5. In this window the independent
first-NoCurrent record is from that same callback: DPC entry 524377027924,
State acquired 524377028990, nominal serviced deadline 524376903575. Thus
the callback enters 12.4349 ms late and entry-to-State is 106.6 microseconds;
the late callback is not explained solely by time acquiring State. This
does not distinguish late timer arm from dispatch/host scheduling delay,
nor justify relaxing epoch or freshness protection.

Earlier in that case request 18372 (user ticket 373) has header QPC
524376107809, target/entry elapsed times 3,725,477,200/3,730,529,700 ns,
5.0525 ms submission lateness, and actual TooOld callback QPC 524376214086
(10.6277 ms header age). The distinct empty callback occurs between its
user header and reservation-side pre-forward boundary. An accepted async
submission is not evidence of completed queue insertion before the callback.

All six original files are checked and removed from the guest; the
23-event ETL decodes with zero loss/errors/unknown formats. Host archive
`target/vad-target-archive/20261027-native-e4aef04-mic-event60` retains the
entire failure. At `2026-10-27T13:54:15+08:00`, only the current 431,104-byte
probe remains, with 30.099 GiB free; runner, trace tool and run files are
removed after verification. The qualification remains failed/open, not a
reason to enlarge queues, weaken freshness or ignore the changed gate.

### 2026-10-27 read-only MMDevice-to-interface topology follow-up

`b7896f3` retains a bounded optional observation from the actual held
MMDevice topology connector. Its `GetDeviceIdConnectedTo` result is preserved
exactly (UTF-16 bounded, with no trimming, prefix removal or path guessing)
and offered once to read-only SetupAPI. Raw SetupAPI rejection HRESULTs,
configuration-manager identities, and typed interface friendly-name
readbacks remain separate fields. These diagnostics never replace the exact
MMDevice FriendlyName, alter ownership/removal/Engine gates or make a failed
observation stop the rename continuation. The HeldClient does not retain extra
COM/SetupAPI objects between observations.

All 94 integration-example tests, strict example/test Clippy, scoped format
check, release build and independent review pass. On the VM the seeded
`202610271357` rename probe exits 1 after 3.8 seconds without watchdog,
storage-cap or cleanup failure. Direct old Removed, fresh replacement/native
initialization, Engine confirmation and release still pass both directions;
the pre-existing strict-held and exact-name failures remain sticky.

The actual topology adapter IDs are the Audio-category `MoirenEndpoint`
interface IDs under the matching MOIREN/VAD_RAW PDO, for the exact playback
and microphone stable IDs. The exact IDs are retained in
`target/vad-target-archive/20261027-rename-b7896f3-r1/evidence/report.json`.
Passing those opaque values unchanged to `SetupDiOpenDeviceInterfaceW`
returns HRESULT `-536870363` (`0xE0000005`) for both directions, both before
and after same-ID recreation. No SetupAPI PDO/parent or interface FriendlyName
readback is consequently available; no speculative normalized retry is
attempted. This is a concrete failed mapping observation, not evidence that
the typed publication targeted a different interface or that Windows cache
caused the name result.

MMDevice DeviceDesc and final FriendlyName still retain the original values
for Playback (`扬声器`, composed with adapter 29) and Microphone (initial
seed description, composed with adapter 30), while exact requested rename
names fail. The experiment narrows the question to resolving the opaque
topology adapter ID to its real system interface path using a documented
mapping API; it does not justify another setter, ID rotation, or acceptance
relaxation. The 122-event ETL has zero lost, malformed or unknown events.
Ten original run files are host-length/hash verified and guest-rechecked
before deletion; the host archive is
`target/vad-target-archive/20261027-rename-b7896f3-r1`. The trace utility,
runner and run files are removed immediately. The guest retains only the
7,657,472-byte current integration probe with 30.092 GiB free. Windows,
DriverStore and rollback packages are untouched.

### 2026-10-27 explicit exclusive minimum-period diagnostic

The user-level latency objective remains calibrated physical p95 <= 10 ms for
the 48 kHz `Input -> Bus -> Output` graph with no other processing nodes, plus
stable continuous output and zero unexpected drops, silence, underruns,
discontinuities or epoch resets. A minimum-period request is only a bounded
feasibility experiment, not a latency result or a change to production defaults.

`405f6ec` adds opt-in `vad_native --minimum-period`; the baseline request path
is unchanged. The report keeps queried default/minimum, initial/final requested
period, alignment retry, native buffer capacity and driver packet size separate.
All period fields explicitly remain `unqualified-request-only`; no requested
value is represented as actual stream cadence. The 60 native example tests,
feature-enabled all-target strict Clippy, format check and release example
build pass. The code commit was pushed before the VM run.

On `DRIVER-TEST`, the release probe was verified at 518,656 bytes and SHA-256
`3C31131FBB26537E758EB882DF4709AF21E372F7B1F9EF37FD1F3DC5E625AAFA`. The
15-second-per-case microphone event subset failed in its first Mono PCM16 case
after about 0.48 seconds of submission: queried default 100,000 hns (10 ms),
minimum/request 30,000 hns (3 ms), native buffer 144 frames, and driver packet
144 frames. There were 109 WRITE submissions and 36 dropped completions; the
first completion was already dropped. The diagnostic counted 10,368 signal and
12,096 silent frames, with zero WASAPI native discontinuities. The first write
was 8.9626 ms late at submission entry against a 9.8905 ms target deadline. A
post-failure QUERY observed `gate_open=true` and gate epoch 2 after admission
at epoch 1, then `GetOverlappedResult(VAD PCM)` returned `0x8007051A`.

The epoch advance is consistent with the driver's event-deadline clock rebase:
`SchedulePacket` rebases a missed packet deadline, and the callback invalidates
old PCM by advancing the admission epoch without closing the gate. The QUERY is
later than the failing completion and does not prove that a rebase caused this
run's first drop. The failure archive has no matching WPP ETL, so causality is
still open. The next reproduction must correlate the first-drop QPC neighborhood
and its neighboring WRITE submissions with WPP `Clock rebase` cause and
before/after state. Until then, the proven facts are the late first submission,
the initial dropped completion, subsequent drops/silence, an epoch change, and
the revision-mismatch result. The explicit 3 ms request fails the short
zero-drop gate; it neither qualifies a smaller cadence nor establishes whether
smaller packets help.

The runner captured `TimedOut=false` and completed in 2.13 seconds, but its
PowerShell `ExitCode` field is null; the failure classification therefore uses
the raw HRESULT, counters and cleanup evidence, not an asserted process exit
code. Guest payload lengths and SHA-256 were checked against its manifest
before cleanup. Only the 7,657,472-byte integration probe
`vad_integration-b7896f3.exe` remains in `C:\MoirenVadTest`; its hash is
`AC9C985B6BDAD102E5999EB9A2F590E2A0895C3D9C75AA4BFC7D2492CF910708`, and
30.092 GiB is free at the latest post-run inventory. The guest temporary native
probe, runner and run directory were removed; the installed driver, Windows
files, DriverStore and rollback packages were not touched. The VM's 64 GB
system disk has 30.092 GiB free and `C:\MoirenVadTest` contains only the
integration probe needed by the still-open rename check. Host failure evidence
is retained at
`target/vad-target-archive/20261027-native-405f6ec-mic-event15-minimum/`; the
executed `run.ps1` and `cleanup.ps1` helpers have also been removed, leaving the
verified 518,656-byte probe binary and five raw run-evidence files (547,071
bytes total).

Do not promote minimum-period mode or claim VAD is faster than Process Loopback
from this result. Continue with the specific scheduling/gate failure and
period negotiation investigation, then compare both application and native
paths under the same source/output, measurement and quality gates. Buffer
capacity remains configurable separately from normal queue residency; reducing
capacity alone is not evidence of lower end-to-end latency.

Execution order and acceptance gates are recorded in
`docs/superpowers/plans/2026-10-26-vad-live-qualification.md`: reproduce and
correlate the first drop with driver clock diagnostics; fix only a demonstrated
driver or client defect; pass a short zero-drop gate before a separate period
matrix; revalidate the VAD application path; compare VAD and Process Loopback
under the same physical setup; sweep one buffer target at a time; then require
30-minute and two independent two-hour stable runs before default promotion.
Every responsibility-sized code or evidence update is to be staged narrowly,
verified, committed atomically, and pushed before moving to the next stage. The
VM is for driver compatibility/debugging; it cannot substitute for calibrated
physical latency qualification.

### 2026-10-27 current-build VAD collection repeat

The release `e2e_latency_probe` from source commit `80ccf8d` was built on the
host, then copied to `DRIVER-TEST` only after confirming its 3,843,584-byte
length and SHA-256
`69437FAEF67200E8DB499FEE5A4C5CC7AA65A6203B2807BF838EE4256263C539` on both
machines. Read-only endpoint inventory confirmed current capture
`{0.0.1.00000000}.{55E7D663-AD05-4729-A5D8-CEA7688D9A92}` and render
`{0.0.0.00000000}.{71789a68-9934-4137-9e1a-ba0a5730aa68}`. The 15-second,
48 kHz run used stable VAD ID `...0019`, the production
`VirtualPlayback -> Bus -> master output` graph, a 48,000-frame scheduled
source, an independent recorder, and supervised controlled load. The source
progressed for 0.484 seconds and the independent recorder for 0.553 seconds
before quality-gated teardown. `report.json` retained `status=unqualified`,
`qualified=false`, and incomplete collection/cleanup. The wrapper's guest
manifest serialization then failed because Windows PowerShell did not support
`-Encoding utf8NoBOM`; consequently the child process exit code is unavailable.
Do not interpret `observed_duration_seconds=0` in that partial report as the
native observation duration.

The report retains 20,160 of 48,000 acknowledged source frames; 39 VAD
completion-interval observations across 40 completed packets (the telemetry
legacy field named `completion_gaps` counts every monotonic completion
interval; it does not count missing packets); zero VAD driver drops, silence
frames or
epoch resets; one virtual boundary xrun and 33 soft events. The
independent WASAPI capture negotiated 279 actual frames despite a 245-frame
request (capabilities: 139 minimum, 488 default/maximum); its 84 packets and
23,400 frames contain 83 gap events, 1,873 gap frames and one discontinuity.
The controlled-load worker exited cleanly but covered 0.528 seconds of the
required 1.2 seconds, so its evidence is invalid. This repeat confirms the
existing early-quality failure in the integrated current-build path; it is not
a calibrated latency result, a VAD-versus-Process comparison, or evidence that
the 10 ms objective is met.

The seven raw guest payloads (report, collection result, manifest, PCM, packet
metadata and both input plans) are retained under
`target/vad-target-archive/20261027-collection-80ccf8d-15s/` with
`host-archive-manifest.json` and `summary.json`. Each raw payload's byte length
and SHA-256 matched between guest and host before cleanup. The test process was
absent; only the known run directory and five named current-run temporary files
were removed. The preserved `vad_integration-b7896f3.exe` was rechecked at
7,657,472 bytes with unchanged SHA-256
`AC9C985B6BDAD102E5999EB9A2F590E2A0895C3D9C75AA4BFC7D2492CF910708`. The
guest test directory now contains only that rename-test probe and has
32,309,325,824 bytes free (30.09 GiB). The VM driver, DriverStore, Windows
files and rollback packages were untouched. The host archive's raw payload
manifest excludes its derived summary and itself; the report remains the
authoritative run record.

Future guest runners must use an encoding option supported by Windows PowerShell
5.1 and preserve the actual child exit code even if later manifest writing
fails. Before any extended or candidate run, diagnose and remove the VAD
completion-interval timing, virtual boundary xrun, and independent capture
continuity failures under a short zero-quality-failure gate. Process Loopback
native smoke, same-condition comparison, physical reference/wiring and
calibration, stable output, and the
30-minute plus independent two-hour qualification runs remain outstanding.

### 2026-10-27 VAD collection repeat without controlled load

The same host-built `80ccf8d` collector, 48,000-frame VAD source schedule,
production `VirtualPlayback -> Bus -> master output` graph, and independent
WASAPI microphone recorder were repeated with controlled load disabled. The
runner still failed to retain the child exit code; report duration zero is not
treated as runtime, and this is one unqualified VM diagnostic, not latency
evidence.

The report records 20,640/48,000 acknowledged source frames over 0.429 seconds,
41 completed VAD packets, zero driver drops/silence/epoch resets, one boundary
xrun and 33 soft events. The 40 legacy-field `completion_gaps` count monotonic
completion-interval timing observations (maximum 11.0118 ms), not lost
packets. The independent capture retained 86 packets / 23,957 frames and
failed continuity validation: 85 device-position mismatches contributed 2,340
absolute frame differences, with two DISCONTINUITY-flagged packets. 84
mismatches were 17-23-frame regressions and one was a 444-frame forward jump.
These observations are not a direct count of lost capture frames. Teardown
also recorded a cancelled VAD
PCM I/O error. Removing controlled load did not eliminate the early boundary
xrun or capture failure, but this single repeat cannot establish root cause.

The nine raw payload lengths and SHA-256 hashes matched the guest and are
retained with their manifest at
`target/vad-target-archive/20261027-collection-80ccf8d-noload-15s/`. After
guest verification, only the run's explicitly named directory, inputs, logs,
and temporary collector were removed. The VM keeps only the unchanged
7,657,472-byte `vad_integration-b7896f3.exe` rename probe; its SHA-256 remains
`AC9C985B6BDAD102E5999EB9A2F590E2A0895C3D9C75AA4BFC7D2492CF910708`, with
30.09 GiB free. Driver, Windows, DriverStore and rollback files were untouched.

### 2026-10-27 bounded provider trace and output-only cadence control

Runner `59e781b` keeps the original child handle and numeric exit result,
then publishes artifact hashes only after both redirected output tasks have
successfully drained and their streams have closed. A real child that exits
while an independently cooperative descendant holds its inherited pipes
reproduces the original defect. With the repair, the receipt records
`output_state=unavailable` and `artifact_manifest_state=unavailable`, retains
partial logs and withholds the manifest. The original runner fails this
regression; the repair passes all nine PowerShell 5.1 behavior groups.
Independent spec and code-quality review passed.

Collector `d64250f` initializes the existing workspace logger only when
`RUST_LOG` is explicitly set. It parses the filter strictly and writes to
stderr without a file writer. The handle lives through CLI cleanup, including
native owner joins. The example passed 126 tests, strict example Clippy,
focused formatting and a release build. Five real release `--help` invocations
verified default silence, target filtering, invalid-filter exit 2, stderr-only
logging and unchanged successful stdout. Independent review passed. The
matching `routing_probe` release also built successfully; it was not modified.

These target runs used the same render and independent capture endpoints as
the earlier collections. Their reviewed binaries were host-built, copied
individually and hash-checked before execution:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `e2e_latency_probe.exe` | 5,784,576 | `61E2FDC6FDC47C376C11DBA169366548CBC5148604EB0B5FC8DB4ECC100EDB02` |
| `routing_probe.exe` | 2,652,160 | `73AB74EE20914C8F079F44C178BA062D3A39362FE0B2F4F9C3CF202C951B3CA0` |
| `vad-collection-run.ps1` | 9,017 | `32AD3AB9C6401BCE8ECDC08A95DCF539FF9B3FC1DB9B593E565D1841F1100088` |

The VAD run used the 15-second collection ceiling, no controlled load and
`RUST_LOG=info,moiren::audio::vad=debug`. The runner preserved actual exit
code `1`, confirmed process termination, stable output and the three-artifact
manifest; it returned after 5.2138878 seconds without forced termination.
The run retained 49 startup observations and provider debug records, but
stopped early with one boundary xrun, 33 soft events and incomplete collection.
All final provider drop/silence/recovery/epoch-reset counters were zero.
A native render wake interval was 350.8985 ms. Between positive-queue control
observations at QPC 628742753788 and 628745814790, Engine progress was 16,320
frames over 0.3061002 seconds (about 53,316 frames/s), while completed packets
advanced from 6 to 37 at 480 frames each (about 48,612 frames/s). The queue
was observed at 2,384 frames and later at 467 before exhaustion. These sampled
observations suggest a rate mismatch, without assigning its underlying cause.
The independent recorder still failed its own continuity gate: 75 packets,
20,892 frames, 74 position mismatches with 1,669 absolute position differences
and one discontinuity flag; zero tap drops, overflow, silent frames or
timestamp-error packets. Position differences do not directly count missing
PCM frames.

The graph's physical output is explicitly `ClockRole::Master`, and backend
preparation selects `WindowsTransport`. It does not execute the independent
virtual-owner QPC debt schedule; absence of `virtual_clock_gap` output is
therefore expected and cannot prove or disprove this failure's cause.

The separate two-second output control ran:

```text
routing_probe --output {0.0.0.00000000}.{71789a68-9934-4137-9e1a-ba0a5730aa68} --seconds 2 --profile compatible --trace
```

It has no VAD or capture, mixes two software tones behind zero-gain guards,
and uses the same physical output. The runner records exit `0`; the report
records healthy routing, muted output, closed host and clean shutdown.
Both client and native mix report 48 kHz, a 480-frame period and native clock
frequency 384,000. Its 224 callback timestamps span 2.0054339 seconds.
The 223 submissions after the first total 107,040 frames, about 53,375
frames/s, with median wake interval 8.9781 ms. Full native elapsed time is
2.0089849 seconds and total submissions are 109,152 frames including priming.
The first retained wake has empty padding; it is a startup observation, not
a clean physical-quality result. Faster-than-nominal output consumption thus
reproduces independently of VAD. Its native-device, format or QPC cause
remains unproven. This diagnostic graph and its successful control exit do
not qualify the reference graph or physical latency.

Commit `778c964` retains the paired `IAudioClock` device position/QPC and
HRESULT in each bounded render trace record, while preserving the existing
`CallbackObservation` construction API. The latency trace schema is now 2 and
the WASAPI render report schema is 10. The Windows audio crate's full test run
passed (479 library tests / 2 ignored, plus integration suites), along with
strict Clippy, formatting and diff checks. A fresh
two-second output-only control on `DRIVER-TEST` used the release
`routing_probe` from this commit (`BF10761710B99D24993983A25D3CCE79C798BF73EA34E93DCEB2BBE24BE775C5`)
and exited 0 without timeout or forced termination. Its 224 callbacks retain
successful 48 kHz client/native format and a 480-frame period. Across the
device-clock QPC span of 2.0132517 seconds, the native position advanced 852,536
units at the reported 384,000-unit frequency: approximately 52,933 frames/s
after conversion to 48 kHz frames. Subsequent render submissions advanced
107,040 frames over the wake-QPC span of 2.01325 seconds, approximately 53,168
frames/s. The device-position and submitted-frame rates exceed nominal 48 kHz
by approximately 10.28% and 10.77%, respectively. This does not identify whether the
cause is the VM/native clock or its QPC time base, is not physical latency,
and does not qualify the unmuted `Input -> Bus -> Output` path. The receipt,
runner artifacts, compact cleanup inventory and analysis are in
`target/vad-target-archive/20261027-778c964-render-control-2s/`; each
runner-owned file's guest-manifest length and SHA-256 matched before the
guest run directory and temporary probe were deleted. The VM returned to five
retained input/probe files and approximately 30.08 GiB free.

Device-position/QPC slopes still need comparison with an independently timed
native control; distinguish device advancement from VM time-base behavior
before assigning a cause. Stage 7, source comparison and calibrated physical
latency remain unqualified.

The host archives retain the executed scripts, input provenance, raw payloads,
guest inventories and derived analysis:

- `target/vad-target-archive/20261027-collection-d64250f-providertrace-15s/`:
  11 raw payloads totaling 1,552,998 bytes; the prior 3,855,360-byte probe
  was also copied and hash-checked on the host before replacement.
- `target/vad-target-archive/20261027-collection-d64250f-render-control-2s/`:
  four raw payloads totaling 67,674 bytes, plus final guest inventory.

Raw payload counts exclude host-authored provenance, analysis and cleanup
records. Each guest payload length/SHA-256 matched on the host, and the guest
was rechecked against that same inventory before scoped directory deletion.
The collector cleanup's initial direct encoded command exceeded the target shell
command-line limit after archiving; no evidence was lost or deleted by that
failure. The same cleanup logic was transferred as a hash-checked script,
executed, and removed. The temporary routing probe and all command helpers
were removed too.

The `d64250f` checkpoint's `C:\MoirenVadTest` inventory was five files: current collector,
`vad_integration-b7896f3.exe`, runner and two JSON input plans; no test
processes or present dynamic VAD children, and 30.081 GiB free. The guest
retains no run directories or superseded collector. Driver/package, device
defaults, Windows and DriverStore were not changed. Next investigate native
output demand against QPC/device-clock/format progression before another
integrated repeat; keep capture qualification separate and do not enlarge
buffers or weaken quality gates to mask the VM behavior. Physical calibration,
same-condition Process comparison and the sustained qualification windows
remain open; the 10 ms target is not achieved.

### Device-clock trace follow-up and guest cleanup

Independent review of `778c964` found no RT allocation or audio correctness
blocker, and identified two follow-ups. Commit `cb5d5df` restores
`GetPosition` after the padding query, matching the documented preexisting
order, and exercises `Some(device_clock)` under the allocation/free guard
through all 256 slots and 17 overflow observations. Saved clocks and raw
HRESULTs must remain unchanged. All 11 latency tests, 28 Windows render
tests, strict all-target crate Clippy, formatting and diff checks pass.
These changes have host verification only; the following guest binaries
were built from `778c964`, before this sampling-order correction.
The schema-2 public `TraceRecord` adds `device_clock`, so Rust consumers
using exhaustive construction or patterns must add that field or use
`..Default::default()` / `..`. The input `CallbackObservation` and
`LatencyRecorder::record()` interfaces retain their existing shape.

The matching collector is 5,786,624 bytes, SHA-256
`23B9661CC8BF09BE593E128127CDBC24C87FBE1550FFA0B92302473424A9A38E`.
Two attempted 5/6-second collection ceilings were rejected before native
startup with `collection phase budget exceeds --seconds`; both returned
actual exit 2 and are parser failures, not audio results. For the current
one-second source plan the code reserves 13,320 ms, so an integer ceiling of
at least 14 seconds is required. Reusing the prior 15-second ceiling entered
the production path and returned actual exit 1 after 4.5701648 seconds,
without timeout or forced termination. It retained 55 startup samples,
one boundary xrun / 33 soft events, 53 completed VAD packets and zero
provider drops, silent frames, epoch resets or recovery events. Render trace
retains 91 callbacks and a 459.8513 ms maximum wake interval. Cleanup reported
`GetOverlappedResult(VAD PCM)` cancellation `0x800703E3`; its cause is not
established. The independent capture retained 106 packets / 29,530 frames,
105 position mismatches / 3,211 absolute position differences and four
discontinuity flags, with zero tap drops, silent frames, overflow packets or
timestamp errors. The collection remains incomplete and unqualified.
The inventory helper described below was still running during this collection.
Its uncontrolled memory/CPU pressure invalidates a no-load or same-condition
comparison; the 459.8513 ms wake interval cannot be assigned solely to baseline
VM, native driver or VAD behavior.

Raw diagnostics are in
`target/vad-target-archive/20261027-778c964-live-vad-trace-15s/`.
The three runner-owned receipt/stdout/stderr files match the guest manifest
in length and SHA-256. Seven probe files (including raw PCM and packet
metadata) were copied before cleanup, but the guest directory was deleted
before a separate guest hash inventory was retained for those seven files.
Host hashes are recorded as such; full cross-machine integrity verification
cannot be claimed for this run, and it must not be used for qualification.
The two parser-failure receipts and stderr files are retained separately.
The old 5,784,576-byte collector was copied to the host and its original
length/SHA-256 verified before its guest copy was removed.

A read-only inventory helper accidentally serialized PowerShell's attached
provider metadata and reached about 9.59 GB private memory, causing the
system-managed pagefile to expand. It overlapped the 15-second-ceiling VAD
collection and was stopped only afterward. The exact helper PID/encoded command was
verified and stopped; Windows subsequently reduced its pagefile from
4,421 MiB to 512 MiB. The final live inventory reports 30.070 GiB free,
no probe processes or present dynamic VAD children, and exactly five files:
the `778c964` collector, the rename integration probe, runner and two JSON
inputs. All completed guest run directories, superseded probe and staging
copies are gone. Keep remote inventories to explicitly selected scalar
fields; obtain a guest length/hash inventory for every payload before
deletion and preserve raw receipt bytes during host verification.

### 2026-10-27 VAD name readback and target cleanup

Commits `ca57e14`, `fc99d2b`, `9ea627f`, `9d45291`, and `744f596` add
control-side Windows endpoint-name diagnostics across the provider, app
service, and UI. The report distinguishes the persisted/requested name, the
driver provider's CREATE name, and the name Windows actually resolves from the
owned MMDevice identity. The Windows reader enumerates all MMDevice states,
then still requires an exact root instance, stable ID, and flow match; an
inactive endpoint is observable without implying that it is active or passing
the Engine gate. This is diagnostic readback, not a rename repair.

The target is `DRIVER-TEST` at `192.168.111.128`, reached with the pinned
`moiren-test` SSH identity. The latest run used
`vad_integration-9d45291-allstates.exe` (7,781,376 bytes, SHA-256
`D7E3C04D50A25CB55CD8F0D46FB63A7DF6611C113F3A72B14C6BB8A788AD396C`). Its
three run directories each contained six payloads plus their manifest. Host
copies and the guest were independently checked against manifest file counts,
lengths, and SHA-256 before deletion. The latest all-states report is retained
at `target/vad-target-archive/rename-service-name-9d45291-allstates-20261027-195318/`.

Both directions recreated successfully with fresh native generations, exact
direct-Removed observations, Engine confirmation, clean client/service
teardown, and no cleanup errors. Rename acceptance still fails:

- Playback requested `Moiren rename Playback 6460`, and the provider reports
  that name, but MMDevice `PKEY_Device_FriendlyName` remains
  `扬声器 (35- Moiren virtual audio endpoint)`.
- Microphone requested `Moiren rename Microphone 6460`, and the provider
  reports that name, but MMDevice FriendlyName remains
  `Moiren integration microphone seed 202610271955 (36- Moiren virtual audio endpoint)`.
- Both Playback and Microphone cases retain a strict-held failure,
  `missing current observation generation`, when the provider is absent during
  recreation. Exact-name and lifecycle gates were not relaxed. The report is
  `unqualified`, not a pass.

Therefore users can edit and persist the desired VAD name and Moiren can send
the new CREATE name, but the driver/Windows endpoint stack does not yet
surface that name as the actual Windows endpoint FriendlyName. Do not claim a
complete end-user rename feature until exact readback passes for both flows.
Next trace which registered KS interface EndpointBuilder consumes; the latest
properties show Playback DeviceDesc `扬声器`, Microphone DeviceDesc retaining
the seed, and both DeviceInterface FriendlyNames retaining their numbered
Moiren suffix. Avoid adding further property setters until that mapping is
proven.

The guest initially held three completed rename run directories and three
probe executables (23,600,872 bytes total). After verifying all 18 payload
files against host archives, cleanup removed the three run directories and
two superseded probes, freeing 15,814,359 bytes under
`C:\MoirenVadTest`. The latest 7,781,376-byte probe remains. The C: volume now
has 32,287,281,152 bytes free (30.07 GiB); there are no active probe processes
or present dynamic VAD PDOs. A separate read-only inventory found 40 historical
`MOIREN\VAD_RAW` child nodes, all `Unknown` and non-present. Those stale child
nodes alone were removed with PnPUtil; a fresh inventory reports zero remaining
children. The installed `ROOT\MEDIA\0000` control bus remains present; its
driver package, DriverStore, Windows files, audio defaults, and rollback
packages were not touched. A separate inventory identified 35
`SWD\MMDEVAPI` AudioEndpoint records whose FriendlyName identifies the Moiren
virtual endpoint; each was `Unknown` and non-present. Those stale endpoint
devnodes were removed with PnPUtil as well. The post-cleanup inventory reports
no stale RAW children or Moiren MMDevice AudioEndpoint devnodes.

The VM is sufficient for lifecycle, property, and driver debugging, but cannot
qualify calibrated physical audio latency. The `Input -> Bus -> Output` target
remains p95 <= 10 ms at 48 kHz with continuous stable output and zero unexpected
drops, silence, underruns, discontinuities, or recovery resets. That target is
not achieved. First repair the probe observation-generation gap and resolve
the endpoint naming property source; then clear the VAD boundary xrun and
capture-continuity failures, characterize the VMware clock-rate mismatch, run
same-condition VAD/Process comparisons and configurable queue-target sweeps,
and finally perform calibrated 30-minute and two-hour physical qualification.
Buffer capacity, queue target/residency, device period, and measured physical
latency must remain separate quantities.

### 2026-10-27 ten-second output-clock control and immediate cleanup

The output-only `routing_probe` was built at `7de0c8c` and copied to
`DRIVER-TEST`. The 2,663,936-byte executable has SHA-256
`97CDFC5BB42D4CD761679FAD03D895E9EE8706DDCED383242B364DE627D12D6A`.
The exact output endpoint is
`{0.0.0.00000000}.{71789a68-9934-4137-9e1a-ba0a5730aa68}`;
arguments select Compatible, ten seconds, and tracing. The guest run began
at `2026-10-27T12:37:52.0085539Z`.

The archive is
`target/vad-target-archive/routing-clock-7de0c8c-10s-20261027-2037/`.
All four payloads match the guest manifest in byte length and SHA-256, and
the manifest itself was compared across machines before cleanup:

| Payload | Bytes | SHA-256 |
| --- | ---: | --- |
| `preflight.json` | 701 | `7E861AD6DBBE1C1C5B7CF7964826F953601C0C58F1032450DCD1E7A241F11905` |
| `receipt.json` | 609 | `3CDB54CCCCDD5C541DA5F37A0199A105846D360487F7146659D01AAD531802A6` |
| `stderr.log` | 73 | `2C53281038FF09C365B5F23C3804690F1412C41C1AF6C6284ADC7C57CFBA985E` |
| `stdout.json` | 129,421 | `B881446A49FEB1A7BE5C6CBEB6349754927064ACD0A42B14D8AFAC611FD28948` |

The application reports 10.0171921 active seconds, `healthy=true`,
`clean_shutdown=true`, no fault, and a stopped native render stream. The
original receipt nevertheless has `exit_code=null`; the wrapper did not
reliably capture the child exit code. It must stay unknown. Its
`clock_frequency=null` is also a wrapper extraction gap: the authoritative
`native.render.clock_frequency` is 384,000 units/s. The receipt's 20.359-second
elapsed value measures the wrapper, not the active stream.

At 48 kHz, native positions convert to frames by dividing by eight. The
native latency telemetry has 1,030 callback observations, but retains only
the first 256 exact trace records; the other 774 records exceed diagnostic
capacity. That counter is not a PCM-drop count. The retained native clock/QPC
pairs span 2.3287991 seconds, with positions advancing from 8,448 to 980,032
units (121,448 frames of progress). Their first-last slope is
52,150.483913 frames/s; OLS over all 256 pairs gives 53,024.191535 frames/s.
First-half and second-half OLS slopes differ: 54,488.565458 and
50,930.860809 frames/s. This changing, retained-window rate is not a fixed
calibration or a whole-run rate. All retained clock reads succeed; no native
QPC or position regression occurs. One adjacent native position is stationary.

Whole-run wake intervals have 1,029 samples, minimum 6.8002 ms and maximum
12.4233 ms. Their p50/p95/p99 histogram upper bounds are each 11.4687 ms;
these bounds are not exact quantiles. The retained 255 exact wake intervals
have nearest-rank p50/p95/p99 of 9.0922/10.0683/10.3861 ms. Whole-run callback
duration ranges from 0.0484 to 1.8547 ms; its p95 histogram upper bound is
0.1791 ms. No timestamp or wake regression counter is set. Timeout wakes,
zero-demand wakes and bridge shortfall frames are zero; the initial callback
has empty padding. Those counters do not establish PCM integrity.

The actual Client3 period is 480 frames (10 ms), with an advertised minimum
of 128 and fundamental of 32. Capacity/target are 1,056 frames (22 ms);
steady retained padding is 576 frames (12 ms). These are separate values,
not measured physical delay. Output is muted, with no input, capture,
nonzero sample or marker; physical end-to-end latency remains null.
Neither a VAD/Process comparison nor a Stage 7 pass can be inferred.

After host verification, the guest independently rechecked every payload
and the executable digest before deleting the run directory and this
temporary executable. Cleanup released 2,795,950 bytes. A fresh inventory
reports 32,286,109,696 bytes free (about 30.07 GiB), no active probe, and
`ROOT\MEDIA\0000` still `OK`. The sole remaining test file is the
7,781,376-byte rename probe required for follow-up testing. Keep this
archive-verify-delete sequence after subsequent runs; the 64 GB system VM
must not accumulate completed runs or superseded binaries.
