# Minimal I/O latency implementation and qualification evidence

## Source and scope

- Date: 2026-10-26.
- Branch: `feat/minimal-io-latency`.
- Source base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
- Latest native target source: `ee0cf79`, including driver diagnostics
  `1d30d78` and pre-Run admission `25951de`. The matching test-signed
  package is installed as `oem32.inf`;
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

The quality evidence remains a failure: VAD completed 42 packets with 41
completion gaps; the virtual boundary recorded one xrun and 33 soft events;
the independent capture recorded 87 gap events, 2,128 gap frames and two
discontinuities. The complete seven-file run archive, including manifest,
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
packets and completed 300 with 299 completion gaps; the virtual boundary
reported 7 xruns and 135 soft events. Provider driver/carrier/stale/silence/
epoch/recovery counters were zero, but those counters do not override the
completion and boundary failures. The run used diagnostic pulses without a
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
subsets remain unqualified. In event 5s, the first TooOld rejection has age
10.8821 ms; event 15s stereo PCM16/Float32 first rejection ages are 10.3580 ms
and 10.2939 ms. All three ETLs decode with zero lost events, format errors and
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
