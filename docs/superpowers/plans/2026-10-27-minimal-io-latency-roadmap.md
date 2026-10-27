# Minimal I/O Latency Roadmap

> **For agentic workers:** Execute one numbered stage at a time. Use
> `superpowers:executing-plans` or `superpowers:subagent-driven-development`;
> each accepted code/evidence slice ends with focused verification, an atomic
> commit and an immediate push.

**Goal:** For a 48 kHz `Input -> Bus -> Output` graph with no other processing
nodes, find and qualify the smallest stable configuration with calibrated
physical end-to-end p95 at or below 10 ms and continuous output.

**Architecture:** Keep VAD driver transport and Process Loopback as separate
input sources feeding the same Engine graph, physical render endpoint and
measurement boundary. Treat requested queue targets, prepared capacities,
native periods, observed packet delivery, acoustic/electrical latency and
calibration uncertainty as different evidence. Diagnose quality failures
before reducing buffers or comparing paths.

**Tech Stack:** Rust 2024, Moiren Engine, WASAPI, ACX/KMDF VAD driver, existing
bounded PCM evidence tap/sink, latency probe and qualification evaluator,
PowerShell 5.1 target runner, calibrated physical loopback or dual-channel
reference.

## Global Constraints

- Reference graph is exactly `Input -> Bus -> Output`, 48 kHz, with no other
  processing nodes.
- Final latency gate is calibrated physical p95 plus stated uncertainty <= 10
  ms. p50 <= 8 ms is an exploration goal, not a substitute gate.
- A passing run has zero unexpected drops, silent steady-state frames,
  underruns, marker gaps, discontinuities or epoch/recovery resets.
- Internal timestamps, queue fill, WASAPI loopback and VM measurements are
  diagnostic only; they cannot qualify physical end-to-end latency.
- Capacity is spare storage; target is desired normal queue residency. A
  smaller capacity alone does not reduce latency. Record requested and
  effective settings separately.
- Preserve epoch fencing, gate admission, microphone freshness, stable
  endpoint identity and candidate-versus-active revision rules. Do not weaken
  quality gates to make a smaller target pass.
- The VAD driver has CREATE/QUERY/REMOVE but no in-place rename operation.
  Rename remains an app-level remove/recreate and is not accepted until both
  directions read back the exact requested Windows endpoint FriendlyName.
- Build probes on the host. On the 64 GB `DRIVER-TEST` VM, keep only files
  needed by the active driver test; archive and hash-check each run on the host
  before deleting its guest run files and superseded executable. Never remove
  Windows, DriverStore, installed driver, or rollback files as test cleanup.
- Stage exact paths only. Preserve unrelated worktree edits. Commit each
  responsibility-sized change atomically and push before starting the next.

---

## Confirmed Baseline (2026-10-27)

- VAD supports a requested name on CREATE. Moiren's UI/service can retain the
  desired name and perform remove/recreate with the stable endpoint ID and
  graph binding. The latest target runs show the provider receives the new
  name, but Windows `PKEY_Device_FriendlyName` remains composed from the old
  or seed name for Playback and Microphone. User-visible rename is therefore
  not complete. Trace the registered KS bridge pin/interface and the exact
  EndpointBuilder input before changing driver properties.
- The production VAD path reaches
  `VAD MMDevice -> native PCM -> VirtualPlayback -> Engine -> Bus -> physical
  render`, but is not stable/qualified. Short tests have retained boundary
  xruns, dropped writes, silence, epoch changes and/or PCM completion errors.
- Process Loopback's input queue policy and VAD's virtual carrier target are
  configurable in Moiren. Process Loopback is the simpler structural path;
  there is not yet a same-condition measured winner. Do not claim either is
  faster from code shape or subjective listening.
- VM output-only testing independently showed device-position and submitted
  frame rates around 53 kframes/s while configured for 48 kHz. This makes the
  VM unsuitable for latency qualification and means clock/QPC behavior must be
  separated from VAD behavior.
- Audible doubling does not prove delay exceeds 80 ms. At 48 kHz, 80 ms is
  3,840 frames; determine delay by calibrated marker measurement, not by that
  listening inference. Echo audibility depends on signal and listening
  conditions, and can occur at much shorter delays.
- Latest code checkpoint `0abb8e6` separates accepted Absent lifecycle
  generations from current native stamps and historical Removed evidence.
  Verification passed: 13 service tests, 96 integration-example tests, 13 UI
  acceptance tests, strict app Clippy with the probe feature both off/on,
  formatting and diff checks. The change is pushed to
  `origin/feat/minimal-io-latency`.

## Stage 0: Reproducible Preflight and Storage Discipline

**Files:** `scripts/vad-collection-run.ps1`, target inventory/cleanup helpers,
`docs/validation/2026-10-26-minimal-io-latency.md`.

- [x] Verify `DRIVER-TEST` through its pinned SSH key/host fingerprint and
  confirm the VM is running before any driver test.
- [x] Confirm the guest run directory is empty, no probe process is running,
  `ROOT\MEDIA\0000` is `OK`, and about 30 GiB remains free.
- [ ] Keep target inventory commands scalar and bounded; record only named
  fields, never serialize full PnP/provider objects.
- [ ] Ensure every runner records the actual child exit code even if artifact
  or manifest writing later fails, and use Windows PowerShell 5.1 encodings.
- [ ] Before each run, record exact executable/config hashes, duration ceiling,
  expected endpoint identities and free space. Refuse to start if another
  audio probe is active or the run would exceed the storage budget.
- [ ] After each run, copy all required artifacts to the host, check every
  guest-manifest length and SHA-256 on both sides, preserve failure evidence,
  then remove only the verified run directory and superseded named binaries.
  Re-inventory the guest and record free space.
- [ ] Keep one current probe only when a follow-up test is queued; otherwise
  leave `C:\MoirenVadTest` empty. Do not clean unrelated VM/system files.

**Gate:** No run starts without verified provenance, bounded storage and a
scoped cleanup procedure. No evidence is deleted before host and guest hashes
match.

## Stage 1: Finish VAD Rename Diagnosis and Acceptance

**Files:** `driver/vad/src/`, `crates/moiren-app/examples/vad_integration/rename/`,
Windows endpoint-name diagnostics, `docs/validation/2026-10-26-minimal-io-latency.md`.

- [x] Keep the strict held-device contract: an accepted Absent observation may
  have a current diagnostic generation, but must not inherit a current native
  stamp or historical Removed record.
- [ ] Trace the exact registered bridge-pin/category and KS interface
  properties consumed to build the MMDevice endpoint FriendlyName for both
  Render and Capture. Retain raw IDs and typed property-read errors.
- [ ] Identify the authoritative name source before changing a property or
  INF. Do not infer endpoint naming from adapter `DeviceInterface_FriendlyName`.
- [ ] If the source is driver-controlled, add a focused host/driver regression
  proving CREATE name publication for both flows. Preserve stable ID, graph
  binding, remove/recreate, gate and Engine confirmation behavior.
- [ ] Run the real two-direction rename probe on `DRIVER-TEST`; require fresh
  native identity, direct old Removed evidence, successful replacement,
  Engine confirmation, clean owner teardown, and exact Windows FriendlyName
  equality. A provider-reported name alone is not a pass.

**Gate:** Both directions pass the complete production rename probe. Until
then, report the desired/provider name separately from the actual Windows
endpoint name and leave rename acceptance open.

## Stage 2: Diagnose VAD and Output Clock Failures

**Files:** `crates/moiren-windows-audio/src/vad/pcm.rs`,
`src/vad/provider/pcm_worker/`, `src/virtual_owner/boundary.rs`,
`crates/moiren-windows-audio/examples/e2e_latency_probe.rs`,
`driver/vad/src/` only for a correlated driver defect.

- [ ] Reproduce the earliest VAD event/timer failure with a bounded short run.
  Correlate first failed WRITE/completion QPC, packet sequence, expected
  deadline, native epoch/gate state and WPP clock-rebase records. Fix the
  runner first if it cannot preserve actual exit status or timestamps.
- [ ] Separate playback and microphone results. For each, retain first-drop
  context, native HRESULT, submitted/completed frame counts, silent/dropped
  frames, boundary fill/target/xruns, soft events, resets and orderly cleanup.
- [ ] Reproduce the VM render device-clock/QPC rate mismatch with an
  independent timebase/control. Do not attribute it to the driver without
  correlated evidence; do not use the VM for latency numbers.
- [ ] Diagnose the independent WASAPI capture position/discontinuity behavior
  separately from VAD and Engine. Position mismatches are not automatically
  missing frames; preserve packet PCM, flags, positions and QPC.
- [ ] Make only a demonstrated scheduling, lifecycle, clock conversion or
  reporting correction. Add a regression before the fix and a paired
  regression for the other delivery mode where applicable.

**Gate:** Short VAD playback and microphone tests complete with zero unexpected
drops, silence, boundary xruns, revision mismatches, discontinuities, epoch
resets or cleanup errors. VM clock behavior is characterized well enough not
to contaminate physical testing. If the gate fails, keep diagnosis at this
stage; do not increase queue targets to mask the defect.

## Stage 3: Establish Stable Short Source Paths

**Files:** `crates/moiren-windows-audio/examples/e2e_latency_probe.rs`,
source-neutral collector, `scripts/vad-collection-run.ps1`,
Process Loopback native probe and integration tests.

- [ ] Establish the exact 48 kHz source schedule, graph revision, Bus binding,
  physical output identity, render policy, CPU-load profile and evidence
  format. Use the same source frames and graph for VAD and Process Loopback.
- [ ] Run Process Loopback with an actual audio-producing source process; the
  source PID/tree and process lifetime must be retained and validated.
- [ ] Run VAD through the production native PCM/provider/virtual-boundary path.
  Wait for the recorder and source to reach their observed running states;
  broadcast stop before joining owners or draining the bounded sink.
- [ ] Validate marker schedule and physical capture continuity. A submitted
  marker is not an observed marker; internal WASAPI loopback is not the
  calibrated physical measurement boundary.
- [ ] Run bounded smoke durations first and retain every failed artifact.
  Never promote an exit code to audio qualification unless all quality gates
  pass.

**Gate:** Both paths can complete repeatable short collections with complete
source, graph, capture and cleanup evidence and zero quality failures. Keep
their latency comparison unqualified until the measurement/calibration gate.

## Stage 4: Matched VAD vs Process Loopback Comparison

**Files:** latency probe configuration/report and offline qualification tests;
`docs/validation/2026-10-26-minimal-io-latency.md`.

- [ ] Use one machine, source schedule, 48 kHz graph, Bus, physical endpoint,
  render configuration, CPU-load profile and measurement chain. Change only
  the input path/configuration under test.
- [ ] Alternate/interleave VAD and Process runs to expose time/order effects;
  retain requested/effective input targets, capacity, observed packet/gap
  distributions, output period/padding, queue fill/age and all quality counts.
- [ ] Report p50/p95/p99/max marker delay and uncertainty only when the
  calibrated common physical boundary is valid. Label internal queue/timestamp
  observations diagnostic and keep `unknown` when age is not measurable.
- [ ] Compare the same initial compatible settings first. Then test expert
  settings as separate candidates. Do not declare a winner from different
  targets, different loads or VM-only runs.

**Gate:** Publish a defensible paired result or explicitly conclude that the
paths are indistinguishable within the measurement uncertainty. No result is
valid if a quality counter fails or configuration/provenance differs.

## Stage 5: Tune Queue Targets Without Hiding Faults

**Files:** Process Loopback `ProcessLoopbackOptions`/prepared bridge policy,
VAD `VirtualBufferOptions`/provider boundary preparation, settings UI and
latency report.

- [x] Requested input capacity/target/jitter/packet/gap values are configurable
  and distinguished in app settings; Process target propagation and VAD target
  preparation have host regression coverage.
- [ ] Verify on the target that each requested setting reaches the effective
  prepared queue and is reported as such. Keep physical output period,
  `IAudioClient3` buffer, driver packet size and Engine block size separate.
- [ ] Sweep one variable at a time from a known-safe larger target to smaller
  values. Candidate examples: Process 2048/1024/512/256 frames; VAD native
  packet candidates 480/240/128/64 frames only when driver/client negotiation
  actually accepts them. These are hypotheses, not promised effective values.
- [ ] For each point, use identical workload and marker windows, repeat enough
  to observe variance, and stop reducing at the first xrun, stale/freshness
  violation, drop, silence, discontinuity or recovery reset.
- [ ] Choose the smallest stable target with explicit jitter margin. Capacity
  may be reduced only after proving it is not required for the accepted burst
  and reset bounds; do not use capacity as a proxy for normal residency.

**Gate:** Every retained candidate has verified effective settings and
zero-fault repeated short runs. Keep current Compatible defaults until
qualification succeeds.

## Stage 6: Calibrated Physical Measurement and Qualification

**Files:** `crates/moiren-windows-audio/src/latency/qualification.rs`,
`examples/e2e_latency_probe.rs`, calibration/evidence storage, qualification
scripts and validation report.

- [ ] Provide a calibrated electrical loopback or external dual-channel
  reference with a shared sample clock, identified physical input/output,
  retained calibration artifact/hash, validity period and quantified
  uncertainty. A VM virtual device is not acceptable.
- [ ] Run exactly `Input -> Bus -> Output` at 48 kHz with no other processing
  nodes. Capture expected markers and PCM continuously at the physical
  boundary. Retain original waveform, marker schedule, config hash, binary
  hash, endpoint IDs and raw quality evidence.
- [ ] Evaluate nearest-rank p50/p95/p99/max; require every planned marker to be
  accounted for and all quality counters to remain zero. Missing markers are
  failures, not omitted percentile samples.
- [ ] Run at least 30 minutes for each viable VAD/Process candidate under the
  defined controlled load. Require calibrated p95 plus uncertainty <= 10 ms.
- [ ] Run two independent two-hour repetitions of the proposed default using
  the identical qualified configuration and setup. Record separate run IDs,
  hashes, calibration and distributions.
- [ ] Promote a default only after both long runs pass. Otherwise preserve the
  Compatible default and report the failed gate without extrapolating from
  shorter runs.

**Gate:** Calibrated p95 + uncertainty <= 10 ms, p50 goal reported, full marker
accounting, zero quality failures, one accepted 30-minute candidate and two
independent accepted two-hour repetitions.

## Stage 7: Product Status and Delivery

- [x] Present requested name, provider name and actual Windows FriendlyName
  independently; never label a pending desired name as the confirmed name.
- [x] Present requested/effective queue targets and physical latency as
  separate measurements. Display `unqualified` rather than a latency number
  when calibration or required evidence is absent.
- [ ] Audit the final UI, diagnostics, project/preferences persistence and
  failure/refusal states with GPUI acceptance tests.
- [ ] Run focused tests after each slice; before final qualification run the
  relevant Windows-audio/app suites, workspace check/tests/fmt/strict Clippy,
  RT allocation guards, driver analyzed build and applicable VM smoke.
- [ ] Commit only accepted paths, push each atomic commit promptly, verify
  remote HEAD and worktree status, and retain hashes for all qualification
  evidence.

## Immediate Next Slice

1. Trace the Windows endpoint FriendlyName source consumed by EndpointBuilder;
   do not add speculative property setters.
2. Correlate the first VAD native drop with WPP clock-rebase and gate/epoch
   evidence, while independently tracking playback, microphone and render
   failures.
3. Reproduce and characterize the VM output/capture clock anomalies without
   treating them as physical latency.
4. Re-run bounded short quality gates only after the relevant cause is
   understood and corrected; archive/verify/delete guest files after every
   run.
5. Start matched path and target sweeps only after both input paths pass the
   short stability gate and a calibrated physical measurement chain exists.

## Status Ledger

- [x] Accepted Absent lifecycle generation is distinct from current native
  identity and historical Removed evidence (`0abb8e6`, pushed).
- [x] Opaque `IConnector::GetDeviceIdConnectedTo` IDs are resolved through
  `IMMDeviceEnumerator::GetDevice` for optional read-only adapter evidence.
- [x] VM reconnected by its pinned SSH host key and test key: `DRIVER-TEST`
  is online, `C:\MoirenVadTest` has zero files, no Moiren/VAD/probe process is
  running, and C: has 32,289,374,208 bytes free (about 30.07 GiB). No old VM
  test files needed removal; leave Windows, DriverStore, and driver rollback
  files untouched.
- [x] Stage 7 UI status slice: export cards and diagnostics distinguish
  requested, provider-created and exact Windows endpoint names; a Windows
  mismatch remains visible even while provider recreation is pending. Live
  native queues are labeled as effective prepared values, and physical E2E
  latency remains `Unknown / unqualified` without calibrated evidence. Focused
  unit/GPUI checks and all 13 app UI acceptance tests passed.
- [ ] Exact Windows endpoint FriendlyName rename acceptance for Playback and
  Microphone.
- [ ] Zero-quality-fault VAD playback/microphone short gates and characterized
  VM capture/output clock behavior.
- [ ] Same-condition VAD/Process comparison and effective-target sweep.
- [ ] Calibrated physical p95 <= 10 ms, 30-minute candidate and two independent
  two-hour default repetitions.

## Verification Commands

Focused host checks for a service/probe change:

```powershell
cargo test --locked -p moiren-app --features vad-integration-probe native_stamp
cargo test --locked -p moiren-app --features vad-integration-probe --example vad_integration
cargo test --locked -p moiren-app --features "gpui-test vad-integration-probe" --test ui_acceptance
cargo clippy --locked -p moiren-app --all-targets -- -D warnings
cargo clippy --locked -p moiren-app --all-targets --features vad-integration-probe -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Broader relevant Windows-audio verification before integrating latency changes:

```powershell
cargo test --locked -p moiren-windows-audio
cargo test --locked -p moiren-app
cargo test --locked -p moiren-app --features gpui-test
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
cargo clippy --locked --workspace --all-targets -- -D warnings
```
