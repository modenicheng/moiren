# VAD and application-loopback latency: goals and audit

Recorded 2026-10-28 (Asia/Shanghai). Source audit: `c92895e`.

This records the user's requirements, the inspected implementation, and a
proposed sequence of independently verifiable changes. It does not approve a
new transport design, change running audio settings, or claim a latency pass.
The canonical driver qualification verdict remains in
[`driver/vad/STATUS.md`](../driver/vad/STATUS.md).

## Required outcome

- Make the minimum `Input -> Bus -> Output` route as low-latency as possible,
  aiming for measured end-to-end latency below 10 ms with stable, real output.
  The graph contains no additional DSP or deliberate delay nodes.
- Do not trade continuity for a better number: count silence, short reads,
  dropped/duplicated frames, discontinuities, resets, and correlation failures.
- Users must be able to change a VAD endpoint's displayed name. Preserve the
  stable project identity, and distinguish desired names from observed names.
- Treat application bypass here as the existing Process Loopback capture path,
  not bypassing a DSP node. Compare it with VAD under the same output conditions.
- Build and prepare on the development host; install and debug the kernel
  driver only on the separate test target. The existing VMware guest is the
  functional driver target, not permission to install the driver on the host.
- Keep each implementation/test/documentation slice atomic locally. Per the
  2026-10-29 user update, batch pushes at meaningful verified integration
  checkpoints to avoid triggering CI for every small commit.
  Do not stage unrelated screenshots, JSON, or other user work.
- The VM has a roughly 64 GB system disk. Delete obsolete, owned scratch
  artifacts after verified archival; do not accumulate build trees or binaries.

## Inspected state

### Driver and naming

- ABI v2 exposes capabilities, create/query/remove, gate control, and bounded
  Direct I/O PCM. It has no in-place rename IOCTL.
- CREATE accepts a NUL-terminated UTF-16 name in 64 units of storage. The driver
  stages `FriendlyName` before circuit publication and supplies the bridge pin
  name. The app validates 1-63 UTF-16 units and offers a Rename control.
- The reconciliation resource key includes the name. Name changes therefore
  retire and recreate the endpoint rather than update an active Windows object.
  Recreation must wait for confirmed old-resource removal, and actual PDO
  destruction may still require a negotiated CREATE deferral/retry.
- Keep the current safe-rebuild approach unless target testing demonstrates a
  product need for uninterrupted in-place naming. Active-client rename and
  Windows' observed name after same-ID recreation still need explicit evidence.

Source anchors:
`driver/vad/include/moiren_vad.h`, `driver/vad/src/Control.cpp`,
`driver/vad/src/Policy.cpp`, `driver/vad/src/audio/Circuit.cpp`,
`crates/moiren-app/src/ui/window/devices/exports.rs`,
`crates/moiren-app/src/service/exports/reconcile.rs`, and
`crates/moiren-windows-audio/src/vad/provider/lifecycle.rs`.

### Integration and qualification

- The Windows app discovers `WindowsVadProvider` and attaches it to its managed
  professional backend. This is an implemented integration, not just a design.
- Recorded Stage2 VM runs verify actual 48 kHz stereo Float32 PCM through
  `VirtualPlayback -> Gain(0.5) -> VirtualMicrophone`, using exclusive event
  clients with 480-frame buffers. Both signal phases meet their consecutive
  matching-frame gate. Stop produces observed silence and releases ownership.
- The driver also has bounded event/timer format matrices and multi-child
  lifecycle evidence. These do not qualify every client or establish lossless
  sustained throughput.
- The eight-hour idle gate passed on 2026-10-22. Eight-hour music, full 16-hour
  operation, broad crash/lifecycle repetition, release signing, clean install,
  and physical end-to-end latency remain unqualified in the canonical record.
  Do not infer a result for the unfinished last music-run entry.
- VM scheduling/clock disturbances are recorded, but new failures must be
  classified from their own evidence rather than dismissed as environmental.

Evidence: [`docs/vad-lifecycle-validation.md`](vad-lifecycle-validation.md),
[`docs/vad-validation-2026-10-21.md`](vad-validation-2026-10-21.md), and
[`driver/vad/STATUS.md`](../driver/vad/STATUS.md).
These are historical recorded runs, not tests rerun by this audit.

### Where the latency budget is spent

| Boundary | Inspected behavior | Meaning |
| --- | --- | --- |
| Process Loopback ingress | `ClockBridgeConfig`: target 2048, capacity 8192 frames; process preparation retains these defaults | At 48 kHz the target corresponds to about 42.67 ms of queued input, not measured total latency |
| VAD carrier | Provider reserves at least 4800 frames; drift reference derives from ring geometry | Roughly 50 ms per boundary's safety-inventory scale plus FIR lookahead; two VAD boundaries can both contribute |
| Native VAD stream | Tested event clients use 480-frame buffers; source admits smaller packet shapes down to 48 frames | 10 ms is a tested buffer shape; 1 ms is only a source-level minimum, not negotiated/qualified operation |
| Engine | Default processing block is 256 frames | It is not an extra 5.33 ms for every IO/Bus node; pure virtual pacing and actual native demand must be observed separately |
| Physical output | Backend, actual native period, padding and hardware vary | Must be held constant for VAD/Process Loopback A/B and measured independently |

Do not add these numbers as if they were disjoint measurements. Capacity is
storage, occupancy is queued content, packet period is cadence, and an API
latency value is not the entire route's presentation delay. Subjective doubling
is a valid symptom but does not establish an 80 ms lower bound.

Source anchors: `crates/moiren-windows-audio/src/clock_bridge.rs`,
`crates/moiren-windows-audio/src/capture/wasapi/preparation.rs`,
`crates/moiren-windows-audio/src/vad/provider.rs`,
`crates/moiren-windows-audio/src/virtual_owner/boundary.rs`,
`crates/moiren-windows-audio/src/virtual_owner/clock.rs`, and
`crates/moiren-engine/src/audio_config.rs`.

## Target and storage receipt

The read-only SSH inspection reached `DRIVER-TEST` at the already-known guest
address using the existing dedicated key; no host-key verification was bypassed.

- Windows 11 build 26200; one fixed `C:` volume, 62.96 GiB total and about
  30.01 GiB free at inspection. Guest `D:` is installation media, not a second
  writable evidence disk.
- Present Moiren control bus: `ROOT\MEDIA\0000`, status OK. Installed INF:
  `oem35.inf`; reported driver version: `13.30.29.665`. The loaded SYS hash and
  exact source correspondence were not verified. Record both before new tests.
- No present Moiren dynamic children were returned by the scoped PnP query.
  No Moiren/VAD/probe/build process was returned by the process-name inspection.
  An installed, healthy bus does not mean an audio route is currently running.
- `C:\MoirenVadTest` is empty. `C:\DriverTest` is about 0.295 GiB and contains
  WDK target/debugging infrastructure; preserve it.
- The historical `D:\tmp\moiren-vad-20261020` location is absent on this guest.
  Historical paths in documentation are not proof that evidence is still here.
- An earlier cleanup in this work session removed one confirmed redundant
  134,649,192-byte OneDrive installer. No additional deletion was justified by
  the final scoped inventory; do not claim a free-space increase from it.

Ongoing storage policy for this work:

1. Never copy Cargo `target`, WDK intermediate trees, dependency caches, source
   clones, or full toolchains into the guest. Build on the host.
2. Use only a concrete owned subtree of `C:\MoirenVadTest` for new staging.
   Keep the active executable/package and one known-good rollback copy.
3. Copy closed reports/logs/crash evidence to host storage and compare SHA-256
   before deleting their guest scratch copies. Preserve sufficient provenance:
   source commit, executable/SYS hash, command, target build, and verdict.
4. Proposed run guard: at least 15 GiB free before staging; keep owned scratch
   within 2 GiB. Refuse a new run/copy rather than deleting unknown data when a
   guard fails. Bound logging and signal snippets; do not record endless PCM.
5. Check that no process, test task, or debugger uses a candidate artifact.
   Resolve and verify absolute paths within the owned subtree before any
   recursive removal. Use `Remove-Item -LiteralPath` in PowerShell end-to-end.
6. Do not manually prune DriverStore, WinSxS, installed debug tools, system
   caches, update services, or user files as part of test-artifact cleanup.

## Proposed design direction

Three possible approaches:

1. **Measurement plus independent, admitted queue targets (recommended).**
   Keep current lifecycle/clock safety, expose occupancy/age, separate reserve
   capacity from operating target, and lower each route in tested increments.
2. **Shrink fixed rings/defaults immediately.** Small patch, but not recommended:
   current targets, native packet demand and the virtual clock debt cap are
   coupled. This risks converting audible latency into counted or hidden loss.
3. **Replace the transport/clock architecture first.** Potential later fast
   path, but greater driver/lifecycle scope before the dominant delay is measured.

The recommendation is a proposal, not an approved code change. Keep timing,
queue admission and transport recovery in `moiren-windows-audio`; keep graph
topology in core and preparation/runtime contracts in engine.

## Proposed work slices and acceptance

### S0 - Preserve requirements and establish provenance

- Commit/push this requirement and audit record separately from code.
- Before each implementation slice, check worktree state; stage explicit paths,
  run relevant checks and commit locally. Batch remote updates at verified
  integration checkpoints, then verify the remote hash.
- Before deployment, record host/guest builds, loaded package/SYS hash, binary
  hash, active settings, installed-driver status and free space.

### S1 - Measure the exact routes before tuning

Use the same deterministic input and only IO/Bus in the measured graph:

- `Process Loopback -> Bus -> physical render`.
- `VAD playback -> Bus -> the same physical render`.
- `VAD playback -> Bus -> VAD microphone` as a separate software-route result.
- Physical input/output where supported, measured separately from app capture.

Preserve source application mode/rate/buffer, output device/mode/period, graph,
clock roles and load for an A/B comparison. VAD requires changing source routing;
record that difference rather than claiming an identical capture point.

Add bounded, preallocated observations at actual boundaries: packet frames and
arrival intervals, source/sample stamps where valid, FIFO occupancy and oldest
frame age, render padding, callback time, pacing phase, resets and loss counters.
Unknown timestamps remain unknown; do not use an all-zero Process Loopback
position as a hardware clock. Do not read clocks/log/allocate per sample.

Use a known changing signal/correlation with clock/drift handling, not a constant
tone or nominal frame arithmetic, to identify end-to-end delay. Validate the
analyzer on known delay, drift, silence, dropped data, and epoch changes. Report
measurement endpoints, uncertainty, sample count, p50/p95/p99/max and startup
separately. A software-loopback receipt does not certify acoustic/physical E2E.

The VM remains the driver functional and failure-recovery target. Physical
qualification needs a suitable separate hardware target, or an explicitly
documented physical-device/measurement setup in the guest. Do not install the
test driver on the development host to bypass this constraint.

### S2 - VAD operating budget, independently of capacity

- Introduce a preparation-time virtual boundary budget with independent target,
  reserve capacity, actual packet demand, Engine transfer, FIR lookahead, wake
  allowance and supported recovery burst. Publish requested and admitted values.
- Do not derive operating latency from `MAX_PACKET_FRAMES` or half the ring.
  Keep safe maximum-packet storage without forcing that inventory in steady state.
- Audit both directions: playback prime/servo, microphone occupancy/free-space
  servo, half-period write phase, and reset reprime margin.
- Reconcile the virtual clock's eight-block debt cap with a lower target. The
  current 2048-frame repayment burst cannot be silently retained under a much
  smaller budget. Reject impossible combinations or define counted recovery.
- Test native packet period and Engine block combinations before selecting
  lower settings; source-level 48-frame support is not a target PASS.
- Preserve epochs, cancellation, ownership and no-allocation/no-blocking RT rules.

### S3 - Process Loopback bridge configuration

- Expose validated input target, independent capacity and jitter/age budget
  through the process capture API and its actual application/CLI entry points.
  Verify the current professional desktop integration separately from the legacy
  working `monitor --process` path; do not claim CLI support proves desktop support.
- Preserve compatibility defaults initially. The currently configurable native
  device buffer is not the same setting as this 2048-frame capture bridge target.
- Use observed packet gaps/phase and consumer demand for admission. Do not reuse
  an unrelated physical endpoint's period as the Process Loopback period.
- Try 1024, 512 and 256-frame operating targets only when each is admitted; these
  are experiment candidates, not promised presets. Keep capacity independently
  bounded and reject unsafe custom settings with a visible reason.
- Apply structural changes by preparing/replacing an owner, not resizing a live
  SPSC queue on RT. Expose desired versus active configuration and unsupported
  budgets honestly. Do not silently call fallback settings the requested result.

### S4 - Naming lifecycle qualification

- Exercise rename while stopped, streaming, held by an external client, and
  racing Apply/Stop/removal; include repeated rename and sibling endpoints.
- Verify observed Windows name, preserved stable ID, stale-generation rejection,
  drained old transport, confirmed removal/CREATE deferral, successful new audio,
  no cross-endpoint leakage and unchanged default roles.
- Keep old audio/name observations distinct from candidate intent during a
  pending rebuild; surface failure and retry state instead of a false success.
- Add focused failing-then-passing regressions only where a defect is found.
  Do not add a kernel rename IOCTL merely because the UI calls the action Rename.

### S5 - Stable lower-latency qualification

- Run deterministic phase/packet-size/jitter/drift/recovery models first and
  preserve explicit RT allocation/deallocation assertions.
- Qualify candidates with repeated 10-minute runs, then at least two hours of
  real non-silent streaming, before attempting eight-hour music qualification.
  Separate quiet-host baseline, controlled pressure and injected failures.
- Target p95 and p99 below 10 ms for the declared minimum route on supported
  equipment, with adequate independent observations; report max and uncertainty.
  Do not call one minimum sample a stable sub-10-ms result. These are proposed
  engineering acceptance criteria, not a hard Windows scheduling guarantee.
- Require no unexplained steady-state drops, underruns, resets, silent holes or
  signal-analysis failures. Expected fault-injection loss is a separate verdict.
- If native client/device cadence prevents the target, retain the best stable
  setting and report the measured floor and owning boundary. Do not hide failure
  by increasing buffer size or changing a threshold in the same experiment.
- Run focused Rust/driver tests per slice; complete relevant fmt/check/test,
  strict Clippy and Windows/Ubuntu/Miri CI before integration. Kernel changes
  additionally require WDK analyzed build, INF checks and a new target package.

Only S0 is performed by this document. S1-S5 are proposed future work; no new
PCM measurement, buffer tuning, rename target run or stability pass is claimed.
