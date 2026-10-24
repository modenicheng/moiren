# VAD lifecycle client and validation

The Rust client implements the ABI v2 control and bounded PCM client in
[`moiren_vad.h`](../driver/vad/include/moiren_vad.h), with semantics documented in
[`ABI.md`](../driver/vad/ABI.md). The source includes the ACX audio/stream
capability path, PCM conversion and Direct I/O helpers, the Windows provider,
and the Engine/application lifecycle adapter. This document records their
control-side boundaries and opt-in probe. Host validation does not prove that a
target exposes working MMDevice endpoints. Driver installation and target
qualification remain separate.

## What the foundation advertises

The current source and driver advertise raw PDO lifecycle, staged policy, ACX
audio, native streams, bounded PCM, and explicit no-resource CREATE deferral
(`Flags=63`). This is an implementation
capability, not target-machine qualification: a successful capability query
still does not prove MMDevice visibility, software volume/mute behavior, or a
working PCM loop. The production `WindowsVadProvider` supplies prepared
`VirtualBoundaryProvider` resources. The application reconciles export intent
through an asynchronous adapter and admits each driver gate only for the
Engine-confirmed active revision and matching session/endpoint/stream epochs.
An absent or incompatible optional driver reports an unavailable export; normal
physical-device and offline operations remain independent.

A raw PDO is **not** a WASAPI endpoint. A successful CREATE or QUERY is evidence
about the raw PDO lifecycle only. `PolicyPrepared=1` is the driver's report of
policy staging; the probe does not independently prove Windows policy
enforcement, MMDevice visibility, or KS/ACX streaming behavior.

The client's audio-admission check requires all advertised ABI v2 capability bits, PCM16
and Float32, both intents, mono and stereo, one stream per pin, and the 48,000 Hz
target. Partial audio advertisements are rejected as incompatible.
`ControlClient::open_gate` checks capabilities before issuing any query or
SET_GATE. Even with complete capabilities, it requires a present PDO with
prepared policy and ready audio.

## Wire and ownership rules

- Every driver message uses explicit byte-array little-endian encoding and
  decoding. There is no native Rust/C struct reinterpretation of the control
  protocol. Capabilities responses are 64 bytes and endpoint snapshots are 128 bytes; request sizes are defined by the ABI header.
- The client checks the exact buffer length, `Size`, ABI version, every reserved
  word, enum values, canonical `0`/`1` booleans, known capability/format bits,
  channel-count/intent masks, endpoint limit `1..=64`, stream limit `1`, and
  target sample rate `48000`. A channel-count mask is not a Windows speaker mask.
- Snapshot validation rejects open gates without prepared policy and ready
  audio, audio readiness without complete capabilities, and active audio on
  Removing/Removed PDOs. Request acknowledgments must have the requested token
  and state; CREATE also checks intent, channels, closed gate, and staged policy.
- `RuntimePdoToken` is a nonzero `u64`. The ABI field called `EndpointId` is a
  runtime PDO token, not a persistent endpoint identity or MMDevice identifier.
  It belongs to the owning live session and must not be persisted over client
  closure, driver/device restart, or root replacement.
- SetupAPI discovery selects only present instances of the control interface
  GUID. Detail buffers use the reported variable length, bounded allocation,
  SDK alignment and architecture-specific `cbSize`, and a bounded size-race retry.
  Wide paths remain wide, including unpaired UTF-16 code units, and are checked
  for a terminating NUL. No fixed `MAX_PATH` buffer is used.
- A connection opens one non-inheritable, exclusive `GENERIC_READ |
  GENERIC_WRITE` file object and immediately validates capabilities. There is no
  arbitrary-string path opener. The handle remains owned for the whole session.
  Drop closes it and triggers driver session cleanup; cleanup's child removal
  completion is asynchronous.
- Microphone request sequence numbers belong to that owning session, not a
  replaceable four-slot helper. Clock/carrier resets drain old requests without
  rewinding the sequence under an unchanged native gate epoch.
- When negotiated, CREATE tag 5 is a strict no-resource acknowledgment for an
  old-session PDO with the same stable ID still retiring. It echoes only the
  requested stable ID/intent/channel shape and current session; it grants no
  runtime key, reserves no quota and transfers no old ownership. The lifecycle
  adapter may retry only after this validated response or confirmed absence.
  Timeouts and malformed responses retain uncertain CREATE state.

Discovery, opening, IOCTLs, sleeping, cancellation, and Drop are **control-thread
only**. They may allocate or block and must never run in an audio callback.

## Probe commands

The parent integration exposes `pub mod vad` on all platforms so portable
protocol tests remain available. Windows discovery and client APIs are
`cfg(windows)`. The `vad_lifecycle` example requires the opt-in `vad-probe`
feature. Its Windows API dependencies include
`Win32_Devices_DeviceAndDriverInstallation`, `Win32_Foundation`,
`Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_System_Threading`, and
`Win32_Security`.

Build and inspect the help without contacting a driver:

```powershell
cargo check --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe
cargo run --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe -- --help
```

With a driver separately installed on an authorized Windows test system:

```powershell
cargo run --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe
cargo run --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe -- --exercise
cargo run --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe -- --exercise --rounds=100
cargo run --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe -- --exercise --simultaneous --rounds=100
```

The default enumerates present control interfaces and opens each in turn to
query capabilities. **Even this capability-only open creates an exclusive driver
session** and closes it afterward. It creates no children and issues no gate or
removal requests. An existing owner can cause the open to fail; the probe never
bypasses ownership.

`--exercise` creates playback mono, playback stereo, microphone mono, and
microphone stereo raw PDOs sequentially. Each case performs CREATE, QUERY,
SET_GATE(false), REMOVE, and QUERY polling until Removed before starting the next
case. One hundred rounds performs 400 sequential lifecycles per discovered root,
not 400 concurrent PDOs. Tokens observed within a session must not be reused.
No interfaces is a diagnostic result in the default mode and an error in
exercise mode. Invalid, repeated, or unsupported arguments fail before discovery;
`--rounds` requires `--exercise` and accepts `1..=10000`.

`--simultaneous` also requires `--exercise`. It creates all four direction/shape
combinations together, then removes and recreates each stable identity while
QUERY-validating that the three siblings retain their keys, gate/stream epochs,
closed gates, ready audio and staged policy. It waits for matching Removed
before recreating, rejects reused runtime tokens, and finally removes the four
remaining resources. Each round therefore completes eight raw PDO lifecycles.
This is a concurrent raw-resource check, not proof of distinct KS/MMDevice
endpoints, native-client isolation or independent PCM routing. Those target
checks remain separate. Fake-client regressions cover the traversal, lost or
changed siblings, reused tokens, pending removal and unready creation.
The updated example suite passes 11 host tests; feature-enabled strict Clippy,
workspace all-target check and formatting also pass. This follow-up's logs are
in `%TEMP%/moiren-vad-instance-integration-20261020`. Only `--help` was executed
outside the test harness; no installed-driver exercise has run.

The lifecycle probe never installs or removes a driver package, changes system defaults,
opens an audio stream, sends gate-open, or equates a raw PDO with a usable audio
endpoint. An error closes the owning session, allowing driver cleanup to remove
children still owned by it.

### Opt-in native PCM probe

The separate `vad_native` example adds an installed-driver validation entry
point. Its default and help paths only print usage, without discovering or
opening a control interface. Ordinary example tests exercise argument parsing,
identity matching, format descriptors and PCM evidence checks without COM
activation, native streams or a driver session.

```powershell
cargo test --locked -p moiren-windows-audio --example vad_native --features vad-probe
cargo run --locked -p moiren-windows-audio --example vad_native --features vad-probe -- --help
# Run only on the prepared target with the test package separately installed:
cargo run --locked -p moiren-windows-audio --example vad_native --features vad-probe -- --exercise --seconds=2
```

`--exercise` requires exactly one present Moiren control root and holds its
exclusive owning file throughout. It creates all four direction/shape
combinations with default eligibility disabled, then waits for four distinct
active MMDevice identities. Friendly names never authorize a stream:

1. SetupAPI resolves the discovered control interface's actual PnP root.
2. Each MMDevice's `IDeviceTopology` connector supplies an opaque adapter ID,
   passed to `IMMDeviceEnumerator::GetDevice` without parsing.
3. The adapter's `PKEY_Device_InstanceId` and Configuration Manager parent must
   identify this root's `MOIREN\VAD_RAW` child with the complete expected stable
   ID. Missing or ambiguous identity is a failed validation.
4. The opened MMDevice's topology is checked again before native initialization.

The probe follows the Microsoft
[device-topology traversal](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-topologies)
and [connected-device ID API](https://learn.microsoft.com/en-us/windows/win32/api/devicetopology/nf-devicetopology-iconnector-getdeviceidconnectedto).
Only the product's own PnP instance suffix is matched; opaque MMDevice and
topology IDs are not parsed. These checks still need target execution to
establish the actual ACX topology/property projection.

For each endpoint, PCM16 and Float32 are requested as exact exclusive 48 kHz
formats. No format substitution, SRC or shared-mode mixer is used. The eight
cases send different bounded constant patterns by case and stereo channel;
mono's second internal slot stays zero. Playback is observed through driver
READs; microphone WRITEs are observed through native WASAPI capture. Initial
silence does not count as successful PCM. Each case requires at least half its
nominal frame count to match both active channels and rejects other nonzero
audio, nonfinite samples and malformed frames. Render packet sequences must
increase, and their QPC stamps must not regress, before frames can count as
evidence; gaps from bounded drops are allowed. The documented exclusive-capture
`AUDCLNT_E_BUFFER_ERROR` means an empty processing pass and does not satisfy
the evidence requirement. Microphone capture also observes
silence before admission and after gate revocation on the same RUN stream,
after draining two native buffers of previously delivered audio.

Playback uses at most four pending READs; microphone uses one pending WRITE.
Gate closure precedes the Direct I/O cancellation drain and native Stop.
Both cleanup operations are attempted even when the signal phase or gate
revocation fails; neither failure can produce a successful case.
Matching final Removed must be observed before reporting successful cleanup.
Errors release the owning session through the existing client/PCM RAII paths;
an unresponsive kernel may still block their final drain, as described below.
The probe reads system defaults before/after and rejects any of its endpoints
observed as default. It never installs a package, selects defaults, or writes
volume/mute settings.

Successful execution emits JSON with endpoint/adapter IDs, formats, both buffer
sizes, stream generations, matching/silent frame counts, microphone drops,
native discontinuities and default-role observations. `--seconds=1..=60`
controls each case's signal interval; it is a bounded correctness smoke test.
Default arrays are ordered render Console/Multimedia/Communications, then
capture Console/Multimedia/Communications.
The control-side 1 ms polling, startup-silence allowance and 50% matching floor
are not latency, sustained-throughput or CPU acceptance criteria.

This tool has not been run with `--exercise`. Its host tests do not prove PCM,
MMDevice identity, policy enforcement, active-client remove/recreate, simultaneous
stream routing isolation, Windows shared software volume/mute, the Engine
provider path or P1/P5 target qualification. Those remain separate acceptance
checks; all four PDOs stay present, but only one native stream runs at a time.

The native probe's 14 host tests pass on 2026-10-20, including the initially
failing packet-replay, transient-capture and cleanup-order regressions. The
existing lifecycle probe's 11 tests and VAD library's 69 tests also pass,
including all 30 provider tests. Feature-enabled all-target strict Clippy,
workspace all-target check, formatting, default/help execution and diff
whitespace checks pass. Only the native default/help paths were executed;
there was no driver discovery or native audio exercise. Logs are under
`%TEMP%/moiren-vad-native-probe-20261020`.

## Deadlines and asynchronous removal

REMOVE success acknowledges an accepted asynchronous PnP request. Removing is
not completion. Only a validated matching Removed snapshot ends a lifecycle
successfully. The client rejects identity/intent/channel changes and regression
to Present while polling. It does not treat `STATUS_NOT_FOUND` as proof of
completion: the ABI permits reclaiming tombstones after a later CREATE, and the
probe deliberately observes completion before issuing that CREATE.

Normal IOCTL waits have a five-second control timeout. `remove_and_wait` uses
one caller-supplied positive deadline for REMOVE and subsequent QUERY waits and
25 ms polling sleeps. The example supplies 15 seconds per removal. A zero or
unrepresentable deadline fails before submission. There is no unbounded polling
loop and no audio-thread wait.

IOCTLs use overlapped I/O. On a failed completion wait, the client requests
cancellation, retains request buffers/event/OVERLAPPED if still pending, and
rejects further IOCTLs in that uncertain session. Memory is not freed while the
kernel may still reference it. Drop drains an outstanding cancelled request
before closing its file object. **A broken driver that never completes or
honors cancellation can block that final drain.** The bounded operation deadline
is not a guarantee of bounded process teardown against a hung kernel driver.
Opening the file, discovery APIs, scheduling, and driver cleanup also have their
own OS latency. Target cancellation/unload validation is required.

## Portable and Windows checks

```powershell
cargo test --locked -p moiren-windows-audio --lib vad:: --features vad-probe
cargo test --locked -p moiren-windows-audio --example vad_lifecycle --features vad-probe
cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings
```

Protocol tests are platform-independent. They cover foundation admission
refusal; truncated and oversized responses; size/version/reserved validation;
unknown enum, flag, format and mask values; endpoint/stream/sample-rate limits;
noncanonical booleans; exact request offsets and 64-bit token preservation;
incomplete audio advertisements; gate/policy/audio/state combinations; response
identity; and asynchronous removal versus completion. Detail-buffer parsing
tests cover long paths, NUL bounds, malformed sizes, and wide code-unit
preservation. Deadline tests on Windows do not need an installed driver. Example
argument tests prove that exercising the lifecycle requires an explicit option.

These tests do not execute SetupAPI discovery against a test driver, inject
kernel completions, prove driver cancellation, or establish audio correctness.
Passing host tests and WDK package/build checks is not a target lifecycle result.

### Engine/application host checkpoint on 2026-10-20

- Provider lifecycle and fake PCM tests cover idle native streams, admission
  revision/epoch fencing, asynchronous removal, uncertain creation, request
  order, cancellation and clock-reset carrier retirement. No-client and paused
  streams do not block gate readiness after Engine confirmation.
- A same-stable-ID CREATE colliding with an old-session Removing PDO now has a
  negotiated, exact 128-byte no-resource deferral response. Rust validates all
  bytes, fields and capability negotiation before releasing the attempted
  CREATE reservation; old drivers, malformed replies and uncertain IO remain
  fail-closed. Driver tests cover all stable-ID bits and ensure deferral neither
  mutates slots nor consumes quota/runtime IDs. This is host/source evidence;
  same-ID recreation during real asynchronous ACX/PnP destruction remains a
  target test.
- A Removing observation does not acknowledge REMOVE submission: the driver
  revokes admission before the ACX removal call, which can fail. Provider
  regressions cover automatic and explicit retry of that failure, preserving
  the key and error until a valid acknowledgment, then observing asynchronous
  destruction without duplicate submissions. Gate closure and PCM drain still
  precede submission, and only Removed permits recreation.
- Before granting a new owner, the driver retries retained children from a
  departed owner. The sweep attempts all children and reports its first failure;
  a failed open keeps the owner/session unchanged. Existing refusal checks
  bypass recovery, protecting a healthy owner. Accepted asynchronous removals
  permit opening without claiming final destruction or stable-ID reuse. Host
  regressions exercise these shared production transitions with injected
  statuses; real ACX/PnP failure injection remains a target requirement.
- Producer/consumer quiescence fences cover a reset between the last epoch
  check and carrier publication. Graph returns silence while awaiting reset
  acknowledgment, never waits/spins, and cannot accept the old late packet as
  fresh. Microphone uses the symmetric producer-stop / consumer-discard order.
- Native events pace ordered PCM collection. Four buffers are preallocated;
  microphone uses one pending just-in-time WRITE and rotates those slots so
  future packets are not timestamped together and expired after one period.
  Active workers wait on the oldest ticket for at most 10 ms and immediately
  continue without an additional sleep; inactive/reset polling retains its
  10 ms interval. Event wait timeouts neither collect/cancel requests nor trigger
  device recovery, and the wait rejects Windows' infinite-timeout sentinel.
- Virtual boundaries cover f32/f64 graphs, mono/stereo and 44.1/48/88.2/96 kHz
  processing with preallocated SRC, starvation silence, backpressure and
  generation resets. RT allocation guards include the reset paths.
- The Professional backend tests cover native/virtual mixing and one-owner
  capture/follower cleanup; the virtual clock tests skip missed periods rather
  than replaying queued work. QPC interruption tests use a fake clock.
- Project/preferences v2 and managed-service tests cover migration, local
  default-eligibility isolation, Stop retaining endpoints, explicit exit
  retirement, pending dispatch revocation and active-revision admission.
- Apply/Start receipt does not revoke a confirmed graph before candidate
  validation. Regressions reject an unbound output while preserving the old
  admission revision, owner and gate without a close/reopen. Valid edits still
  revoke before owner submission; replacement, Stop and exit retain revocation.
- Explicit exit does not require final PDO disappearance: after PCM drain it
  attempts known revocation/removal once, then closes the owning file. Unknown
  CREATE and client-held Removing states remain visible, not converted to
  absence. Last-file cleanup is responsible for session-wide revocation.
- These are source/host results only. Driver cancellation, real native stream
  timing, default-device policy and active-client PnP deletion remain target
  gates. No driver has been installed by this work.

The final 2026-10-20 host verification after CREATE-deferral integration passed
1,282 workspace tests (six ignored), 260 `gpui-test` application tests (one
ignored), 75 standalone RT tests, 75 VAD library tests, 11 lifecycle-probe tests
and 14 native-probe tests. Workspace all-target check, strict all-target Clippy,
formatting, diff whitespace checks, driver lifecycle/audio/ownership host suites,
and WDK `build.ps1 -Analyze` pass; the latter builds SYS/CAT, runs static/API
analysis and validates the INF. VAD host Miri passed 33 tests under Tree Borrows.
These results do not establish guest power policy, installed-driver behavior,
Windows endpoint visibility, native PCM, or target lifecycle/performance gates.

Mixed physical-clock drift tests exercise +/-200 ppm across f32/f64, mono/stereo
and the four Engine rates, without RT allocation/deallocation. This does not
admit production latency: the current ring-derived safety reference for a
4800-frame provider carrier is about 50 ms plus FIR lookahead. Negotiated native
packet/jitter timing, selected latency-profile admission, CPU and end-to-end
measurement remain open P3 gates. Do not present this reference as a measured
or low-latency guarantee. WindowsBackend rejects virtual IO combined with
Balanced/Low or explicit custom queue targets before preparing resources;
Compatible remains explicitly unqualified rather than silently weakening an
admitted low-latency request.

Final broad host checks on 2026-10-20, after the provider handshake/timing freeze:
workspace 1,271 passed / six ignored; gpui-test package 366 passed / two ignored;
standalone RT 75 passed. The workspace run includes the late-publication
handshake, single pending microphone WRITE, four-slot rotation, actual Windows
event timeout/signaling, conservative latency admission and shutdown-error
regressions. Linux-emulated Tree Borrows Miri passed 13 prepared-boundary and
30 portable VAD tests; the long deterministic drift matrix passed in the native
workspace run instead. Miri does not exercise Windows provider/kernel FFI.

Workspace fmt/check and strict all-target Clippy passed. The VAD probe's three
argument tests and strict all-target Clippy with `vad-probe` enabled passed,
as did the three Engine examples and the application offline demo. Driver
lifecycle/audio/ownership host tests, the analyzed linked WDK build, API/INF
checks and CAT generation passed. The feature-gated probe remains an explicit
action, never automatically run against a driver by ordinary tests. Full logs
and machine-readable test counts are in
`%TEMP%/moiren-vad-final-handshake-timing-20261020` on this development host;
the earlier checkpoint logs remain in `%TEMP%/moiren-vad-validation-20261020`.

The later removal-recovery checkpoint passes 1,275 workspace tests with six
ignored, 368 gpui-test tests with two ignored, 71 VAD library tests, 75 RT tests,
11 lifecycle-probe tests and 14 native-probe tests. Workspace all-target
check, strict workspace and feature-enabled package Clippy, fmt and whitespace
checks pass. Driver lifecycle/audio/ownership suites, analyzed WDK build,
API/INF checks and CAT generation pass. Logs and machine-readable counts:
`%TEMP%/moiren-vad-removal-ack-20261020`. No target exercise has run.

### Host validation recorded on 2026-10-19

- Windows x64: 21 focused VAD library tests and three example argument tests
  passed with `vad-probe` enabled.
- The example check/build and `--help` execution passed. The help path does not
  enumerate interfaces or open a driver session.
- Strict package Clippy passed for all targets with `vad-probe` enabled.
- A temporary standalone harness referencing these source files passed Windows
  x86 cross-compilation with the minimal Windows API features used by the client.
  This verifies compilation, not execution of x86 SetupAPI calls.
- The same harness ran the 17 portable protocol/detail-path tests under nightly
  Miri's Linux emulation. All passed. Windows FFI and cancellation are outside
  that portable Miri run.

No installed-driver exercise or target audio validation was performed by this
client task.

## Continuous host validation

The runtime workflow tests the lifecycle/native probe examples on Windows and
Linux with `vad-probe` enabled, in addition to the ordinary workspace suite.
The examples' test harnesses use fake clients and bounded validation helpers;
they do not run `main`, install a driver or open a native audio stream.
Feature-enabled strict workspace Clippy also checks the probe implementation.

The Miri matrix includes the portable VAD library tests under both Stacked and
Tree Borrows:

```powershell
cargo +nightly miri test --locked --no-fail-fast -p moiren-windows-audio --lib vad::
```

This filter excludes Windows FFI and the long clock simulations. Ordinary
workspace tests retain the clock simulations; installed-driver cancellation,
PnP and audio validation remain separate target requirements.

## Remaining P1 release gates

Record the Windows build, driver/package version, signing mode, target identity,
test account, command, duration, and relevant logs for each target run. Required
evidence remains:

1. Real target installation and present-interface discovery, capability exchange,
   standard-user access, exclusive ownership, and session isolation.
2. All four raw PDO combinations, repeated sequential and simultaneous lifecycle
   runs, observable asynchronous Removing-to-Removed completion, 64-PDO
   saturation/rejection, and no identifier
   reuse or stale-session access. Neither probe mode tests 64-PDO saturation.
3. Cleanup after normal exit, owner crash, pending-request cancellation, surprise
   removal, root restart, and unload, including driver-verifier/leak checks.
   Session close alone is not proof that PnP deletion completed.
4. Independent verification of staged disabled-interface policy and the absence
   of unwanted endpoint/default-device changes before any gate is opened.
5. Target validation of the advertised ACX/KS/MMDevice audio endpoints with real
   PCM16/Float32, 48 kHz mono/stereo playback and capture, the one-stream limit,
   activation gating, and correct stop/remove behavior. Two bounded App Stage2
   runs now verify one stereo Float32 event-driven virtual graph, two known
   signals and capture silence during Stop; they do not qualify all advertised
   shapes/formats or sustained behavior.
   Circuit-instance URIs now retain direction and all 128 stable identity bits;
   host regressions and the analyzed driver build cover this correction.
   Simultaneous multi-PDO OS identity and sibling routing/removal isolation
   still require target evidence, not just unique source identifiers.
6. Installed-driver Engine/application integration: verify the implemented
   provider, endpoint lifecycle adapter, active-revision gate admission and
   project/export failure handling against actual asynchronous PnP and native
   streams. The two bounded App Stage2 passes below provide partial target
   evidence; they do not close the broader lifecycle, endpoint-absence or stress
   gate.

This document records the foundation boundary and reproducible validation
procedure. It does not claim target hardware/VM validation, WASAPI functionality,
audio quality, policy enforcement, or completion of the P1 audio release gates.

## Current host integration checkpoint - 2026-10-20

The earlier checkpoint counts above remain historical results. The current
source integration includes these committed changes:

- `ef1fe47` integrates virtual IO with the Professional backend's native owner.
- `aa2f5af` integrates application export persistence, lifecycle, admission and
  recovery.
- `18655db` classifies an expired microphone WRITE completion using the Win32
  `ERROR_SEM_TIMEOUT` HRESULT instead of comparing it with raw
  `STATUS_IO_TIMEOUT`. The regression first failed against the old helper, then
  passed using the real OS `RtlNtStatusToDosError` translation. Only that expired
  microphone completion becomes a dropped packet; playback, cancellation and
  wait-timeout errors remain distinct.
- `9f80acb` adds lifecycle/native probe test harnesses, feature-enabled strict
  Clippy and portable VAD Miri coverage to CI. Its cfg changes retain portable
  slot tests and guard the ASIO example's entry point and helpers for 64-bit
  Windows, with an explicit unsupported-platform fallback.
- `709e870` fences same-stream packet-clock resets and rejects unsupported
  one-packet native allocations.

Application resource readiness is separate from PCM transport admission.
Initial Apply can prepare the Engine owner and PCM worker before transport
admission is available, while keeping the gate closed until transport readiness
and the Engine-confirmed revision/epochs permit admission. Transport recovery
drains and retires a stranded owner before preparing its replacement, retaining
the healthy PDO. An enabled but unbound export remains present with a closed
gate and no active binding; it requires no PCM worker and does not block
subsequent graph applies. Focused independent review of these application
recovery paths reported no findings. These are control-side and host-test
observations, not evidence that target PCM has flowed.

Current completed host checks:

- Full workspace: exit code zero across 61 suites, with 1,290 passed, zero
  failed and six ignored.
- Application `gpui-test`: 375 passed and two ignored; standalone native RT
  harness: 79 passed.
- Windows VAD library: 76 passed; lifecycle probe: 11 passed; native probe:
  14 passed. Probe tests use fake clients and validation helpers.
- Portable VAD Miri under Linux emulation: 33 passed under Stacked Borrows and
  33 passed under Tree Borrows. Windows FFI, including the OS translation
  regression, is outside these portable runs.
- Windows workspace all-target strict Clippy with `vad-probe` enabled: passed.
- Linux-target audio package all-target strict Clippy with `vad-probe` enabled:
  passed after the ASIO example's missing-module regression was reproduced.
- Workspace all-target check and formatting check: passed.

The completed workspace run used unchanged Rust sources at commit `9f80acb`.
The long clock suites completed as part of that run; the earlier progress
reports are not additional test totals. The complete log and machine-readable
summary are
`C:/Windows/TEMP/moiren-vad-continued-20261020/workspace-tests.log` and
`C:/Windows/TEMP/moiren-vad-continued-20261020/test-summary.json`. Other Rust
check logs are in the same directory. The application `gpui-test` aggregate is
verified by
`C:/Windows/TEMP/moiren-vad-app-integration-20261020/gpui-tests-final.log`.

Driver commit `709e870` advances the PCM epoch and publishes Paused atomically
under the shared lock before purging requests during a same-stream packet-clock
reset, preventing fresh-epoch admission in the pre-purge interval. Native
allocation rejects one-packet timer mode and supports the two-packet
event-driven path. Three regressions first reproduced missing epoch
invalidation, acceptance of one-packet allocation and fresh-epoch admission
before purge, then passed with the fixes. Fresh `driver/vad/test.ps1` and
`driver/vad/build.ps1 -Analyze` checks passed, with zero build errors/warnings
and successful INF validation and CAT generation. Build evidence is in
`driver/vad/build/build.log` and `driver/vad/build/infverif.log`. Independent
specification and code-quality reviews passed with no findings. These are
source/host/build results; timer-mode support and real WDF/PnP, cancellation and
audio qualification remain open target work.

The read-only VM audit on 2026-10-20 was a historical checkpoint: it reported
zero running VMs and no power-preflight receipt at that time. It is superseded
by the later guest power verification and bounded Stage2 runs below. It does not
close P1/P3/P5 or establish release qualification.

## App Stage2 target checkpoint - 2026-10-21

Two bounded Stage2 runs, `app-integration-stage2-readmission2` and
`app-integration-stage2-readmission3`, completed on guest `DRIVER-TEST` with
exit code 0 and no timeout. Both ran executable SHA256
`FEEDC9F47333FD5C7E5016BFD8C050CA72D96025E1A56278180D4D5F75199E3C`, built from
source commit `6c4e3c285a704fd67f49993488f89527dec2d9c1` (the Stage2
gate-correlation implementation). The guest power receipt at
`D:/tmp/moiren-vad-20261020/power-verified.json` records AC/DC automatic sleep
disabled and hibernation disabled on VMware Windows 11 build 26200.

Each run restores a persisted v2 typed VirtualPlayback -> Gain(0.5) ->
VirtualMicrophone project. Before starting probe native clients, the Engine
remains Running through a 4,004-4,005 ms hold with 14 forward `frame_end`
advances at 48 kHz and no timeline-epoch changes; progress is also observed
beyond the prior three-second threshold. The probe then verifies exact root,
stable child IDs and MMDevice topology identity and opens stereo 48 kHz Float32
exclusive event-driven render/capture streams with 480-frame buffers, no SRC and
no fallback. It sends `[0.25, -0.5]` and verifies `[0.125, -0.25]`, then sends
the distinct `[-0.375, 0.125]` and verifies `[-0.1875, 0.0625]`. Each signal
phase requires 48,000 consecutive matching frames. Observed longest runs are
48,000/48,000 frames in readmission2 and 48,000/48,096 in readmission3. Each
phase includes a one-second settle interval; PCM remains bounded evidence, not a
throughput or latency qualification.

Native stream generation advances from 0 to 1 in both endpoints/runs. Capture
gate telemetry records a close followed by reopen after 31 ms in readmission2
and 34 ms in readmission3. Playback is observed advancing generation while its
gate remains open; its bounded readmission observation is pending in the final
gate-event record, while subsequent PCM checkpoints show both gates open and
active. The probe therefore records per-endpoint observations rather than
claiming identical gate transitions. During App Stop, capture remains RUN at
request, after Stop acknowledgment and throughout silence collection; each run
observes 48,000 real silent frames. The service joins and the exclusive control
session is released. At exit, both endpoint resources are only `Removing`, not
confirmed absent; `endpoint_absence_confirmed` remains false.

A subsequent root read-only PnP snapshot found no present `MOIREN\VAD_RAW`
children after asynchronous exit removal had time to complete. This later
snapshot is separate evidence from the probe's exit checkpoint, which reported
`Removing`; it does not retroactively change either JSON report's
`endpoint_absence_confirmed=false` and is not evidence of default-device policy
or default arrival.

Exact evidence is under `D:/tmp/moiren-vad-20261020/`:

- `app-integration-stage2-readmission2.result.json`,
  `.report.json`, `.stderr.log` and `.ssh.log`.
- `app-integration-stage2-readmission3.result.json`,
  `.report.json`, `.stderr.log` and `.ssh.log`.
- `power-verified.json` records the guest sleep/hibernation preflight.
- `guest-timer-resolution.json` records 100 waits requested at 5.3333 ms:
  default flags (0) had median 16.2714 ms, P95 17.1947 ms, maximum 23.2712 ms
  and 100/100 waits over two requested periods; high-resolution flags (2) had
  median 6.2024 ms, P95 6.8804 ms, maximum 8.2355 ms and 0/100 over two periods.
  This is a bounded guest wait-timing observation, not a WASAPI timer-mode or
  end-to-end latency qualification.

These are two bounded runs only, not long stress or release qualification.
They cover neither timer-mode, shared-mode nor physical-device integration;
default-device policy/arrival was not qualified, and the probe does not change
user defaults. Eight-hour idle/audio runs, 100 lifecycle/crash rounds, Driver
Verifier, production signing, fresh-OS install, and confirmed final endpoint
absence remain open. The results do not establish broad audio quality,
repeatability or release readiness.
