# Moiren VAD driver status - 2026-10-20

The development project now links ACX audio circuits and bounded Direct I/O PCM.
**P1 target-machine qualification has not passed.** A linked driver, successful
static analysis, valid INF and generated CAT do not prove Windows endpoint
visibility, default-device policy, active-client removal or PCM operation.
The package is unsigned and has not been installed by this work.

## Implemented source

- Independent x64 Debug ACX 1.1/KMDF 1.31 package, WDK 10.0.28000.0, MEDIA root
  `ROOT\MoirenVad`. The root starts with no audio circuit. Cargo does not use WDK.
- One exclusive, non-inheritable owning control file object, standard-user ACL,
  named-open rejection and session epochs. Duplicated handles share ownership.
  Cleanup closes gates first, cancels PCM and requests asynchronous removal.
- Up to 64 dynamic raw audio PDOs, registered/removed using ACX circuit-device
  APIs. No static child-list publication. Quota includes creating/removing PDOs
  until final destruction. Shared state survives root cleanup and late children.
- Stable 128-bit user endpoint IDs determine bus-relative PDO instance IDs.
  Runtime tokens/generations remain separate. Repeated identity is refused
  while its old PDO is still alive, including active-client removal delays.
- Each circuit uses a component URI containing its direction and the complete
  stable endpoint ID. Identity survives Prepare/Release and remove/recreate,
  and does not share the earlier provisional direction-wide ComponentIDs.
  Host tests cover all 128 identity bits, direction separation and byte order.
- Child PrepareHardware stages disabled AUDIO/direction/REALTIME interfaces on
  a system worker, with matching Association and NeverSetAsDefaultEndpoint in
  EP/0. Only after success does it call AcxDeviceAddCircuit. A local allow-default
  flag changes eligibility; no code selects a system default device.
- Mono/stereo, 48 kHz PCM16/Float32, RAW/DEFAULT, one native stream per endpoint.
  Native streams can open with Engine gate closed: render discards, capture
  emits zero. No tone/file paths, demonstration mute/volume or DSP/SRC.
- Exactly two native MDL packets for the event-driven path; timer rundown
  precedes packet release. Single-packet timer-driven allocation is explicitly
  rejected until segmented position/consumption accounting is implemented.
  QPC deadlines rebase after stalls instead of replaying every missed packet.
  Packet wrap, EOS bounds and capture availability have host regression tests.
- Circuits and pins are constructed afresh in PrepareHardware after policy
  staging. ReleaseHardware clears handles deleted by AcxDeviceRemoveCircuit;
  partial unpublished construction is discarded before retry.
- ABI v2 buffered management and OUT_DIRECT render / IN_DIRECT capture PCM.
  Up to four pending requests per endpoint, each one native packet, at most
  4,800 frames and 38,400 PCM bytes. There is no extra kernel PCM ring.
  Four slots are capacity, not mandatory concurrency. Microphone WRITE can use
  one event-paced pending request with four client slots preallocated; the
  driver still rejects payload older than one native period at consumption.
  MDLs map at passive submission, never in the timer; queued cancellation is
  owned by WDF. The timer returns a fixed four-entry completion batch, releases
  all spin locks, then completes retrieved requests with PCM rundown still held.
- Session/endpoint/stream/gate epochs fence all PCM. Gate close, stream reopen,
  resume/stall and retirement reject old requests. Every full packet-clock reset
  advances the gate epoch before PCM purge, including ReleaseHardware,
  FreeRtPackets and cleanup. Paused state and the new epoch are published under
  the same lock, blocking new-epoch admission until RUN. Same-object
  Release/Prepare cannot reuse an old stamp even when polling misses the stopped
  state; epoch exhaustion closes admission. Old microphone payloads are
  discarded by QPC age. Empty capture queues emit zero; render overflow drops.
- Root rundown protects queues from late DPC/child access. Passive per-queue
  drains serialize forwarding against cancel/removal. Control purge is restarted
  after owner cleanup; a second instance cannot take over a healthy session.
- A precreated control work item claims one cancelable request from the
  sequential queue via a spin-lock-protected single-slot handoff. Cancellation
  before claim completes exactly once; successful unmark transfers ownership to
  the worker, then completion or the existing PCM queue. No new PCM queue or
  per-request work allocation is introduced. Synchronous drains execute outside
  every EvtIo callback. Cleanup drains the control work item before restart.
- Root initialization failure shuts down even partially created PCM queues
  before EvtDeviceAdd returns. Final root cleanup releases shared storage only;
  it never uses child queue handles that WDF may already have deleted.

## Verification and remaining gates

`build.ps1 -Analyze` has passed linked SYS generation, C++ analysis with zero
warnings, API validation, native INF validation and CAT generation. Later source
changes must rerun this check. `test.ps1` passes lifecycle/ABI/bounds tests plus
clock, pause/resume, rational deadlines, render wrap/EOS, capture, bounded
completion batches and single-slot cancellation/claim ownership tests. Extracted
production-helper regressions cover packet-clock reset fencing before counters
restart, stale-stamp rejection, sequence-zero admission only under a new epoch,
closed-gate reset, epoch exhaustion and rejection of one-packet allocations.
These are host tests; they do not exercise WDF, PnP, cancellation or audio APIs.

Required target checks remain: standard-user control; real MMDevice/KS/WASAPI
endpoints; pre-exposure default eligibility; software mute/volume; data in both
formats/shapes; active-client revoke/remove/recreate; power transitions; Verifier;
eight-hour idle/audio runs; 100 lifecycle/crash rounds; resource/latency reports;
installation/upgrade/rollback/uninstall; signing and Windows-version validation.
Timer-driven client compatibility remains open and is currently refused at
allocation. It needs segmented position/consumption support and separate target
qualification; passing the two-packet host checks does not establish it.

The per-endpoint component URI replaces the provisional direction-wide IDs.
A regression first reproduced equal identities for two distinct endpoints,
then passed with the complete stable-ID mapping. This fixes the source-level
identity contract, not the remaining multi-PDO target gate: create multiple
simultaneous playback and microphone children, verify distinct KS/MMDevice
endpoints and independent routing, then remove/recreate one without affecting
siblings. Host string uniqueness does not prove OS endpoint/routing isolation.
The opt-in `vad_lifecycle --exercise --simultaneous` probe holds four raw PDO
combinations and replaces each while checking sibling identities/epochs. Its
fake-client tests do not replace running it on the target or opening KS streams.

The separate opt-in `vad_native --exercise` probe now prepares four owned PDOs
and requires four distinct MMDevice identities mapped through topology adapters
and their PnP parent to the exact control root and stable endpoint IDs. It then
requests all eight exact exclusive format/direction/shape combinations, checks
PCM patterns through Direct I/O and native clients, and observes microphone
silence on the same RUN stream before admission and after gate closure. Its
bounded host checks never contact a driver. This probe has not been exercised
against an installed package and does not close the PCM, identity, policy,
shared software volume or active-client lifecycle target gates.

The first step after guest access is `prepare-target.ps1` **inside the target**:
disable automatic sleep/hibernation and verify them before deployment/testing.
On 2026-10-20, the `Windows 11 x64` VM at
`D:\VM\Windows 11 x64\Windows 11 x64.vmx` was resumed through the existing
VMware GUI after automatic suspension. Its desktop was observed. `vmrun` guest
operations still require the encrypted-VM password; there is no verified
elevated guest command channel. TCP port 50005 has been reachable, but CDB
remote attempts did not establish a usable debugging session. Neither a TCP
connection nor a desktop screenshot proves KDNET or guest command access.
The shared-folder power preflight has not produced a success receipt, so guest
sleep/hibernation settings remain unverified. Do not deploy the driver or run
long-duration qualification until the elevated target script passes inside the
guest. No host power configuration has been changed.

A later access check resumed the VM through the existing Workstation GUI, but
the guest still did not execute the power preflight. Current-credential DCOM
process creation returned Access Denied, and both CDB remote-session and
process-server connections to port 50005 failed with HRESULT `0x80010007`.
The VM subsequently suspended again (`vmrun list` returned zero running VMs).
These are access failures, not a passed power preflight or a driver failure.
