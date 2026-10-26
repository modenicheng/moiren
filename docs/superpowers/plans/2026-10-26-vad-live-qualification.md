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

## Decision Record

| Question | Current answer | Proof still required |
| --- | --- | --- |
| Can the driver rename an existing VAD endpoint in place? | No. The ABI has no `RENAME` IOCTL; the name is supplied only by `CREATE`. | None for the current product path. Add a driver rename protocol only if remove/recreate interruption becomes a demonstrated product blocker. |
| Can a user rename a VAD in Moiren? | The app implements stable-ID remove/recreate with the requested CREATE name and preserved graph binding. Its `actual_name` currently reflects the stored provider spec, not Windows property readback. | Real-service held-client rename, direct old-Removed evidence, and exact Windows MMDevice-name verification. The latest CREATE smoke still shows a generic Windows name; cause is unknown. |
| Is VAD integrated into Moiren? | Yes: the target production smoke exercised VAD MMDevice -> native PCM worker -> `VirtualPlayback` -> Engine -> Bus -> physical render. | The 10-second run had seven boundary xruns; full native, rename, and calibrated physical qualification remain open. |
| Which is faster, VAD or Process Loopback? | No defensible measured winner yet. Process Loopback is the shorter structural baseline; VAD has driver, native PCM, worker, and virtual-boundary work. | Same physical endpoint, graph, marker, render policy, load, and calibration. |
| Can application bypass buffering be reduced? | Yes, the input capacity/target/jitter/packet/gap and render target are configurable. Reducing capacity alone does not reduce normal residency. | Packet/gap telemetry and physical sweep must prove a candidate is stable. |
| Does audible doubling prove `>80 ms`? | No. Direct output plus a delayed Moiren path creates comb filtering or slapback at much shorter delays. | A marker correlation above 3,840 frames at 48 kHz, measured at the physical boundary, is required to claim `>80 ms`. |

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
completion_gaps, max_completion_gap, driver_drops, quota_drops,
carrier_drops, stale_packets, silence_packets, silence_frames,
epoch_resets, recovery_events, effective_packet_frames
```

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
  old endpoint becomes/remains removing, final `Removed` is observed, the new
  endpoint has the requested name, the stable ID and graph binding are kept,
  and the gate reopens only after Engine confirmation.
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
