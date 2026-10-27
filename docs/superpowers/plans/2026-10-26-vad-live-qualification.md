# VAD Live Integration and Sub-10ms Qualification Plan

> For agentic workers: use `superpowers:subagent-driven-development` or
> `superpowers:executing-plans` to implement this plan task by task. Keep each
> task independently testable and do not promote a latency default before the
> qualification gates pass.

**Goal:** Complete the live VAD path, compare it with Process Loopback under
identical conditions, and qualify the smallest stable `Input -> Bus -> Output`
configuration whose calibrated physical end-to-end p95 is at most 10 ms.

**Architecture:** Keep the existing VAD driver ABI and application-level
remove/recreate rename protocol. Extend the existing latency probe with a
source-neutral evidence model and a VAD playback source, then measure both
sources through the same Engine graph, physical render endpoint, marker,
calibration boundary, and controlled-load profile. Keep requested buffer
intent, effective prepared budgets, empirical packet timing, and physical
end-to-end latency as separate facts.

**Tech Stack:** Rust 2024, WASAPI, ACX/KMDF VAD driver, existing
`WindowsVadProvider`, `WindowsBackend`, `EngineHost`, bounded PCM artifacts,
PowerShell target scripts, and the existing offline qualification evaluator.

## Global Constraints

- Reference graph is exactly `Input -> Bus -> Output`, 48 kHz, with no other
  DSP nodes.
- Product target is calibrated physical end-to-end p95 `<= 10 ms`; p50 `<= 8`
  ms is an exploration target, not a pass condition.
- A passing continuous window has zero unexpected drops, silent frames,
  underruns, marker gaps, discontinuities, and epoch/recovery resets.
- A queue capacity is storage headroom, not normal latency. A requested target
  is not an effective target until native preparation accepts it.
- Internal timestamps and WASAPI loopback are diagnostic evidence only. Only a
  calibrated physical cable or calibrated external dual-channel measurement
  can qualify physical end-to-end latency.
- Do not change device defaults, install a competing control owner, or hide a
  failed low-latency candidate by silently increasing its buffers.
- No RT allocation/deallocation, blocking, COM setup, filesystem I/O, JSON
  serialization, or unbounded collection is allowed on the audio path.
- Preserve the current epoch fencing, gate admission, microphone freshness,
  stable endpoint identity, and candidate-versus-active revision rules.
- Stage each responsibility-sized change separately and push it promptly. The
  current branch already contains the implementation commits through the
  native-probe cleanup; this plan records what is implemented, what failed on
  the target, and what still requires live qualification.
- The target VM has a 64 GB system disk. Build on the development host. Copy
  completed evidence, including failures, back to the host and verify file
  lengths and SHA-256 hashes before deleting guest copies. Remove superseded
  binaries and staging files promptly; retain the current probe, active-run
  files, and packages needed for the next install or rollback.
- The VM is a driver compatibility and dual-machine debug target, not a
  substitute for a calibrated physical audio measurement setup. Run physical
  latency qualification only on a real source/output path with the calibration
  boundary below.

## Decision Record

| Question | Current answer | Proof still required |
| --- | --- | --- |
| Can the driver rename an existing VAD endpoint in place? | No. The ABI has no `RENAME` IOCTL; the name is supplied only by `CREATE`. | None for the current product path. Add a driver rename protocol only if remove/recreate interruption becomes a demonstrated product blocker. |
| Can a user rename a VAD in Moiren? | The app implements stable-ID remove/recreate with the requested CREATE name and preserved graph binding. Its `actual_name` currently reflects the stored provider spec, not Windows property readback. | Real-service held-client rename, direct old-Removed evidence, and exact Windows MMDevice-name verification. The latest CREATE smoke still shows a generic Windows name; cause is unknown. |
| Is VAD integrated into Moiren? | Yes: the target production smoke exercised VAD MMDevice -> native PCM worker -> `VirtualPlayback` -> Engine -> Bus -> physical render. | The 10-second run had seven boundary xruns; full native, rename, and calibrated physical qualification remain open. |
| Which is faster, VAD or Process Loopback? | No defensible measured winner yet. Process Loopback is the shorter structural baseline; VAD has driver, native PCM, worker, and virtual-boundary work. | Same physical endpoint, graph, marker, render policy, load, and calibration. |
| Can application bypass buffering be reduced? | Yes, the input capacity/target/jitter/packet/gap and render target are configurable. Reducing capacity alone does not reduce normal residency. | Packet/gap telemetry and physical sweep must prove a candidate is stable. |
| Does audible doubling prove `>80 ms`? | No. Direct output plus a delayed Moiren path creates comb filtering or slapback at much shorter delays. | A marker correlation above 3,840 frames at 48 kHz, measured at the physical boundary, is required to claim `>80 ms`. |

## Execution Plan After the Minimum-Period Failure

### Current Baseline

- `405f6ec` is pushed. `vad_native --minimum-period` is opt-in and records a
  requested WASAPI period separately from actual buffer and driver packet
  sizes. It does not change the production default.
- On `DRIVER-TEST`, the first 144-frame mono microphone event case requested
  30,000 hns (3 ms); the client buffer and driver packet were both 144 frames.
  Its first WRITE entered 8.9626 ms after its 9.8905 ms target deadline. The
  first completion was dropped; the case then recorded 36 dropped writes,
  10,368 signal frames, 12,096 silent frames, and zero WASAPI discontinuities
  before `GetOverlappedResult(VAD PCM)` returned `0x8007051A`.
- A post-failure QUERY saw `gate_open=true` and `gate_epoch=2` after admission
  at epoch 1. The driver advances the PCM admission epoch on an event deadline
  rebase while leaving the gate open. This makes a clock rebase a concrete
  hypothesis, not yet a proven cause for this run: the next reproduction must
  correlate the probe's first-drop QPC neighborhood with the driver's WPP
  `Clock rebase` event and its before/after gate state.
- At the earlier minimum-period inventory, `C:\MoirenVadTest` retained the
  7,657,472-byte integration probe needed by the remaining rename check; the
  system disk had 30.092 GiB free. That host failure archive is 547,071 bytes
  and retains the verified probe plus raw run evidence, without the executed
  runner scripts.
- `5c143a6` completed the telemetry naming correction: schema 2 emits
  `completion_interval_observations` and `max_completion_interval_100ns`,
  with schema-1 JSON aliases retained for reading historical evidence.
  Completion intervals are timing observations, not missing-packet counts;
  all drop, silence, epoch and boundary-xrun gates remain unchanged.
- The two current-build collection attempts (with and without controlled
  load) both failed early with one virtual-boundary xrun and 33 soft events.
  Neither wrapper retained the actual child exit code. Startup samples have
  observation order but no sample timestamps: `health.stamp` contains PDO,
  session, endpoint, stream and gate identities, not elapsed milliseconds.
  Historical samples cannot establish the fault's QPC time. Capture
  device-position mismatches likewise do not directly count lost PCM frames.
- Commit `c346424` added bounded QPC control-read windows and Engine timeline
  frames to the startup trace. The no-load 15-second run retained 52 samples;
  its first render wake interval was 3,930,974 QPC 100-ns units (393.0974 ms)
  at QPC 601387562250. The same QPC appears on the independent capture's first
  packet. The Engine timeline advanced from frame 2,016 to 2,976 across the
  surrounding control observations, then continued while the VAD completion
  count advanced at about one 480-frame packet per 10 ms. The carrier was
  observed at 2,384 frames, later at 467, then starved with one xrun. This is
  evidence of an initial native render wake hiatus and subsequent carrier
  drain. The graph declares its physical output as `ClockRole::Master` and uses
  `WindowsTransport`, so the independent virtual-owner QPC debt schedule does
  not drive this path. The retained Engine timeline is sampled only by the
  control owner; it does not prove the native hiatus's underlying cause.
- A separate 2-second independent capture-only run, with no VAD playback or
  Engine graph, reproduced capture position discontinuities: 223 packets and
  61,913 frames over 1.379 seconds, 222 mismatches / 5,704 absolute position
  differences, three discontinuity flags, and zero reported dropped or silent
  frames, overflow, or timestamp errors. After three initial packets with
  `DATA_DISCONTINUITY` and repeated position zero, 221 subsequent packet
  positions advanced by 256 while their PCM packet sizes were 278/279 frames.
  Their QPC wake gaps were mostly 5-7 ms. Since the client and advertised mix
  rates are both 48 kHz, this VM capture-position behavior is independent of
  the VAD/Engine chain, but its underlying device/virtual-audio clock cause
  remains unproven. Do not treat the absolute position differences as a count
  of missing PCM frames or weaken the capture quality gate.
- Raw VAD-run evidence is at
  `target/vad-target-archive/20261027-collection-c346424-qpc-trace-noload-15s/`
  (11 files, 1,627,419 bytes). The independent capture-only evidence is at
  `target/vad-target-archive/20261027-capture-only-c346424-2s/` (7 files,
  532,034 bytes). Both archives were length and SHA-256 checked against a
  guest inventory taken after collection. Only the explicitly scoped run
  directories were deleted on the guest. The system disk retains about
  30.08 GiB free; the reviewed probe, schedule/config inputs, runner and
  rename-test executable remain; Windows, DriverStore and rollback packages
  were untouched.
- Commit `fcf2124` replaced the runner's fast-exit PID/handle race with the
  original `Process.Start()` handle and passed the Windows PowerShell 5.1
  regressions. The QPC collection receipt retained the actual exit code `1`.
  Independent review then found that a redirected-output drain timeout could
  still lead to hashing unstable logs. Commit `59e781b` closes that defect:
  unproven output is explicitly unavailable and no manifest is published.
  A real inherited-pipe regression fails against the original runner and
  passes with the repair; all nine PS5.1 behavior groups and independent
  spec/quality review passed.
- Commit `d64250f` adds example-only opt-in `RUST_LOG` stderr logging, with no
  render-callback logging or audio-default changes. All 126 example tests,
  strict example Clippy, focused formatting, two release builds and five
  real filter/routing invocations passed; independent spec/quality review
  passed. Previously, this example installed no logger, so setting `RUST_LOG`
  alone could not enable provider debug output.
- The matching 15-second-ceiling collection with provider debug enabled
  returned actual exit code `1` after 5.214 seconds. Its 49 startup samples
  retain a 350.8985 ms native wake interval, one boundary xrun and 33 soft
  events; final provider drop/silence/recovery/epoch-reset counters remain
  zero. Across 0.3061002 seconds of positive-queue control observations,
  Engine frame progress implies approximately 53,316 frames/s while completed
  VAD packets imply approximately 48,612 frames/s. These unsynchronized
  control observations suggest a consumption/production mismatch; they are
  not physical latency or exact packet-event timestamps.
- A separate muted two-second `routing_probe` control on the same output,
  with no VAD or capture, also consumed faster than nominal 48 kHz:
  224 retained callbacks span 2.0054339 seconds; the 223 submissions after
  the first total 107,040 frames, approximately 53,375 frames/s. Median wake
  interval is 8.9781 ms for the nominal 480-frame period. It reports healthy
  routing, closed host and clean shutdown. Its two software tones and
  zero-gain guards make it an output-cadence diagnostic, not the reference
  qualification graph. The mismatch therefore reproduces without VAD, but
  its native-device/format/QPC cause remains unknown.
- The new raw payload archives are
  `target/vad-target-archive/20261027-collection-d64250f-providertrace-15s/`
  (11 payloads, 1,552,998 bytes) and
  `target/vad-target-archive/20261027-collection-d64250f-render-control-2s/`
  (four payloads, 67,674 bytes). Each length and SHA-256 matched the guest
  inventory and was rechecked there before deleting its run directory.
  The previous probe is archived on the host; the temporary routing probe
  and all guest helpers are removed. Final inventory has five retained files,
  zero test processes/dynamic VAD children and 30.081 GiB free.
- Mark each target checkpoint archived and hash-verified before running the
  next case. The host now retains exact scripts and per-run inventories;
  transfer only the required executable and runner. Keep the rename probe and
  current input plans on the 64 GB guest disk and remove command helpers after
  execution.

### Ordered Work and Atomic Checkpoints

1. **Reproduce and classify the first failure.** Keep the current VM package
   and control session unchanged. The runner now preserves the actual child
   exit code before manifest serialization, and bounded startup samples now
   contain explicitly labeled control-observation QPC windows. The runner
   stability repair, example logger and their independent reviews are
   complete. Before another VAD repeat, investigate native render demand
   versus QPC/device-clock progression: the no-VAD output control already
   reproduces the faster-than-nominal consumption. The QPC windows bracket
   reads, not realtime events. Keep the capture continuity investigation and
   its zero-quality-failure gate separate. The capture-only result means position
   continuity is a separate VM capture qualification failure, not an effect
   attributable only to VAD playback. Retain example-side
   `RUST_LOG=info,moiren::audio::vad=debug` and align retained provider records,
   render wake QPC/demand/padding and Engine timeline observations. This
   physical-master graph does not run the independent virtual owner, so
   `virtual_clock_gap` events are not expected and cannot establish its cause.
   Run the short microphone event case with the existing WPP clock-rebase
   trace enabled; capture the first drop's neighboring WRITE records, driver
   reason/deadline/lateness, and before/after gate epoch
   on a common QPC timeline. Repeat the same case enough times to distinguish a
   repeatable deadline defect from a one-off host scheduling stall. Run a
   default-period control case with the same binary, endpoint, and instrumentation.
   Do not infer epoch cause from a later QUERY alone.
2. **Fix only the demonstrated cause.** If the trace shows driver packet-clock
   rebase or epoch invalidation is incorrect, add a focused regression in the
   driver host tests and fix that state transition. If the driver behaves as
   designed and the client missed freshness deadlines, keep the driver
   unchanged and correct only the proven client pacing/scheduling fault. Do not
   enlarge queues, weaken one-period microphone freshness, ignore epoch changes,
   or turn an epoch mismatch into a retry using refreshed admission state.
3. **Commit and qualify each driver/probe slice.** For each responsibility-sized
   code change, first run its focused host tests, `driver/vad/test.ps1`, and
   `driver/vad/build.ps1 -Analyze` when driver code changes. Commit and push
   before copying a matching, hash-verified binary/package to the VM. Record
   the deployed source commit and artifact hashes in the validation ledger.
   Keep the prior install/rollback package until the new target smoke is
   classified.
4. **Re-run period negotiation as a separate experiment.** After the short
   zero-drop gate passes, compare default and queried-minimum requests in
   separate event-mode playback and microphone cases. Record requested period,
   `GetBufferSize`, driver packet frames, actual completion cadence, packet
   gaps, late submissions, drops, silence, clock-rebase reason, and all epoch
   transitions. Add timer mode only as a separately labeled matrix. A smaller
   request is not a pass unless cadence and zero-failure quality are observed.
5. **Verify live application paths, then compare them fairly.** Reconfirm VAD
   MMDevice -> native PCM worker -> `VirtualPlayback` -> Engine -> Bus -> physical
   render. Then compare VAD and Process Loopback using the same source marker,
   physical output endpoint, `Input -> Bus -> Output` graph, render policy,
   controlled load, measurement inputs, and calibration. Disable the source's
   direct physical path so a doubled signal cannot contaminate correlation.
   Report structural/software timing separately from physical latency.
6. **Sweep application bypass budgets one variable at a time.** Starting from
   a stable baseline, sweep Process input target 1024/768/512/256/128 frames,
   VAD target 480/240/128/64 only when native packet timing admits it, render
   target 480/240/128/64, and input capacity from 8192 downward only after
   measured packet headroom and complete demand are known. Capacity is storage
   headroom, not a target queue fill. Reject any candidate with a drop, silent
   frame, underrun, discontinuity, marker gap, unexpected reset, or refusal;
   do not hide a failed candidate by raising another budget.
7. **Qualify duration and promote conservatively.** Only candidates passing a
   2-5 minute smoke enter a 30-minute controlled-load PCM run. Require zero
   quality failures and calibrated physical p95 plus uncertainty `<= 10 ms`.
   Then run two independent two-hour repetitions before changing a default.
   Keep Compatible as default and any faster expert profile experimental until
   both repetitions pass. Explore p50 `<= 8 ms` without making it a substitute
   for the p95 gate.

### Storage and VM Run Hygiene

- Build all Rust probes and driver packages on the development host; transfer
  only the exact artifacts needed for the next bounded case.
- Before and after every VM run, record free bytes and enumerate
  `C:\MoirenVadTest`. Use unique run directories and cap every test duration.
- Copy run evidence to the host, verify every file length and SHA-256 against
  the guest manifest, then remove only that run directory and its superseded
  temporary probe/runner. Keep the integration probe until rename qualification
  is complete. Never remove Windows, DriverStore, current driver, or rollback
  package files as space cleanup.
- Keep raw failed output on the host. Remove duplicate staging copies and
  completed temporary scripts immediately; avoid archiving reusable binaries
  unless their hash is needed to reproduce a result.

### Commit and Push Boundaries

- Commit the causal diagnostics, any proven correction, period observations,
  application/latency harness slices, and each qualification-ledger update as
  separate reviewable units. Stage only the files for that unit.
- Run the focused verification before each commit. Push each commit as soon as
  it passes; do not batch several completed target experiments into one large
  unpushed change. Record the commit ID beside its test and target evidence.
- A failed experiment is still committed as an evidence-ledger update after
  hashes and cleanup are verified. A failure does not become a code fix unless
  the trace establishes a defect.

### Exit Criteria

- VAD and Process Loopback have comparable same-condition physical results;
  neither is declared lower-latency from topology or subjective listening.
- The 48 kHz graph is exactly `Input -> Bus -> Output` and calibrated physical
  p95 plus uncertainty is `<= 10 ms` with stable continuous output.
- Both two-hour qualification repetitions have zero unexpected drops, silent
  frames, underruns, marker gaps, discontinuities, epoch changes, or recovery
  resets. Otherwise the objective remains open and the default is unchanged.

## Task 0: Freeze the Measurement Contract

**Files:**

- Modify: `docs/validation/2026-10-26-minimal-io-latency.md`
- Modify: this plan
- Preserve: `docs/superpowers/plans/2026-10-26-minimal-io-latency.md`

**Purpose:** Record the user-facing target and prevent software estimates from
being reported as physical qualification.

- [x] Add a short `User objective` section to the validation record stating:
  `Input -> Bus -> Output`, stable output, physical p95 `<=10 ms`, exploratory
  p50 `<=8 ms`, and no quality-counter failures.
- [x] Record the current state as `software path implemented; live VAD and
  calibrated physical qualification pending`.
- [x] Record the current interpretation of the audible doubling and the
  48 kHz conversion thresholds: 10 ms = 480 frames and 80 ms = 3,840 frames.
- [x] Do not mark Stage 7 complete or change the Compatible default in this
  task.

Validation:

```powershell
git diff --check
rg -n "User objective|10 ms|3,840|Stage 7|not qualified" `
  docs/validation/2026-10-26-minimal-io-latency.md
```

## Task 1: Make the Evidence Model Source-Neutral

**Files:**

- Modify: `crates/moiren-windows-audio/src/latency/qualification.rs`
- Modify: `crates/moiren-windows-audio/examples/e2e_latency_probe/cli.rs`
- Modify: `crates/moiren-windows-audio/examples/e2e_latency_probe/live.rs`
- Modify: `crates/moiren-windows-audio/examples/e2e_latency_probe/tests.rs`
- Create if the existing module boundaries require it:
  `crates/moiren-windows-audio/examples/e2e_latency_probe/source.rs`

**Interfaces:**

- Extend `EvidenceSourceKind` with `VadPlayback` while retaining
  `ProcessLoopback`.
- Introduce a source-neutral live specification containing source kind,
  source identity, marker format, output endpoint, Engine block, input policy,
  render policy, and duration.
- Make report construction consume that specification instead of hard-coding
  `EvidenceSourceKind::ProcessLoopback`.
- Preserve one versioned report schema. Add optional VAD fields rather than
  creating a second evaluator.

- [x] Write failing tests proving a VAD report serializes a distinct source
  kind, preserves stable endpoint/MMDevice identity, and remains unqualified
  without calibration.
- [x] Write failing tests proving Process Loopback reports retain their current
  schema meaning and cannot accidentally inherit VAD identity fields.
- [x] Refactor report creation so source kind, native format, packet bound,
  effective input queue, render queue, engine block, telemetry, and quality
  counters are populated by the source adapter.
- [x] Keep artifact creation collision-safe and bounded. Generate markers and
  allocate recording buffers before native streaming starts.
- [x] Run the focused example tests and strict crate Clippy.

Validation:

```powershell
cargo test --locked -p moiren-windows-audio --example e2e_latency_probe
cargo clippy --locked -p moiren-windows-audio --example e2e_latency_probe --all-targets -- -D warnings
```

## Task 2: Add the `live-vad` Harness Through the Production Path

**Files:**

- Modify: `crates/moiren-windows-audio/examples/e2e_latency_probe/cli.rs`
- Modify or split: `crates/moiren-windows-audio/examples/e2e_latency_probe/live.rs`
- Modify: `crates/moiren-windows-audio/src/vad/provider.rs` only for missing
  control-side observations or telemetry access
- Reuse: `crates/moiren-windows-audio/src/engine_backend/preparation.rs`
- Reuse: `crates/moiren-windows-audio/src/virtual_owner/provider.rs`
- Reference: `crates/moiren-app/examples/vad_integration/`
- Reference: `crates/moiren-windows-audio/examples/vad_native/`

**Command contract:**

```text
live-vad --output ENDPOINT --artifact-dir DIR
    --stable-endpoint-id ID --name NAME [--seconds 1..600]
    [the same measurement, calibration, queue, engine, and provenance options
     as live-process-loopback]
```

- [x] Add parser and dry-run tests for `live-vad`, including invalid stable
  IDs, invalid names, missing artifact pairs, and missing calibration.
- [x] Discover the VAD control interface with `WindowsVadProvider::discover()`.
- [x] Create a playback export with an explicit stable ID and probe name. Do
  not use an imported default-device permission.
- [x] Poll until the export is `Present`, policy-prepared, and audio-ready.
  Do not treat `CREATE` success alone as endpoint readiness.
- [x] Resolve the newly created VAD playback MMDevice by identity, open a
  native WASAPI render client, and feed the preallocated marker-bearing PCM
  into that endpoint. The source application must have no simultaneous direct
  physical output.
- [x] Build the production graph as `InputSource::VirtualPlayback -> Bus ->
  OutputTarget::Physical`. Use the production `WindowsBackend`/Engine host
  lifecycle and active-revision confirmation rather than a probe-only virtual
  shortcut.
- [x] Prepare the physical render owner with the exact same output endpoint,
  shared/exclusive mode, sample rate, render target, engine block, and
  controlled-load settings used by the Process Loopback run.
- [x] Open the VAD gate only after Engine confirmation and a current native
  endpoint stamp. Run the marker schedule only while all owners report ready.
- [x] Stop in the required order: stop marker source, close gate, stop/join
  Engine and PCM owners, persist artifacts off the audio path, retire the VAD
  export, and wait for final `Removed`.
- [x] Emit `mode: live_vad`, stable endpoint ID, MMDevice identity, requested
  name, actual observed name, native format, native packet frames, and VAD
  telemetry in the report.

Required VAD quality fields:

```text
submission_attempts, submitted_packets, completed_packets,
completion_interval_observations, max_completion_interval_100ns,
driver_drops, quota_drops,
carrier_drops, stale_packets, silence_packets, silence_frames,
epoch_resets, recovery_events, effective_packet_frames
```

`completion_interval_observations` counts monotonic timestamp intervals
between completed packets; it is not a missing-packet/drop counter. Telemetry
schema 2 emits this corrected field name and accepts legacy schema-1 JSON
`completion_gaps` as a deserialization alias. Drop, silence, stale, recovery,
epoch and boundary-xrun accounting remain independent quality signals.

Validation:

```powershell
cargo check --locked -p moiren-windows-audio --example e2e_latency_probe --all-targets
cargo test --locked -p moiren-windows-audio --example e2e_latency_probe
cargo test --locked -p moiren-windows-audio --lib --no-fail-fast
```

## Task 3: Add Matching Process Loopback Telemetry

**Files:**

- Modify: `crates/moiren-windows-audio/src/process_loopback/stream.rs`
- Modify: `crates/moiren-windows-audio/src/process_loopback/bridge.rs`
- Modify: `crates/moiren-windows-audio/src/capture.rs`
- Modify: `crates/moiren-windows-audio/examples/e2e_latency_probe/live.rs`
- Modify: `crates/moiren-windows-audio/src/latency/qualification.rs`

- [ ] Record observed packet frames, callback/completion gaps, bridge fill and
  age, render padding, effective target/capacity, and all delivery-bound
  violations for Process Loopback.
- [x] Keep virtual-client timing labeled empirical or declared; never write it
  into `NativeStreamTiming.period_frames` as if it were a hardware period.
- [x] Make VAD and Process reports expose comparable fields with explicit
  `null`/`unknown` values where a source cannot provide a measurement.
- [x] Add a regression test that changes the Process Loopback target and
  proves the effective prepared bridge changes, not merely the report label.
- [x] Add a regression test that a missing packet/gap bound refuses an explicit
  low-latency target instead of silently selecting Compatible behavior.

Validation:

```powershell
cargo test --locked -p moiren-windows-audio --test process_loopback --test capture --test latency_budget --test transport_budget
cargo test --locked -p moiren-app --test monitor_cli --no-default-features
```

## Task 4: Target-Machine VAD and Rename Smoke Qualification

**Files:**

- Reuse: `driver/vad/build.ps1`, `driver/vad/test.ps1`,
  `driver/vad/prepare-target.ps1`
- Reuse: `crates/moiren-windows-audio/examples/vad_native/`
- Reuse: `crates/moiren-app/examples/vad_integration/`
- Test: `crates/moiren-app/src/service/managed/rename_tests.rs`
- Test: `crates/moiren-windows-audio/src/vad/provider/lifecycle/tests.rs`

- [x] Unlock or provide an equivalent Windows target. Run power and sleep
  preflight; record Windows build, driver package hash, and binary hashes.
- [x] Build and install the analyzed/test-signed package without changing
  unrelated device defaults.
- [x] Confirm the control interface and all four expected raw child/MMDevice
  identities. Record whether each endpoint is `Present`, policy-prepared, and
  audio-ready.
- [ ] Run playback event-mode and timer-mode PCM smoke cases. Record actual
  packet frames, packet gaps, driver drops, silence, stale requests, and epoch
  changes. Run microphone cases separately; microphone freshness failures must
  not be hidden by the playback result.
- [ ] Hold a native client, close the Engine gate, request a rename, and verify:
  safe native revocation, a direct fresh old `Removed` observation, a fresh
  replacement with the exact requested Windows MMDevice name, preserved stable
  ID and graph binding, and gate reopen only after Engine confirmation. A held
  COM reference does not guarantee the old PDO remains `Removing`; retain the
  original strict-probe failure separately rather than changing the driver to
  manufacture that historical expectation.
- [x] Repeat the held-client case with remove failure and create failure. The
  requested name must remain visible as pending; the app must not claim native
  presence or active audio when the recreation failed.
- [x] Preserve the historical target observations as historical evidence; do
  not substitute them for this branch's current live qualification.

Validation commands on the development host:

```powershell
pwsh -NoProfile -File driver/vad/test.ps1
pwsh -NoProfile -File driver/vad/build.ps1 -Analyze
cargo test --locked -p moiren-app --lib
cargo test --locked -p moiren-windows-audio --lib
```

## Task 5: Establish a Valid Physical Measurement Boundary

**Files:**

- Modify: `docs/validation/2026-10-26-minimal-io-latency.md`
- Reuse: `crates/moiren-windows-audio/examples/e2e_latency_probe/pcm.rs`
- Reuse: `crates/moiren-windows-audio/src/latency/marker.rs`
- Reuse: `crates/moiren-windows-audio/src/latency/qualification.rs`

- [ ] Choose either a calibrated electrical loopback cable or an external
  dual-channel reference recorder. Record device model, channel mapping,
  sample rate, clock relation, calibration date, uncertainty, and artifact
  hash.
- [ ] Drive the same marker schedule into Process Loopback and VAD. Use the
  same physical render endpoint and measurement input for both.
- [ ] Disable the original application's direct physical output. For VAD,
  route the source application only to the created VAD playback endpoint; for
  Process Loopback, use an isolated source process or a direct-output-disabled
  source. A simultaneous direct path invalidates latency correlation.
- [x] Keep internal and WASAPI loopback runs as diagnostics, but mark them
  `unqualified` for physical latency.
- [x] Analyze p50, p95, p99, max, marker confidence, missing markers, and
  uncertainty. Convert frame offsets using the explicit 48 kHz schedule.
- [x] Treat a measured delay above 3,840 frames as evidence for `>80 ms`; do
  not use subjective doubling as that claim.

Acceptance for the measurement setup:

```text
calibration present and hash-matched
physical-cable or external-dual-channel boundary
marker source and physical output identity match the manifest
all marker windows analyzable with no ambiguity
```

## Task 6: Run the Same-Condition Latency Sweep

**Files:**

- Modify: `docs/validation/2026-10-26-minimal-io-latency.md`
- Reuse: `scripts/latency-qualification.ps1`
- Reuse: `crates/moiren-windows-audio/examples/e2e_latency_probe`

Run the following in order. A candidate that fails a short smoke run does not
enter the 30-minute run.

| Dimension | Candidates |
| --- | --- |
| Engine block | 256, 128, 64 frames |
| Process input target | 1,024, 768, 512, 256, 128 frames |
| VAD input target | 480, 240, 128, 64 frames, only when the observed native packet/gap bound admits it |
| Render target | 480, 240, 128, 64 frames |
| Input capacity | Start at 8,192; reduce only after packet headroom and full demand are proven |

- [ ] First run Compatible Process and Compatible VAD to obtain packet,
  callback, queue, and physical baselines.
- [ ] For each source, vary one complete configuration at a time. Keep source,
  output, graph, marker, duration, load, and measurement boundary fixed.
- [ ] Run a 2-5 minute smoke window. Reject on any drop, silence, underrun,
  discontinuity, marker failure, stale epoch, recovery reset, or admission
  refusal.
- [ ] Run a 30-minute controlled-load window for each viable candidate. Record
  CPU/load profile, p50/p95/p99/max, calibration uncertainty, queue age/fill,
  render padding, packet frames/gaps, and all quality counters.
- [ ] Select the smallest stable candidate with margin. Do not select solely by
  the lowest configured target.
- [ ] If the minimum native output period or virtual packet residency makes
  p95 `<=10 ms` infeasible, record the limiting period and test the available
  Exclusive/ASIO physical path before changing architecture.

Experimental pass gate:

```text
calibrated physical p95 + uncertainty <= 10 ms
all marker windows valid
zero drops, silent frames, underruns, discontinuities, and unexpected resets
same effective configuration recorded in manifest and report
```

## Task 7: Two-Hour Default Qualification and Product Presentation

**Files:**

- Modify: `crates/moiren-app/src/ui/window/settings/transport.rs`
- Modify: `crates/moiren-app/src/ui/window/settings.rs`
- Modify: `crates/moiren-app/src/service/transport_latency.rs`
- Modify: `crates/moiren-windows-audio/src/latency/qualification.rs`
- Modify: `docs/validation/2026-10-26-minimal-io-latency.md`

- [x] Keep Process and VAD policies independently configurable. Do not infer
  one source's packet timing from the other source.
- [x] Implement software presentation of requested policy, effective prepared policy, actual packet/period
  observations, queue fill/age, render padding, recovery/drop/silence counts,
  and the refusal/fallback reason. Show physical end-to-end latency as
  `unknown` until calibrated evidence exists. Boundary, provider, bridge,
  native integration and UI slices through `7e235a9` passed independent
  review; complete rendered UI acceptance and native qualification remain
  separate gates.
- [x] Preserve and present existing virtual-boundary prepared capacity/target,
  live occupancy/correction, and cumulative xrun/soft-event counters without
  turning them into a physical-latency estimate (`e19b834`; seven RED/GREEN
  regressions, affected library tests and strict Clippy passed).
- [x] Preserve and present provider packet timing, drop/silence/recovery
  telemetry and endpoint-specific errors through the app service (`9b97c08`,
  `ee73c8e`; failed-QUERY measurement provenance fixed in `fe88ad3`;
  independent service-fix and UI reviews passed).
- [x] Publish live qualified pre-transfer oldest-frame age and native render
  padding with explicit unknown values and observation provenance.
  Publication (`e431f38`, `392fab3`, `ce136a2`) and final UI text projection
  (`7e235a9`) passed independent review. Eight focused live-native row tests
  passed; these do not close the complete rendered UI acceptance matrix.
- [ ] Run two independent two-hour repetitions of the proposed default under
  the same controlled-load profile. Keep failed artifacts and diagnostics.
- [ ] Promote a low-latency default only if both repetitions pass. Otherwise
  keep Compatible as the default and label the expert profile experimental.
- [ ] Add or update UI acceptance tests for unknown/unqualified, qualified,
  refused, pending rename, and native recreation failure states.

Software observations are not physical latency estimates. Age retains its
explicit QPC timestamp and pre-transfer sampling point; packet and gap
availability are distinct, and observed zero render padding is not proof of
an xrun. Full UI acceptance, continuous calibrated waveform/load evidence,
and default promotion remain open.

Default promotion gate:

```text
two independent 2-hour runs
calibrated physical p95 + uncertainty <= 10 ms in both
zero quality-counter failures in both
identical source/output/graph/config provenance
```

## Task 8: Final Repository Verification

- [ ] Audit every plan checkbox against a recorded result. A missing target
  machine or calibration remains `pending`, not `pass`.
- [x] Run the software verification commands below. Build/check and workspace
  tests passed at `2c61452` (1,602 passed, eight opt-in tests ignored).
  Formatting exposed one assertion wrap, repaired separately in `41e8d33`.
  The full gpui suite exposed a stale test driver's missing active transport
  confirmation; independently reviewed test-only repair `9b91efb` passed the
  exact reproduction and complete suite (484 passed, four opt-in tests
  ignored). Final formatting,
  strict workspace Clippy and gpui-feature Clippy passed. These Windows host
  results do not replace CI/Miri, ignored hardware tests or physical gates.

```powershell
cargo build --locked --workspace
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p moiren-app --features gpui-test
git diff --check
```

- [x] Run the RT guard, driver analyzed build, and target-specific scripts when
  the target is available.
- [x] Report separately: software tests, host driver tests, historical target
  evidence, current target live evidence, physical qualification, and default
  promotion status.
- [ ] Only after all required gates pass may the validation record state that
  the `<10 ms` goal is achieved.

## Explicit Non-Goals

- Do not add a driver `RENAME` IOCTL in this cycle. Remove/recreate is the
  correct ABI-compatible behavior and already preserves stable identity and
  graph binding.
- Do not call `MaxPacketFrames=4800` a normal 100 ms VAD latency. It is only a
  maximum packet bound.
- Do not reduce the buffer by deleting freshness, epoch, gate, or queue
  protections.
- Do not claim VAD is faster than Process Loopback until both have calibrated
  physical reports under the same conditions.
