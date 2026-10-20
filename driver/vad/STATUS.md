# Moiren VAD driver status - 2026-10-21

The development project links ACX audio circuits and bounded Direct I/O PCM.
A test-signed package is now installed in the VMware Windows 11 target, and
bounded raw-child/control-lifetime checks have passed there. **P1 target-machine
qualification has not passed.** A metadata-only child MEDIA/null-service INF
has now been validated on target, and all four owned MMDevices map successfully.
Production INF child matching is implemented in `e8f769f`. The later
`production-binding` run removes the scratch child package, verifies all four
live MEDIA children bind the combined production INF, and passes eight exclusive
event-mode cases with half-period microphone pacing. The subsequent
`native-timer-production-phase` run also passes the full eight-case timer matrix.
These are bounded target results, not stable-repeatability or complete product
qualification: earlier timer runs aborted on real long-stall epoch mismatches,
and a fresh-OS installation has not been qualified.

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
  The source permits native streams with Engine gate closed: render discards,
  capture emits zero. The bounded event-mode target run now observes microphone
  silence before admission and after gate closure. No tone/file paths,
  demonstration mute/volume or DSP/SRC.
- Two native MDL packets for the event-driven path, or one page-backed native
  ring for the timer-driven path. Single-packet allocation is admitted with
  segmented position/consumption and PCM-copy helpers; timer rundown precedes
  packet release. This is implemented source, not target compatibility evidence.
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
warnings, API validation, native INF validation and CAT generation. That evidence
applies to the checked source revision; later source changes must rerun the check.
The five host suites are lifecycle, audio, ownership, timer and cleanup. They cover
ABI/bounds, clock/pause/resume, rational deadlines, render wrap/EOS, capture,
bounded completion batches and cancellation/claim ownership helpers. Extracted
production-helper regressions cover packet-clock reset fencing before counters
restart, stale-stamp rejection, sequence-zero admission only under a new epoch,
closed-gate reset, epoch exhaustion, event/timer allocation shapes and segmented
timer-ring PCM/gate isolation. Host tests do not exercise WDF, PnP, cancellation
or real audio APIs.

### Target observations - 2026-10-21

- Target access is established over SSH to the guest ending in `.128`, with a
  high-integrity session. The guest is VMware Windows 11 build 26200. Guest
  automatic sleep and hibernation are verified disabled; the earlier guest-access
  and power-preflight blockers no longer describe the current state.
- A test-signed package has been installed and exercised in the VM. This does
  not qualify production signing, other Windows builds or upgrade/rollback.
- Four dynamic raw children simultaneously create/remove/recreate with sibling
  preservation: PASS. This establishes the exercised raw-PDO lifecycle behavior,
  not four usable MMDevice endpoints or independent native audio routing.
- Root cleanup revokes the exact owning file object: PASS. Ten sequential `cap`
  opens, process kill and subsequent fresh reopen: PASS. No owner/data-driven
  driver fault was reported in those bounded control-lifetime checks; they did
  not exercise native PCM.
- Target `active-lifecycle-stage1` exits 0 at 01:07:49 on 2026-10-21
  (Asia/Shanghai). With an exclusive capture client held and a 480-frame buffer,
  gate closure leaves the stream Running. Remove is accepted as Removing for
  runtime PDO 25, then final Removed is observed after 25 ms while the native
  client is still held. After client release, the same stable endpoint recreates
  as runtime PDO/endpoint generation 26 with the same MMDevice ID. All six
  defaults are unchanged. Evidence: `D:/tmp/moiren-vad-20261020/active-lifecycle-stage1.result.json`,
  `active-lifecycle-stage1.stdout.log` and `active-lifecycle-stage1.stderr.log`
  in that directory. This is stage1 active-removal evidence only, not PCM,
  silence, crash-cleanup or default-policy qualification.
- App `app-integration-stage1` exits 0 at 01:11:25 on 2026-10-21
  (Asia/Shanghai), reporting PASS in 270 ms. It verifies the persisted pure
  virtual input -> Gain 0.5 -> microphone graph through production Windows
  `ManagedDriver` ownership, without starting native probe clients. Apply reaches
  App service Running with active revision 2 and open gates; Stop reaches Idle
  with endpoints Present and gates closed; Exit reaches Exited with the service
  joined and owning control file released. `pcm_verified=false` and
  `endpoint_absence_confirmed=false`: Exited/join is not final PDO absence.
  Evidence: `D:/tmp/moiren-vad-20261020/app-integration-stage1.*` (result/report
  JSON and stdout/stderr logs). Root reports the actual `SessionSpec` gate
  regression reproduced RED then fixed GREEN, with the final 229 App library
  tests passing, including the explicit run-limit regression. Final host evidence:
  `D:/tmp/moiren-vad-20261020/app-virtual-intent-final-tests.log`.
  This is partial App/service integration only; Stage2 native-client
  PCM is not yet verified, and unrelated native-probe PCM results do not close
  that App integration gate.
- The earlier ALL-states (mask 15) MMDevice result of 0/4 is superseded by a
  target-validated metadata-only child MEDIA/null-service INF: all four owned
  MMDevices now map successfully. Production INF matching was completed in
  `e8f769f`. Both `packet-offset-install` and `timerelapsed-install` updated the
  root using the production INF, but those earlier runs alone did not exclude
  selection of the same-rank `oem26` scratch child INF. The later production
  binding check below resolves that selection ambiguity, not fresh-OS install
  qualification.
- In an earlier run, the native probe entered its first exclusive Event Playback
  Mono Pcm16 case and started the native stream, but the first PCM transfer
  failed at `GetOverlappedResult` with revision mismatch `0x8007051A`. That result
  is historical; the later event-mode run below reaches eight successful cases.
- The first offset of the event-driven double buffer must be
  `allocation - packetSize`; the former offset 0 produced alternating half-buffer
  silence. After correction, target `native-packet-offset` passes 8/8 exclusive
  event-mode cases: mono/stereo, PCM16/Float32 and both directions. Each
  microphone gate-closed observation before admission and after closure contains
  12,000 silent frames. All six default-device observations are unchanged
  before/after the run.
- Event-mode playback records 47,040-47,520 matching frames/s with zero silence.
  Microphone records 32,640-37,920 matching frames/s and 21-32 dropped packets/s.
  Those significant microphone drops are part of the result, not a lossless
  or full-rate claim; an eight-case PASS does not remove that limitation.
- The `timerelapsed-install` timer-mode run passes all four playback cases.
  Its first microphone case records 23,136 signal frames and 25,344 silent
  frames, below the unchanged 50% matching signal threshold. This is the
  baseline before the probe-side half-period microphone pacing experiment.
- Probe microphone pacing keeps the first write immediate and at most
  one pending request, then waits half a native packet period after collecting
  Capture/CaptureDropped before the next write. Driver freshness and the signal
  threshold are unchanged. The first target run, `native-timer-halfphase`, aborts
  on a real long-stall epoch mismatch during stereo playback: not a PASS.
- The second run, `native-timer-halfphase-2`, passes the first seven cases, then
  fails after a long stall in case eight, Microphone Stereo Float32. Its failure
  diagnostic records approximately 22k signal frames, 942 silent frames and zero
  microphone drops at the abort; these are partial-run counters, not per-second
  rates or a full-case PASS. The phase shift substantially improves microphone
  deadline behavior in this observation, but does not eliminate long-stall epoch
  failures or establish an eight-case timer PASS for either of those runs. The
  pacing policy is retained as a probe fix, not a driver freshness change.
  Phase pacing alone does not prove sample-age freshness; the shallow-FIFO
  issue remains open and is being addressed separately.
- The `production-binding` check removes and verifies removal of only the
  scratch `oem26` child package, with no live children, then adds the combined
  production package using `pnputil /add-driver`. All four live children report
  Class MEDIA, INF `oem30`, section `Child.NT` and complete driver keys. This
  excludes scratch-INF selection for that run. Evidence is recorded in
  `production-binding.result`, `production-binding.properties` and
  `production-binding.stdout`. This is not a fresh-OS clean install.
- Under that verified production child binding, exclusive event mode passes
  8/8 cases. Half-period pacing records microphone matching rates of
  43,200-48,000 frames/s, 0-4,800 silent frames and 0-1 dropped packets; playback
  records 43,680-47,040 matching frames/s with zero silence. All six default-device
  observations are unchanged before/after. These later counters do not erase the
  earlier unpaced microphone-drop results.
- The `native-timer-production-phase` result exits 0 at 00:56:08 on 2026-10-21
  (Asia/Shanghai), passing all eight exclusive timer cases. Microphone matching
  rates are 46,053-47,712 frames/s, with 288-768 silent frames and zero drops in
  every microphone case. Playback matching rates are 45,056-47,104 frames/s,
  with 512-2,048 startup-silent frames. Page-rounded native buffer sizes are
  2,048 frames for mono PCM16, 1,024 for mono Float32 and stereo PCM16, and 512
  for stereo Float32. Positions advance in every case; all gate-silence checks
  contain zero signal, and defaults are unchanged. This full-matrix bounded PASS
  does not invalidate the earlier long-stall failures or establish repeatability.

Required target checks remain: fresh-OS installation and repeatable production
package installation/publication; repeated event/timer matrices, including
long-stall epoch behavior; sustained microphone delivery and PCM quality beyond
the bounded runs; investigation of earlier event-mode microphone drops; sustained
PCM rate/latency and broader event/timer client compatibility; standard-user
control (the current session is high integrity); pre-exposure default
eligibility; software mute/volume;
broader active-audio-client revoke/remove/recreate beyond the bounded stage1
result, including PCM and crash cleanup; App Stage2 native-client PCM and final
endpoint-absence checks beyond Stage1 service teardown; power transitions; Verifier;
eight-hour idle/audio runs; 100 lifecycle/crash rounds; resource/latency reports;
upgrade/rollback/uninstall; production signing and broader Windows-version
validation. Raw-child and exact-file cleanup PASS results do not close these
audio-path or long-duration gates.

The per-endpoint component URI replaces the provisional direction-wide IDs.
A regression first reproduced equal identities for two distinct endpoints,
then passed with the complete stable-ID mapping. This fixes the source-level
identity contract. The target now passes simultaneous four-raw-child lifecycle
and sibling preservation, and the metadata-only child INF permits four owned
MMDevices to map successfully. The bounded event-mode run additionally validates
the eight native PCM cases. Concurrent sibling audio-routing isolation and
fresh-OS installation qualification remain open. Production child binding without
scratch selection and one full matrix in each native mode are now observed;
endpoint mapping alone still does not prove PCM flow.
The opt-in `vad_lifecycle --exercise --simultaneous` probe holds four raw PDO
combinations and replaces each while checking sibling identities/epochs. The
bounded target lifecycle results above remain separate from opening KS streams.

The separate opt-in `vad_native --exercise` probe now prepares four owned PDOs
and requires four distinct MMDevice identities mapped through topology adapters
and their PnP parent to the exact control root and stable endpoint IDs. It then
requests all eight exact exclusive format/direction/shape combinations, checks
PCM patterns through Direct I/O and native clients, and observes microphone
silence on the same RUN stream before admission and after gate closure. Default
mode remains exclusive event-driven. The opt-in `--exercise --timer` mode uses
no-event initialization with bounded padding/position-driven polling and no
fallback to event mode. It requires fresh clock progress during the signal phase.
The timer-capable probe passed 19 host tests before these target runs, without
contacting a driver. Failure diagnostics retain signal/silence, microphone drops,
buffer/packet sizes, stream generation, mode and native position without replacing
the original error or skipping revoke/drain. The diagnostic version passed 23
host tests; diagnostic plus half-period pacing passed 25 host tests, scoped
Clippy with warnings denied and a local build. Those checks do not qualify
target audio behavior.
The guest passes bounded eight-case exclusive event-mode runs after the
packet-offset correction. The later verified-production-binding run with
half-period pacing improves the microphone counters as reported above; earlier
significant drops remain part of the evidence.
The earlier timer baseline passes four playback cases but fails its first
microphone signal threshold. Half-period pacing improves observed microphone
delivery; its two target attempts nevertheless abort on long stalls, with the
second reaching case eight after seven PASS results. The subsequent
`native-timer-production-phase` run passes the full eight-case timer matrix, but
does not close the long-stall/repeatability gate. These results do not close
fresh-OS installation, long-duration PCM quality, policy, shared software volume
or active-audio-client gates.
