# Moiren VAD driver status

This file is the canonical owner of VAD run and gate status; the other
`docs/vad-*.md` records are dated evidence logs and link here for the current
verdict. Status updated 2026-10-31.

## Latest physical checkpoint - 2026-10-31

The main-integrated app/probes `0fd3fbf` and diagnostic driver `c985fc3` are
deployed on the separate physical target. New `oem184.inf`, version
`2.15.56.622`, service Running/root PnP OK and exact SYS hash are verified,
with old rollback package retained and no reboot. Native retries are authorized
and have resumed; older paused-retry statements below describe dated history.
Only DAWN headphones are used for audible runs; speaker/default settings stay
unchanged.

The strict Shared microphone control now observes a nonzero, equal16-byte
post-copy native-buffer prefix, while its Shared client still receives all
SILENT packets. Actual packet-delivery/notification remains the next boundary;
no sole-driver or downstream-policy cause is established. A lower software-route
target768/recovery4 completes30seconds with zero retained provider drop/silence
and clock clamp/reset, but quality completeness/marker evidence still fails.
This is not continuous stability or physical latency acceptance. Exact build,
deployment, original failures, settings and verified archive are in the
[integration/capture-boundary receipt](../../docs/validation/2026-10-31-native-integration-capture-boundary.md).

Directions are from Moiren's perspective: virtual speaker is input/Source;
virtual microphone is output/Sink. Accepted ownership/direction/control-error
foundations are merged separately in PR43-45, not by merging draft PR26.
No lower default, calibrated sub-10-ms result or VAD/Process winner is claimed.

**Long-soak gate status (updated for the 2026-10-22 run, `cdd44a2`):** the
eight-hour **idle** run passed for the first time — 28,730/28,800 verified
windows, zero epochs, flat resource trend (see the "Long-soak attempt 2"
closing section of `docs/vad-validation-2026-10-21.md`). The eight-hour
**music** phase and a full 16-hour pass remain unqualified: the music gate
tripped environmentally on this VM (capture-gap + correlation-lag wander), and
the run verdict concludes full 16h PASS is not achievable on that host without
a lag-tracker measurement fix.

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

## Short timer packet progress - 2026-10-29

Local `9cba24b` fixes a reproduced source-level starvation:48/49-frame timer
packets with1.1ms callbacks could continuously rebase with zero progress.
Short packets now admit one bounded quantum of residual debt (capped by mapped
frames), while every callback retains one-packet/three-span/one-transfer work
bounds. Larger packets, event mode, queues and microphone freshness remain
unchanged. Overload still quiesces and records loss; timely redue service and
native PCM retention are not guaranteed by this software policy.

All10 MSVC host suites, WDK analyzed build and native INF validation pass.
Two independent final source reviews find no Important/Critical issue. The
generated unsigned development package is neither installed nor deployed; no
native audio or target qualification occurs. Hardware gates and prior run
verdicts are unchanged. Exact RED/GREEN, bounds and final artifact hashes:
[`2026-10-29-vad-short-timer-progress.md`](../../docs/validation/2026-10-29-vad-short-timer-progress.md).

## Separate physical target - 2026-10-29

The subsequently analyzed/test-signed package is installed on the separate
physical `LAPTOP-NP49RG6H`, not the development host: `ROOT\MEDIA\0001`,
`oem147.inf`, version `18.58.53.555`, service Running / PnP OK. Exact installed
SYS hash, package verification and deployment history are retained in the
[physical-target receipt](../../docs/validation/2026-10-29-physical-target-deployment.md).
This supersedes only the earlier statement that the generated short-timer
package was not deployed; it does not retroactively change that build receipt.

Bounded native VAD IO/Bus recordings now contain changing PCM and complete
collection/cleanup at explicit2048/1024 targets with independent capacity.
They still fail independent-recorder continuity; the2048 case also retains
provider faults. No stable lower preset or calibrated sub-10-ms physical
result is established. The output remains muted and a common pre-route
reference/calibration is not available. P1 and all unfinished long-soak/music
gates remain unqualified. VM timing qualification stays deferred; these
physical results do not erase its historical failures or prove naming
propagation while a native external client holds an endpoint.

The later [native recorder/Process follow-up](../../docs/validation/2026-10-29-native-process-followup.md)
reproduces the independent recorder's raw first-packet discontinuity without a
Process graph. Later packet adjacency has no observed mismatch; startup prefix
integrity is still unknown, and the strict quality failure is not exempted.
Native app parameter replacement now has separate bounded evidence, retaining
later delivery-envelope violations. Neither finding changes the VAD provider,
physical continuity, calibration or long-soak verdicts.

The [packet-period / Engine-block matrix](../../docs/validation/2026-10-29-vad-period-block-matrix.md)
observes144/480-frame playback packets with128/256-frame Engine blocks at
target1024/capacity4800. Three operation/cleanup cases complete; all four fail
recorder quality, and the fourth also retains microphone drop/silence and a
diagnosed probe Running/timeline-availability conflation. Original failed
receipts remain unchanged. No kernel fix, physical latency verdict, lower
default or long-soak qualification follows from this bounded software route.

Probe-only [progress-health repair](../../docs/validation/2026-10-29-vad-engine-progress-health.md)
`f72e03c` now separates transient unknown timeline data from confirmed Running
state, with pinned identity and bounded progress deadlines. Final509 probe
tests pass. A new480-packet/128-block/target1024 native retest completes its
schedule/collection/cleanup but still fails recorder continuity; no transient
null recurs in this run. Installed driver and application are unchanged, build
caches are retained, and physical/long-soak qualification remains open.

### DAWN and interrupted software-route follow-up - 2026-10-30

The later [DAWN app-output receipt](../../docs/validation/2026-10-30-dawn-vad-app-pcm.md)
uses the authorized DAWN headphones rather than the muted Realtek output of
the earlier physical receipt. Production VAD -> Bus -> DAWN PCM is retained;
no independent calibrated physical reference or continuity qualification is
established. No other speaker volume/mute or default role is changed.

Probe-only [source-readiness coordination](../../docs/validation/2026-10-30-vad-source-readiness.md)
waits for a bounded accepted-READ observation before starting its owned source.
Three repaired repetitions record zero driver drops. A35-second physical-master
diagnostic admits Exclusive256 / Engine32 / target768 / capacity4800 with zero
retained active loss/reset/xrun counters, but acquires no PCM and does not qualify
its declared delivery bound. No production default/provider/kernel change.

The separate [pure-virtual follow-up](../../docs/validation/2026-10-30-vad-software-readiness-crash.md)
retains cap1 overflow/reset failures and a cap8/target1024 completed30-second
recording that still fails driver-silence/recorder-continuity gates. Cap8/target768
refuses its budget; cap4/target768 is interrupted by a0x7E bugcheck and automatic
reboot before results persist. Matched kernel symbols/images recover an access
violation at `nvlddmkm+0x262fa6` in a System worker chain, without proving sole
root cause or excluding earlier corruption/indirect Moiren involvement. No
Moiren frame appears in that recovered stack. Native audio retries are paused,
not treated as passed or repeated automatically.

The final post-reboot audit retains the same installed SYS hash, service Running,
root PnP OK and no dynamic child/app/probe/build process. All three exact terminal
owned tasks are unregistered after verified archival; Cargo caches remain.
P1, naming propagation, physical sub-10ms and outstanding long-soak/music gates
remain unqualified. No new driver installation, system-setting change or
speculative kernel patch follows from the crash.

The offline [virtual-clock observability repair](../../docs/validation/2026-10-30-virtual-clock-loss.md)
adds retained owner-level omitted-period/frame counters and mandatory final
probe validation for the pure-virtual route. Native PCM loss and Engine periods
omitted by the recovery cap remain distinct. This changes no kernel source,
clock policy, default budget or hardware gate; native retries remain paused.

The [cross-target/RT-harness follow-up](../../docs/validation/2026-10-30-native-helper-cfg.md)
retains Linux/Miri compilation and private-clock test integration repairs,
plus the final `c222df2` application/probe deployment. Installed SYS remains
unchanged, Running /root PnP OK; no native audio retry or new hardware verdict.

The [probe example-test cfg follow-up](../../docs/validation/2026-10-30-probe-test-cfg.md)
repairs four Windows-only test-build helpers and deploys only the `9de94e4`
probe after528+5 release tests and strict Clippy. App `c222df2` and installed
SYS remain unchanged. Native retries stay paused; software checks do not
close physical continuity, calibration or lower-budget qualification gates.

## Audit follow-up - 2026-10-21

The reported Removing-state retry defect is absent in the current source:
`Revoke` accepts Removing, `VadRemoveChild` retains the PDO and retries
`MarkMissing`, and failed ACX removal leaves the added flag set and returns its
failure. Commit `7ac7283` adds production-extracted removal/cleanup/destroy tests
covering repeated failures, successful submission idempotence, callback timing,
identity checks, quota retirement and reference balance. All six MSVC and
portable host suites pass. Three scratch mutations (skip the retry, swallow
failure, remove the accepted-submission guard) are rejected by the regression.
No kernel source change or new target package is required for this finding.

Commit `93a9c0c` ignores only local driver project settings and generated
x64/ARM64 Debug/Release directories. The separate user-owned `driver/moiren`
template never entered the repository and was removed from the working tree on
2026-10-22.

The authenticated-user control ACL remains the deliberate standard-user,
single-owner design. It does not authenticate the Moiren executable or isolate
mutually untrusted local users. A competing permitted process can open an idle
bus first. See the [access boundary and continuous-test procedure](../../docs/vad-soak-validation.md).
An attempted limited S4U diagnostic task still received an administrator token;
its preflight refused to launch the probe. The temporary task was removed.
This is not a standard-user target PASS.

Fresh host checks pass: workspace tests (1,322 passed, zero failed, six ignored),
workspace all-target check and strict Clippy, gpui-test acceptance, and the WDK
analyzed build/INF validation. These host results do not establish target
long-duration stability. The opt-in continuous soak and its runner require
their own completed target evidence; starting or rehearsing them is not an
eight-hour PASS. Historical timer failures and outstanding release gates below
remain open. The 2026-10-22 eight-hour idle soak pass (header note above)
closes the idle-duration gate; the music-phase and 16h duration gates stay
open.

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
  This was Stage1 only; its native-client PCM gate was still open at that point.
- App `app-integration-stage2-readmission2` and `app-integration-stage2-readmission3`
  both exit 0 without timeout on the Windows 11 VMware guest `DRIVER-TEST`.
  Both use executable SHA256
  `FEEDC9F47333FD5C7E5016BFD8C050CA72D96025E1A56278180D4D5F75199E3C`, built
  from source commit `6c4e3c285a704fd67f49993488f89527dec2d9c1`. The persisted v2
  typed graph is VirtualPlayback -> Gain(0.5) -> VirtualMicrophone. With no
  native probe clients, a 4,004-4,005 ms idle hold observes real `frame_end`
  progress: 14 forward advances at 48 kHz and no timeline epoch change. The
  exact-root/stable-ID MMDevice mapping then starts stereo Float32, 48 kHz,
  exclusive event-driven WASAPI render and capture with 480-frame buffers and
  no SRC or fallback. Each run validates source `[0.25, -0.5]` as
  `[0.125, -0.25]` through Gain 0.5 and a second distinct signal
  `[-0.375, 0.125]` as `[-0.1875, 0.0625]`. Longest consecutive matching runs
  are 48,000/48,000 frames in run2 and 48,000/48,096 in run3; each independent
  signal phase meets the 48,000-frame threshold. During Stop, capture remains
  RUN at request, after acknowledgment, and while collecting 48,000 real silent
  frames in each run. The service joins and the owning control session releases.
  Gate telemetry observes capture stream generation 0 -> 1, a closed interval
  of 31 ms / 34 ms, then reopen. Playback stream generation also advances 0 ->
  1; its sampled gate remains open while the bounded readmission observation is
  pending, and later PCM checkpoints confirm both gates open. On service exit,
  both endpoints are only `Removing`; `endpoint_absence_confirmed=false`.
  Evidence (per tag: `.result.json`, `.report.json`, `.stderr.log`):
  `D:/tmp/moiren-vad-20261020/app-integration-stage2-readmission2` and
  `D:/tmp/moiren-vad-20261020/app-integration-stage2-readmission3`. This is two
  bounded passes, not long stress or release qualification. A subsequent
  read-only PnP snapshot found no present `MOIREN\VAD_RAW` children after the
  asynchronous service-exit removal had time to complete. That later snapshot
  is distinct from the run reports' `Removing` state and is not default-policy
  evidence. Default-device policy/arrival is not qualified; the probe does not
  change user defaults.
  Timer-mode, shared-mode and physical-device paths, eight-hour music/16-hour
  runs (the eight-hour idle run passed on 2026-10-22), 100-round
  lifecycle/crash tests, Verifier and production signing remain unqualified.
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
endpoint-absence checks beyond the two bounded Stage2 passes; default-device
policy and arrival qualification; timer/shared/physical-path integration; power
transitions; Verifier; eight-hour audio/16-hour runs (idle passed 2026-10-22);
100 lifecycle/crash rounds;
resource/latency reports; upgrade/rollback/uninstall; production signing and
broader Windows-version validation. Raw-child and exact-file cleanup PASS results
do not close these audio-path or long-duration gates. The Stage2 probe does not
change user defaults and does not qualify their arrival policy.

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
