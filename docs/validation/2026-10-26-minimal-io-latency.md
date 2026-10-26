# Minimal I/O latency implementation and qualification evidence

## Source and scope

- Date: 2026-10-26.
- Branch: `feat/minimal-io-latency`.
- Source base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
- Current reviewed probe-quality implementation: `a57abde`, with reviewed
  bounded diagnostic collection repair `2c61452`; latest target probe and
  diagnostic run: `2c61452`. Service/UI slices are tracked separately below.
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
