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
  The graph contains no additional DSP or deliberate delay nodes. Per the
  2026-10-31 user amendment, if this is demonstrated impossible under the
  current device conditions, pursue the defensible theoretical minimum and
  best achievable stable setting instead, with the evidence limits below.
- Do not trade continuity for a better number: count silence, short reads,
  dropped/duplicated frames, discontinuities, resets, and correlation failures.
- Users must be able to change a VAD endpoint's displayed name. Preserve the
  stable project identity, and distinguish desired names from observed names.
- Treat application bypass here as the existing Process Loopback capture path,
  not bypassing a DSP node. Compare it with VAD under the same output conditions.
- Build and prepare on the development host; install and debug the kernel
  driver only on the separate test target. Per the 2026-11-07 user amendment,
  use the existing standalone remote test computer directly; VMware is not a
  prerequisite. This is not permission to install the driver on the host.
- Keep each implementation/test/documentation slice atomic locally. Per the
  2026-10-29 user update, batch pushes at meaningful verified integration
  checkpoints to avoid triggering CI for every small commit.
  Do not stage unrelated screenshots, JSON, or other user work.
- Delete obsolete, owned scratch artifacts after verified archival; do not
  accumulate build trees or binaries. If the optional historical VM is reused,
  its roughly 64 GB system disk remains a separate storage constraint.

### 2026-11-07 direct remote target and desktop scope

The user explicitly selects the existing separate remote test computer instead
of requiring VMware. Earlier records that require starting/unlocking the
encrypted VM describe the then-current plan and no longer block direct remote
driver work. Keep package/signature/loaded-SYS provenance, rollback readiness,
endpoint ownership and explicit handling of disruptive install/reboot steps;
do not silently alter boot/security policy or stop protected external clients.

An unlocked, usable desktop is a prerequisite only for tests that exercise
ordinary visible desktop input, not for every SSH, CLI, metadata or audio test.
Schedule noninteractive work independently when its own token/session/device
requirements are met; a locked screen is not itself evidence that such work
cannot run. Conversely, a reachable SSH/session0 token does not establish
usable input in the app's interactive session. The stopped point observation
supports exact-root input refusal in that snapshot, not a WTS lock-state
diagnosis. Do not bypass a foreign/system cover to obtain a desktop pass.

### 2026-11-03 development and test scheduling

The user permits independent development and remote long-duration tests to run
in parallel. Native audio, desktop UI, OBS and driver/device exercises remain
remote-only; do not launch host audio/UI or operate existing host processes.
Host compilation, pure/offline tests and headless UI tests are permitted.

Run a long test only for a distinct unanswered question, with a pinned binary,
isolated settings/evidence, a finite deadline and owned-process cleanup. Reuse
the result while development continues; a new build does not retroactively
change the revision under test. Serialize tests that share audio endpoints,
drivers or recording resources. Preserve external OBS/QQMusic owners; unchanged
process IDs alone do not prove their audio was uninterrupted.

First-packet investigation remains deferred. The user's approximately ten-minute
manual run is useful usability feedback, not calibrated latency or complete
waveform evidence. Missing physical capture equipment and that deferred issue
do not block independent product work. Do not repeat unchanged soaks or invent
extra qualification requirements merely to keep this task active.

### 2026-10-31 target-device fallback

Sub-10-ms remains the preferred objective, not a reason to stop useful
development or repeatedly retest an unrelated limitation. The user accepts
the minimum achievable under the current device conditions only after the
sub-10-ms limitation is evidenced; no such impossibility is established yet.

- Declare the exact device, driver, source path, native mode/rate/period,
  graph, load, measurement endpoints and uncertainty for every conclusion.
  Keep VAD/Process and internal-software/physical results separate. A result
  for one mode/device/path is not a result for every supported configuration.
- Distinguish a justified theoretical lower bound, the minimum observed stable
  configuration and the unresolved gap between them. Do not call the current
  best run the theoretical minimum or add overlapping nominal buffer/period
  quantities as if they were independently measured pipeline delays.
- Support an irreducible limiting boundary with calibrated measurement and/or
  a validated device/driver timing constraint for the declared configuration.
  Account for uncertainty and demonstrate which remaining controls can or
  cannot reduce it. One failed shallow-buffer run, API minimum period, current
  Shared default, VM jitter or incomplete calibration does not prove that
  sub-10-ms physical latency is impossible.
- Reduce removable queue/phase/processing overhead one admitted configuration
  at a time. If a lower bound is not defensible, report the best stable result
  and remaining unknowns rather than manufacturing an impossibility verdict.
- Keep real non-silent output, continuity, cleanup and sustained-load gates
  unchanged. Correctness-mode jitter grace is diagnostic only; it neither
  permits production audio latency nor hides underruns, loss or resets.
- Once the evidence-gated fallback applies, report achieved p50/p95/p99/max,
  uncertainty, the lower bound, remaining gap and owning boundary. A result
  above10ms may meet this conditional device-limited objective, but must never
  be labelled a sub-10-ms pass. Do not silently enlarge a buffer or move the
  measurement boundary to produce a better claim.

## Current pipeline status - 2026-11-07

The original S0-S5 requirements below remain the completion criteria. Subsequent
implementation is recorded in the
[software ledger](validation/2026-10-29-s1-s3-software-ledger.md) and the
[2026-11-04 pipeline checkpoint](validation/2026-11-04-pipeline-checkpoint.md).
The original source audit in the next section is historical, not current
configuration or acceptance evidence.

| Slice | Current evidence | Remaining acceptance |
| --- | --- | --- |
| S0 | Requirements, scoped commits, binary/configuration pins and bounded archival workflows exist; current-main integration/release and scoped portability repairs have local evidence. Published both-route wait repair `8761d07` completes exact-head CI `41915822264` with all 20 jobs successful. Its final-source workspace/headless run passes 2625 tests with both opt-in features, and the optimized example has a verified immutable copy. | Preserve provenance and unrelated work across subsequent native/qualification changes. |
| S1 | All four route harnesses and bounded boundary observations exist, including opt-in pure-virtual Engine callback timing. Frozen metadata rules out assigning all replacement residuals to client Stop-tail inventory, without implying DAC presentation. PCM persistence/decode refusals pass 35 regressions per platform. A new direct-remote waveform scope retains actual Process reference/return media; unchanged nominal analysis resolves all six reference codes and return codes2,3,5. | The new pair stops on forced OBS cleanup before VAD; matched accepted routes and explained full waveform correspondence remain open. Its old close-order defect has an established tested correction, not another native pass. The separate frozen metadata reader still lacks its original token/OBS guard and is not rewritten. Calibrated measurement and original residual/gap failures remain open. |
| S2 | Independent VAD target/capacity, actual-demand admission, priming, servo and bounded recovery are implemented. | Native period/block combinations and sustained quality do not yet qualify a lower default. |
| S3 | API, CLI and desktop support exact input intent, visible refusal and prepared owner replacement; target1280 activation is recorded. The November7 stopped-only read-only point snapshot identifies a foreign LockScreenBackstopFrame over Settings, without diagnostic input or audio. The separate local runner generation correction passes software checks only. | An unobstructed interactive target, successful ordinary desktop retained Stop/Evaluate/Apply and continuous native waveform acceptance remain open; the retained archive's event-order refusal is separate. Forced diagnostic-app cleanup remains a failed lifecycle, not desktop acceptance. |
| S4 | Safe recreation, stable project binding, separate requested/provider/Windows names and interruption notices exist; the software-output category candidate passes host/portable/WDK checks. The streaming harness requires old/new Start/PCM, direct Removed, exact name and fresh confirmation. Readiness waits now retain current-request errors from both route exports before checking readiness; final PCM retains the bounded gate/freshness contract. Pre-client admission permits legal generation zero while final PCM still requires a nonzero generation. Tests pass 145 on Windows and 124 on Linux stable. | Windows-visible name acceptance, migration/default roles and the required native streaming/held-client/race/repetition cases remain open. Use the standalone remote target per the November7 amendment, with package/signature/rollback and endpoint guards; VM startup is not required. |
| S5 | Deterministic/RT software checks and finite native receipts exist. Offline qualification now checks both p95 and p99 plus calibration uncertainty strictly below10ms, matching the current S5 requirement rather than the older p95-only rule. | Repeated/load/non-silent sustained runs and calibrated p95/p99 below10ms, or the evidenced device-limited fallback, remain open. |

No aggregate percentage, software test total or successful lifecycle substitutes
for these acceptance requirements. Existing delivery-gap violations, waveform
residuals and genuine PCM/epoch faults remain visible. Initial-packet work stays
deferred; missing physical capture equipment does not block independent work.

### Later direct remote deployment - 2026-11-07

The later [waveform checkpoint](validation/2026-11-07-matched-waveform-checkpoint.md)
uses clean `b08f61b` binaries directly on the standalone target. One bounded
Process acquisition retains complete source/reference/graph data and closed
OBS media, but forced OBS cleanup stops the pair before VAD. This repeats a
previously documented close-order runner defect; the established correction
is recovered and passes15pure checks without another acquisition. Canonical
v3 offline analysis passes28synthetic checks and finishes against the actual
closed media with unchanged input hashes: all six reference identities, return
identities2,3,5, no quality/latency pass. Both native safety audits and all100
compact archive member pins verify; original29 draft/cache hashes remain
unchanged. Full S0-S5 acceptance remains open.

The later [history/consumption receipt](validation/2026-11-07-endpoint-history-and-consumption.md)
uses guarded read-only queries at21:26 and21:38. Fixed raw PDOs first install
on October25, while the seeded controls first install November7; these dates
do not prove MMDevice consumption or who disabled a device. Exact retained
endpoint GetDevice succeeds for all six IDs, including fixed Playback missing
from a separate PnP inventory. Its endpoint store has no EnableEndpointByDefault
value (S_OK/VT_EMPTY), while both seeded Playback stores retain0x101 despite
identical current interface staging. All are NOTPRESENT after cleanup; the
earlier live DISABLED failure remains. This is a retained policy/name divergence,
not a migration fix, enablement, PCM pass or exact-name acceptance. Both13-file
archives and external-state/idle audits verify; original29 draft/cache pins
remain unchanged. Published0fdc2c6 completes CI42374879801 with20/20 successful
jobs and original watcher99946 exit0, closing software CI for the state slice.
The current display-name-derived KS reference also needs reconciliation with
Windows endpoint settings/identity preservation; do not rotate identities to
manufacture a fresh-policy/name pass. All original S0-S5 gates remain in force.

The [policy/state receipt](validation/2026-11-07-endpoint-state-and-policy.md)
compares18 exact persisted registrations from the fixed and earlier seeded
fixtures. Both Playback policies already contain0x101 enablement and the
same OUTPUT_UNDEFINED association. The snapshot rules out currently missing
staging, without proving policy consumption, disablement origin or migration.
Exact phantom-allowed PDO reads distinguish the fixed Models description
from requested bus text and report no FriendlyName value; no raw MMDevice
name fix or live publication timing follows. All61 terminal files are archived
and length/hash verified, including helper failures/corrections. The target
is used directly, without VM startup, desktop input/unlock or native audio Start.

Product observations now retain raw Windows endpoint state and independent
state-read errors in the service, probe, Exports and diagnostics, separately
from provider presence and Engine readiness. No ACTIVE inference, enablement
or acceptance-gate change is introduced. The pinned17-file slice passes2636
Windows workspace/headless tests with10 ignored and155 integration example
tests; Linux example133, observer17, app618 with2 ignored and acceptance7 pass.
Windows/Linux strict workspace Clippy and formatting pass. Clean released004809
then runs once directly on the target at20:52:50..20:53:14: the new product
fields observe initial NOTPRESENT followed by Microphone ACTIVE and Playback
DISABLED on the same identities, independently matched by failure enumeration.
The PCM run still fails before probe Start, with zero acquired signal frames;
join/release/cleanup and external-state/idle audits pass. Its17 terminal files
are hash/length archived. This verifies new-source state propagation, not
Windows-name, PCM or S1/S4/S5 acceptance. Published6401849's20-job CI is green;
the later0fdc2c6 source publication now also has matching20-job green CI.

The later PCM extension in8318308 passes154 Windows/132 Linux example tests
and strict Clippy without changing ACTIVE admission or PCM gates. At19:43 its
failure collector completes both flows and finds the exact fixed-fixture
Playback/provider MMDevice DISABLED, while the matching Microphone is ACTIVE.
That narrows this failure snapshot; disablement origin, earlier state,
creation-policy consumption and migration remain unproven. The mapping timeout
occurs before probe Start and yields no PCM, despite observed Engine timeline
advancement during the idle hold. Cleanup/join/release and external-state audits
pass; report acceptance remains failed. Both independent PCM terminal archives
are hash/length verified. No endpoint enablement, identity rotation, historical
device deletion or lock-state inference is performed. Next inspect exact
interface EP0 policy read-only rather than repeat the unchanged native run.
Prior published c890e50 finishes all20 CI jobs at19:45; that result does not
qualify the subsequent diagnostic commits or any native/physical gate.

The later [enumeration receipt](validation/2026-11-07-rename-enumeration-evidence.md)
adds failure-only ALL-state metadata while retaining ACTIVE admission. Windows/
Linux tests pass152/130 and strict Clippy passes. At19:29 both directions'
held/replacement initialization, direct Removed and fresh Engine confirmation
complete, but both exact Windows names retain adapter suffixes. Enumeration
does not fail, so the prior timeout's cause/fix remains unproven; Starts0/no
PCM. This bounded observation is still failed/unqualified, not S4 completion.

The [direct remote612 receipt](validation/2026-11-07-direct-remote-category.md)
records actual candidate/rollback signing verification, guarded installation
and exact-root device restart with no Windows reboot or security/default/volume
changes. The old611 package remains verified locally/remotely. The candidate
is the default non-opt-in diagnostic build, not a relabelled611 binary.
Stage1 native service Apply/Stop/join passes. Streaming refuses before Start
or rename on the existing identity; a separate explicitly seeded held-client
diagnostic initializes Playback but fails Stop with a sticky no-ready packet
stream error and nonempty public exit cleanup_error. Source investigation
identifies early first-owner preparation during legal Paused/zero-packet
allocation; the focused repair passes workspace/headless tests, the145-test
integration example suite and strict Clippy. Its clean corrected release is
tested at19:01 on the same explicit fixture: both held clients and direct old
Removed checkpoints complete; Playback replacement enumeration fails while
Microphone replacement/Engine confirmation succeeds but exact name fails on
the adapter suffix. Cleanup/join/release and external-state audits pass; this
held-only run makes no Start/PCM claim. None of these steps closes full
exact-name/streaming/held/repetition/race/default-role acceptance,
the desktop gate, sustained audio or calibrated physical p95/p99.

### Remote-priority update - 2026-11-07

The [fresh remote receipt](validation/2026-11-07-remote-priority.md) supersedes
November5's readiness assumptions, not the S0-S5 acceptance criteria. The
expected target and unchanged loaded driver are reachable. A bounded session1
observer now sees a PowerShell foreground rather than the lock screen, but the
subsequent stopped-only owned-input control still refuses an obscured or
different/child HWND before posting any click. After correcting an evidenced
cross-token process-metadata runner defect, that actual control terminates
Ready/result1 with no audio start, no remaining owned process and unchanged
defaults/render controls/protected identities. It is not desktop acceptance.
Historical OBS12648 is now absent, so the frozen metadata reader lacks that
guard as well as its exact non-admin token. The driver VM remains stopped.
No unchanged native retry, new driver installation, threshold change or
physical/full-pipeline verdict follows.

The same receipt records a local-only retained desktop worker repair: Stop
advances intent1->2, and explicit retained Apply starts owner3 rather than2.
Five expectation sites now follow the existing service contract without
changing predecessor evidence or timing gates. Both PowerShell shells pass34
pure checks each, and the software-backed Rust service regression passes1test
with its F32/F64 loop. No corrected worker is deployed or run natively; the
17:31:38 read-only remote check still finds no test owners and the driver VM
inventory remains empty. These checks do not close S3 or any physical gate.

The receipt's later one-shot observation answers the missing hit identity:
at17:42:07+08:00, Settings center1482/171 hits foreign PID13100,
HWND855132, class`LockScreenBackstopFrame`, not the owned Moiren root.
No diagnostic click/Invoke/audio occurs. Owned Close then forced cleanup
remains a failed lifecycle;13 raw artifacts are verified, protected identities
and driver/defaults/render controls are unchanged, and no owner/running test
task remains. This snapshot neither explains the earlier run retrospectively
nor establishes WTS lock state or a product/DPI defect. No unchanged retry or
bypass follows. Visible desktop input remains unqualified; the later direct
remote amendment removes VM startup as a prerequisite for independent work.

The later [wait-error repair](validation/2026-11-05-s4-wait-error-retention.md)
closes two source-supported false-acceptance paths, not native qualification.

## Inspected state - original source audit

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

Use the separate remote computer for driver functional and failure-recovery
work per the November7 user amendment; a VM is optional, not mandatory.
Physical qualification still needs a declared hardware/measurement setup and
calibration; standalone execution alone is not physical latency evidence.
Do not install the test driver on the development host to bypass this constraint.

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
  setting and apply the evidence-gated target-device fallback above. Report the
  measured floor, justified lower bound, remaining gap and owning boundary.
  Do not hide failure by increasing buffer size or changing a threshold in the
  same experiment.
- Run focused Rust/driver tests per slice; complete relevant fmt/check/test,
  strict Clippy and Windows/Ubuntu/Miri CI before integration. Kernel changes
  additionally require WDK analyzed build, INF checks and a new target package.

The initial2026-10-28audit performed S0 only. Later S1-S5 work is tracked in the
current pipeline status and linked receipts; none of those software increments
alone establishes a native waveform, rename, sustained or physical-latency pass.
