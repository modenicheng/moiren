# VAD implementation ledger

## Scope and baseline

Implement the approved VAD design in ordered, independently verified stages.
Keep Graph, DSP, SRC, routing, persistence and recovery policy in user space.
The driver is an optional, separately built ACX 1.1/KMDF component; normal Cargo
builds must not require WDK or install a driver.

The initial behavioral baseline was `c97730d`. The user explicitly approved
preserving the concurrent refactors through `819e8de` and continuing on
`feat/vad-foundation`. Preserve the existing `driver/moiren/` experiment and the
unrelated repository-remediation draft.

## P0: native owner semantics

Implemented:

- Native followers start after preparation and activation, without waiting for
  master content. Their existing pump emits silence and never renders Graph.
- Remove the three-second no-transfer watchdog. Zero reads/writes, empty/full
  queues, silence and wait timeouts do not authorize resource retirement.
- Keep explicit native failures, malformed transfer lengths, cancellation,
  panic containment and stop/release/drop recovery authorization.
- Keep the follower queue target for drift correction and capture content
  priming; it is not a condition for starting a native stream.

Regression coverage uses fake native streams and bounded iteration counts.
One dedicated test crosses the former wall-clock deadline for master and
follower concurrently; no owner clock remains to replace with a fake clock.
Restoring the old watchdog in an isolated source copy makes that test fail.
This is a regression check, not an eight-hour hardware qualification.

Verification:

- `cargo check --locked --workspace --all-targets`: passed.
- `cargo test --locked --workspace`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- Native-owner RT harness: 75 passed, including seven new idle/transition tests;
  focused code review approved the change.
- Miri's Linux emulation target passes the idle-to-PCM ownership transition
  using Stacked Borrows. The Windows target reports an allocator-header borrow
  violation also reproduced by the existing standalone `System` allocator
  example, without any Moiren dependency.
- Separate harness formatting has a pre-existing issue in
  `professional/asio_rt_tests.rs`; do not mix that cleanup into P0.

## Remaining stages and admission gates

| Stage | Deliverable | Gate |
| --- | --- | --- |
| P1 | Independent ACX root, bounded dynamic raw PDOs, control-session cleanup, endpoint policy before exposure | Real audio endpoints, standard-user control, default qualification, safe deletion/recreation while clients hold streams |
| P2 | Four-slot overlapped Direct I/O per direction, cancellation and generation-fenced PCM | PCM16/Float32, mono/stereo, bounds and stale-completion tests |
| P3 | Explicit virtual resources, channel-aware boundaries, one Engine owner and virtual clock | Virtual-only and mixed physical/virtual operation, no stale catch-up |
| P4 | Project/preferences v2 migration, export management and activation gates | Local default qualification isolation, activation/switch failure behavior |
| P5 | Target-machine stress, Verifier, power, installation and release qualification | Eight-hour runs, 100 lifecycle rounds, bounded resources, rollback, signing |

The P1 control foundation has no audio admission until actual ACX circuit and
stream capabilities exist. Runtime PDO tokens are not persistent
`VirtualEndpointId` values. Do not bind these tokens into projects or expose a
global `AudioBackend::Vad`.

The current foundation hands one cancelable request from its sequential WDF
queue to a precreated system work item. The worker claims it under a bounded
single-slot ownership protocol, then dispatches under the administrative wait
lock. Synchronous PCM purges never execute from an EvtIo callback. Session
cleanup and device stop revoke ownership first, then purge and drain the work
item before restarting dispatch. Per-child policy interface link allocations are
retained for cleanup, while Windows owns the lifetime of the registered
devnode-scoped interface instances. A failed policy setup therefore remains an
unpublished, disabled child and is not treated as a reversible registry delete.
Repeated failed-create/remove and non-present-devnode registry growth remain
target gates.

Later stages must not bypass P1's active-stream removal and pre-exposure policy
gates. Creating a raw PDO or building an unsigned `.sys`/`.cat` does not satisfy
those gates. Test signing, target installation and formal release qualification
are separate from source/build validation.

## User-space implementation checkpoint - 2026-10-20

The optional Windows provider and asynchronous application export adapter now
share one owning control session. Export tracking retains uncertain CREATEs and
Removing resources, requires PCM drain before removal, and fences gate opening
with the Engine-confirmed revision and the complete driver epoch stamp. Stop
closes admission while retaining enabled endpoints; exit requests retirement.
Endpoint readiness does not require an external RUN stream or nonzero PCM.

The Engine uses explicit persistent virtual identities and mono/stereo
boundaries, with f32/f64 graphs and 44.1/48/88.2/96 kHz user-space SRC around the
48 kHz driver. Physical-master projects retain their existing single native
owner, including ASIO duplex groups. Virtual-only projects use one QPC-paced
Engine owner; missed periods are skipped, reset the prepared boundary fences,
and never trigger recursive catch-up. Persistent scheduler overload still
observes cancellation without needing a future timer wait.

PCM workers retain four overlapped slots, collect playback in submission order,
check sequence/time progression, and drain/rebuild after stream/gate/clock
changes. A session-wide microphone sequence remains monotonic when only its
four-slot helper is replaced. Driver API failure remains distinct from idle,
backpressure and a dropped late packet.

Reset now uses a two-phase producer-quiesced / consumer-discarded handshake.
An epoch check followed by an unstamped carrier write is not sufficient: the
producer can publish old data after the consumer's discard. Playback waits
without blocking Graph until old IO is drained; microphone publication pauses
until its native consumer has discarded. Deterministic tests force the late
publication and final-return race in both directions.

Playback keeps up to four reads pending. Microphone preallocates and rotates
the same four slots but submits only one just-in-time WRITE: timestamping four
future native periods together would cause their tails to fail the driver's
one-period age check. Workers wait on the oldest request's event; a finite
timeout is normal scheduling, not cancellation or proof of device failure.
Active workers immediately continue after the event wait, without another
10 ms sleep. Inactive/reset polling retains its 10 ms interval. Waiting validates
the ticket but never collects/cancels IO or changes request storage; the finite
timeout cannot become Windows' infinite sentinel.

Mixed physical/VAD clocks reuse the existing occupancy servo and preallocated
FIR even at equal nominal 48 kHz. Deterministic +/-200 ppm traces cover f32/f64,
mono/stereo and all four tested rates. This is a functional drift check, not a
production latency admission: the current ring-derived safety reference can
hold roughly 50 ms for the provider's 4800-frame carrier, plus FIR lookahead.
Negotiated packet/jitter timing and selected latency-profile admission remain
an explicit P3 production-performance gate. WindowsBackend refuses virtual IO
with Balanced/Low or explicit custom queue budgets before opening resources;
only unqualified Compatible timing is currently accepted. No low-latency
target is claimed.

Project and local preference v2 migration, trusted export enable intent, local
default-eligibility isolation, and startup loading before resource preparation
are implemented and covered by host tests. UI lifecycle state distinguishes
resource presence, gate admission and Engine-confirmed bindings; none claims
that live PCM has actually flowed.

Explicit exit waits for Engine/PCM and lifecycle jobs to drain, requests known
gate revocation/removal, then releases the owning control file. It does not
wait for a client-held PDO to be finally destroyed or invent absence for an
uncertain CREATE. Normal project switching/disable still retains Unknown and
Removing until a real driver observation confirms removal. Host regressions
cover failed removal and unknown creation during exit.

Driver source review also moved completion outside all timer spin locks while
retaining rundown, rebuilds deleted circuits after Release/Prepare, and drains
partially initialized PCM queues before DeviceAdd failure returns. Extracted
ownership host tests and the analyzed WDK build pass; WDF/PnP cancellation
correctness remains a target Verifier/lifecycle gate.
The provisional direction-wide circuit ComponentIDs were replaced after the
broad host checkpoint with a component URI containing direction and the entire
128-bit stable endpoint ID. A host regression first reproduced equal identities
for distinct endpoints, then passed with the per-endpoint mapping, including
all 128 bits, direction separation and stable reconstruction. ACX supports a
component URI as the circuit-instance identifier. The analyzed WDK build passes
with `AcxCircuitInitAssignComponentUri`; no ABI change is required. Simultaneous
multi-PDO identity/routing/removal isolation is still a P1 target gate, since
host identity uniqueness alone does not prove independent Windows endpoints.

Verification checkpoint (before the final UI/drift follow-up): workspace
check/test/fmt and strict Clippy passed; 1,238 workspace tests passed and six
hardware-dependent tests were ignored. The standalone native RT harness passed
75 tests. Linux-emulated Miri passed eight virtual-boundary tests under Tree
Borrows, 29 VAD protocol/format/slot tests and five virtual-clock tests under
Tree Borrows, and 41 compiler tests under Stacked Borrows. Windows FFI is not
exercised by those Miri runs. WDK host tests, linked build/static analysis, API
and INF checks, and CAT generation were rerun successfully. Later source
changes require a fresh final verification; these results do not satisfy P1/P5
target admission or release qualification.

Final broad host verification on 2026-10-20, after the provider handshake/timing
freeze: the full workspace suite passed 1,271 tests with six hardware-dependent
tests ignored. This includes the late-publication handshake and event-wait
regressions, four-slot rotation with a single pending microphone WRITE,
conservative virtual latency admission and shutdown-error injection. Workspace
fmt/check and strict all-target Clippy passed. The gpui-test package passed
366 tests with two ignored, including export click-through and 1024x720 scroll
accessibility. Standalone native RT checks passed 75 tests. Linux-emulated
Tree Borrows Miri passed 13 boundary tests (the long deterministic drift matrix
passed natively instead) and 30 portable VAD tests. The earlier 41-test compiler
Miri run remains applicable. Windows provider/kernel FFI is outside those Miri
runs. The VAD probe's three argument tests and strict all-target Clippy with
`vad-probe` enabled passed, as did all three Engine examples and the application
offline demo. Driver lifecycle/audio/ownership host tests, analyzed linked WDK
build, API/INF checks and CAT generation passed.

The final broad-pass logs and machine-readable test counts are under
`%TEMP%/moiren-vad-final-handshake-timing-20261020`; earlier checkpoint logs remain
under `%TEMP%/moiren-vad-validation-20261020`. No target installation,
sleep-policy success, PCM loopback, eight-hour/100-round stress, Verifier or
release-signing qualification is implied by these host results.

The later circuit-instance identity correction was verified separately with
`driver/vad/test.ps1` and `driver/vad/build.ps1 -Analyze`. It changes only driver
identity preparation and its host regression tests; it does not invalidate the
Rust handshake/timing freeze or constitute installed-driver validation.

The opt-in lifecycle probe now supports `--exercise --simultaneous`: hold all
four direction/shape combinations, remove/recreate one while verifying sibling
keys and epochs, require final Removed before reuse, then remove all. Its fake
client tests cover the concurrent traversal and injected sibling/token/readiness
failures. This closes a missing test entry point, not the target MMDevice/KS
routing or active-client isolation gate.
The example suite now passes 11 host tests. Feature-enabled strict package
Clippy, workspace all-target check and fmt pass. The driver host suites and
analyzed build were rerun after the final identity-bit regression addition.
Follow-up logs are under `%TEMP%/moiren-vad-instance-integration-20261020`.

The later opt-in `vad_native --exercise` example adds a native target smoke-test
entry point: four distinct owned MMDevices must map through topology adapters
and PnP parents to this control root and complete stable IDs. Eight exact
exclusive 48 kHz PCM16/Float32, mono/stereo, playback/microphone cases then
validate case/channel-specific PCM patterns, with microphone gate-close silence
on the same RUN stream. The default/help paths and host tests never contact a
driver. No installed-driver exercise has run. This does not close P1/P5,
shared software volume, simultaneous native-stream routing isolation,
active-client removal or Engine/provider target qualification.
The probe's 14 host tests, including RED-to-GREEN replay, transient-capture
and cleanup-order cases, pass. The lifecycle probe's 11 tests and VAD library's
69 tests also pass; feature-enabled strict all-target Clippy, workspace
all-target check, fmt, default/help paths and whitespace checks pass. Logs:
`%TEMP%/moiren-vad-native-probe-20261020`. Frozen provider/native transport,
boundary and UI implementation files were not edited in this follow-up.

The managed application no longer revokes confirmed export admission merely
on receiving Apply or Start. Two regressions first observed the live revision
changing from 3 to 0 before validation rejected an unbound output. Candidate
project validation now precedes revocation; the actual owner edit still revokes
before submission, and resource replacement/Stop/exit retain their immediate
retirement revocation. Rejected candidates preserve the old owner, confirmed
revision and live gate without a close/reopen cycle. The full application
package passes 323 tests with one ignored, and the gpui-test package passes
368 with two ignored. Workspace all-target check, strict workspace all-target
Clippy, fmt and diff whitespace checks pass. Logs:
`%TEMP%/moiren-vad-admission-commit-20261020`. This is a control-policy fix,
not installed-driver PCM or target lifecycle qualification.

The later provider removal correction distinguishes revocation from an
acknowledged REMOVE submission. The driver closes admission and sets Removing
before asking ACX to remove a child, so a successful QUERY reporting Removing
cannot stand in for a successful REMOVE response. The provider retains its key
and failure, retries an unacknowledged submission only after gate closure and
PCM drain, and stops resubmitting once a valid REMOVE acknowledgment arrives.
Recreation still requires an actual Removed observation. Two fake-driver
regressions reproduced one call instead of the required retry before the fix;
both pass, along with all 71 VAD library tests and feature-enabled strict
all-target package Clippy.
This changes lifecycle control only; the frozen PCM handshake and timing code
remain unchanged.

The driver also retries departed-owner removal failures before granting a new
control session. Its bounded retained-child sweep preserves the first failure
while attempting every child and releasing every temporary reference. A
refused open never runs that sweep; an eligible open leaves owner/session
unchanged on failure and advances them only after successful submissions.
Accepted asynchronous removal does not require waiting for destruction, and
does not permit early reuse of the old stable ID. Shared production helpers
have RED-to-GREEN host coverage for both lost aggregate errors and premature
owner assignment. Driver lifecycle/audio/ownership suites and the analyzed WDK
build, API/INF checks and CAT generation pass.

The next lifecycle edge is a repeated CREATE for the same stable ID while a
previous session's PDO is still asynchronously Removing. An explicit,
negotiated tag-5 response acknowledges that this request created no child,
transferred no key and consumed no quota; the app may safely retry after this
validated response. A timeout, malformed response or old driver remains
uncertain/fail-closed. Canonical wire and transition tests cover all 128 stable
ID bits and ensure no mutation before retry; WDK lifecycle/audio/ownership host
tests and the analyzed package build pass. Actual same-ID reuse against active
Windows clients remains a target lifecycle gate.

Final broad host verification on 2026-10-20 after CREATE deferral passed 1,282
workspace tests with six ignored, 260 gpui-test application tests with one
ignored, 75 standalone RT tests, 75 VAD library tests, 11 lifecycle-probe tests
and 14 native-probe tests. Workspace all-target check, strict all-target Clippy,
fmt/diff checks, VAD Miri (33 Tree Borrows tests), driver host suites and WDK
`-Analyze` pass. This remains host/build evidence only; no driver was installed
or exercised on the Windows 11 x64 guest.

The earlier removal-recovery host checkpoint passed 1,275 workspace tests with six
ignored, 368 gpui-test package tests with two ignored, all 71 VAD library tests,
75 standalone RT tests, 11 lifecycle-probe tests and 14 native-probe tests.
At that checkpoint, workspace all-target check, strict workspace and feature-enabled package
Clippy, fmt and diff whitespace checks pass. Logs and `test-summary.json`:
`%TEMP%/moiren-vad-removal-ack-20261020`. These are host checks, not installed
ACX/PnP qualification. Immediate same-ID restart while an old client still
holds a Removing child remains part of the target lifecycle gate; the later
negotiated deferral protocol does not replace that real target test.

## Target power prerequisite

The user requires disabling automatic sleep and hibernation inside the test VM
as the first step after guest access, before driver deployment or long tests.
`driver/vad/prepare-target.ps1` guards against running on the development host,
sets AC/DC sleep timeouts to zero, disables hibernation, and verifies both.
Do not report this prerequisite complete until it has run in the guest.

On 2026-10-20, `Windows 11 x64` at
`D:\VM\Windows 11 x64\Windows 11 x64.vmx` was resumed through the existing GUI
after automatic suspension, and its desktop was observed. `vmrun` guest
operations still require the encrypted-VM password. TCP port 50005 has been
reachable, but CDB remote attempts did not establish a debugging session.
No verified elevated guest command channel or shared-folder power-preflight
success receipt is available. The guest power settings therefore remain
unverified; no host power setting has been changed. Driver deployment and
long-duration target tests remain blocked until the elevated
`prepare-target.ps1` check passes inside the guest.

The subsequent access audit resumed the VM via the existing Workstation GUI;
`vmrun` guest operations still require its encrypted-VM password. Current-token
DCOM process creation for the power preflight returned Access Denied, and CDB
remote-session/process-server attempts returned `0x80010007`. No preflight
receipt was written, and the VM later suspended again. No host power changes,
driver installation or target acceptance checks were performed.
The latest read-only audit during removal-recovery verification reports zero
running VMs in `vmrun list`; the shared folder still contains neither
`power-verified.json` nor `power-error.txt`. This is not evidence of a guest
power-policy change.

## Current host integration checkpoint - 2026-10-20

Earlier checkpoint totals in this ledger are retained as historical evidence.
The following integration and portability changes are now committed:

| Commit | Scope |
| --- | --- |
| `ef1fe47` | Professional native-owner integration with virtual IO |
| `aa2f5af` | Application export persistence, lifecycle, admission and recovery |
| `18655db` | Expired microphone completion classification in the Win32 HRESULT domain |
| `9f80acb` | Portable VAD/probe CI coverage and slot/ASIO example cfg guards |
| `709e870` | Driver packet-clock reset fencing and unsupported allocation rejection |

The microphone fix recognizes the OS-translated `ERROR_SEM_TIMEOUT` HRESULT,
rather than raw `STATUS_IO_TIMEOUT`, as a dropped late WRITE. Its regression
calls `RtlNtStatusToDosError`, first failed against the old helper, then passed.
Playback errors, cancellation and wait deadlines retain their separate handling.

Application lifecycle readiness permits initial Engine/PCM preparation before
PCM transport admission is available. The gate stays closed until the prepared
transport, Engine-confirmed revision and matching epochs admit it. Recovery
drains and retires a stranded owner before preparing replacement PCM, while
retaining a healthy PDO. Enabled but unbound exports keep their resources with
closed gates and no active bindings; they neither need a PCM worker nor block
later graph applies. Focused independent review of the application recovery
paths reported no findings.

The full workspace test run used unchanged Rust sources at commit `9f80acb`:
exit code zero across 61 suites, with 1,290 passed, zero failed and six ignored.
The application `gpui-test` result is 375 passed and two ignored; the standalone
native RT harness passed 79 tests. Windows VAD library tests passed 76,
lifecycle-probe tests passed 11 and native-probe tests passed 14. Portable VAD
Miri passes 33 tests under each of Stacked and Tree Borrows in Linux emulation;
those runs do not execute Windows FFI or the OS translation regression.
Windows workspace all-target strict Clippy with `vad-probe` enabled,
Linux-target audio package all-target strict Clippy with that feature, workspace
all-target check and fmt check pass. The Linux ASIO example failure was
reproduced before its guards were corrected. CI now retains the probe harnesses
and both portable Miri models.

The existing long clock suites completed within the full workspace run.
Earlier progress reports are not separate totals, and historical checkpoint
counts above remain unchanged. The final log and machine-readable summary are
`C:/Windows/TEMP/moiren-vad-continued-20261020/workspace-tests.log` and
`C:/Windows/TEMP/moiren-vad-continued-20261020/test-summary.json`; other Rust
evidence is in the same directory. The application `gpui-test` aggregate is
verified by
`C:/Windows/TEMP/moiren-vad-app-integration-20261020/gpui-tests-final.log`.

Driver commit `709e870` publishes a new PCM epoch and Paused state atomically
under the shared lock before same-stream packet-clock reset purges. It rejects
unsupported one-packet timer allocation while retaining the two-packet
event-driven path. Three RED-to-GREEN regressions cover absent epoch
invalidation, one-packet admission and fresh-epoch admission before purge. Fresh
lifecycle/audio/ownership host tests and WDK `build.ps1 -Analyze` checks passed
with zero build errors/warnings, INF validation and CAT generation. The build
logs are `driver/vad/build/build.log` and `driver/vad/build/infverif.log`.
Independent specification and code-quality reviews passed with no findings.
This records source/host/build evidence only; timer-mode implementation and
target behavior remain open.

The latest read-only VM audit reports zero running VMs and a preflight
directory containing only scripts, without a success receipt. Guest power
policy remains unverified. No driver installation or target exercise has
occurred, and no P1/P3/P5 deployment, audio, performance or release gate is
closed by this checkpoint.
