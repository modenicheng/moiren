# ACX audio lifecycle

The `src/audio/` subtree is now linked by `MoirenVad.vcxproj`. This file retains
its original development filename; it no longer describes an excluded source
drop. Target-machine audio/PnP qualification is still pending; see STATUS.md.

As of 2026-10-21, a test-signed package is installed in the VMware Windows 11
build 26200 guest. SSH access to the target ending in `.128` is high integrity;
guest automatic sleep and hibernation are verified disabled. Four simultaneous
dynamic raw children create/remove/recreate with sibling preservation: PASS.
Root cleanup exact-file revocation and ten sequential `cap` opens, process kill
and fresh reopen: PASS. No owner/data-driven driver fault was reported in these
bounded exercises. These are control/raw-PDO lifecycle results, not native audio
or complete product qualification.

Target `active-lifecycle-stage1` exits 0 at 01:07:49 on 2026-10-21
(Asia/Shanghai). A held exclusive capture client has a 480-frame buffer; gate
closure leaves Running, and Remove accepts Removing for runtime PDO 25. Final
Removed is observed after 25 ms while the client remains held. After release,
the same stable endpoint recreates as runtime PDO/endpoint generation 26 with
the same MMDevice ID. Six defaults are unchanged. Evidence is
`D:/tmp/moiren-vad-20261020/active-lifecycle-stage1.*` (result JSON, stdout and
stderr logs). This stage1 PASS does not qualify PCM, silence, crash cleanup or
default policy, and does not replace broader active-client qualification.

App `app-integration-stage1` exits 0 at 01:11:25 on 2026-10-21
(Asia/Shanghai), reporting PASS in 270 ms. The persisted pure virtual input ->
Gain 0.5 -> microphone graph uses production Windows `ManagedDriver` ownership.
The probe starts no native clients: Apply reaches App service Running with
active revision 2 and open gates; Stop reaches Idle with endpoints Present and
closed gates; Exit reaches Exited, service joined and owning control file
released. The report explicitly retains `pcm_verified=false` and
`endpoint_absence_confirmed=false`; it does not prove final PDO removal.
Evidence is `D:/tmp/moiren-vad-20261020/app-integration-stage1.*` (result/report
JSON and stdout/stderr logs). Root reports the actual `SessionSpec` gate
regression reproduced RED then fixed GREEN and the final 229 App library tests
passing, including the explicit run-limit regression. Final host evidence:
`D:/tmp/moiren-vad-20261020/app-virtual-intent-final-tests.log`.
This is partial App/service integration, not native-stream or end-to-end PCM
qualification. App Stage2 PCM and final endpoint absence remain unverified;
the separate native-probe results do not qualify them.

The earlier ALL-states (mask 15) MMDevice result of 0/4 is superseded. A
metadata-only child MEDIA/null-service INF has been validated on target, and
all four owned MMDevices now map successfully. Production INF child matching is
implemented in `e8f769f`. Both `packet-offset-install` and `timerelapsed-install`
updated the root with the production INF, but those earlier runs alone did not
exclude selection of the same-rank `oem26` scratch child INF. The first
publication breakthrough was the metadata-only experiment. The later
`production-binding` run removes and verifies removal of only that scratch child
package with no live children, then adds the combined production package via
`pnputil /add-driver`. All four live children report Class MEDIA, INF `oem30`,
section `Child.NT` and complete driver keys. Scratch selection is excluded for
that run, with evidence in `production-binding.result`,
`production-binding.properties` and `production-binding.stdout`; this is not
fresh-OS clean-install qualification.
The earlier first-PCM revision mismatch `0x8007051A` is historical: the later
target `native-packet-offset` run passes all eight exclusive event-mode native
PCM cases (mono/stereo, PCM16/Float32, both directions). The event-driven double
buffer's first offset is `allocation - packetSize`; the former offset 0 caused
alternating half-buffer silence. With the correction, each microphone gate-close
observation before admission and after closure contains 12,000 silent frames,
and all six default-device observations are unchanged before/after the run.
Playback records 47,040-47,520 matching frames/s with zero silence. Microphone
records 32,640-37,920 matching frames/s and **21-32 dropped packets/s**. Those
significant drops preclude a lossless/full-rate claim despite the bounded PASS.

The `timerelapsed-install` timer run passes its four playback cases. The first
timer microphone case has 23,136 signal frames and 25,344 silent frames, below
the unchanged 50% signal threshold. That is the pre-pacing baseline.

Probe-side half-period microphone pacing keeps the first write
immediate and at most one request pending, then delays the next write by half a
packet period after collecting Capture/CaptureDropped. It does not relax driver
freshness or the signal threshold. Target `native-timer-halfphase` aborts on a
real long-stall epoch mismatch during stereo playback. The repeat,
`native-timer-halfphase-2`, passes seven cases but fails after a long stall in
the eighth, Microphone Stereo Float32. At abort, diagnostics report approximately
22k signal frames, 942 silent frames and zero microphone drops. These are
partial-run counts, not rates or evidence of a successful final case. The phase
shift substantially improves the observed microphone deadline behavior, but
neither of these two runs passes the full eight-case timer matrix. Pacing is
retained as a probe fix, without changing driver freshness. The failures remain
evidence even after the later bounded full-matrix PASS. Phase pacing alone does
not prove sample-age freshness; the shallow-FIFO issue remains open and is being
addressed separately.

The verified `production-binding` run passes 8/8 exclusive event cases with
half-period pacing. Microphone matching rates are 43,200-48,000 frames/s, with
0-4,800 silent frames and 0-1 dropped packets. Playback matching rates are
43,680-47,040 frames/s with zero silence; all six defaults are unchanged
before/after. This improves the later result without erasing earlier drops.

The subsequent `native-timer-production-phase` result exits 0 at 00:56:08 on
2026-10-21 (Asia/Shanghai), passing the complete eight-case exclusive timer
matrix. Microphone matching rates are 46,053-47,712 frames/s, with 288-768 silent
frames and zero drops in all microphone cases. Playback matching rates are
45,056-47,104 frames/s, with 512-2,048 startup-silent frames. Page-rounded native
buffers are 2,048 frames for mono PCM16, 1,024 for mono Float32/stereo PCM16 and
512 for stereo Float32. Every case has advancing position, all gate-silence
checks contain zero signal, and defaults are unchanged. This is a bounded
full-matrix PASS, not a stable-repeatability claim: earlier real long-stall epoch
mismatches remain recorded. Long-stall behavior, repeated runs and sustained
audio quality still require qualification.

Child initialization follows the pinned SdcaVad raw-audio PDO sequence:
ConfigureInit before WdfDeviceCreate, Initialize afterward, then
AcxDeviceAddCircuitDevice on the root. There is no second static child list.
PrepareHardware runs policy staging on a system worker and adds the child's
circuit only after successful registry preparation. RemoveCircuitDevice starts
asynchronous PnP removal; ReleaseHardware stops/drains native streams and removes
the circuit. Final PDO destruction alone releases its quota slot.

The source permits one native stream while the Engine PCM gate is closed.
Native stream generation and gate epoch are independent. The closed-gate source
path discards render and produces capture silence; the bounded event-mode guest
run now observes microphone silence before admission and after closure. No file,
tone, fake hardware volume/mute or DSP paths are present. PCM uses the bounded root manual queues
described by ABI.md.

Native packet/timer resources are serialized by an embedded passive lifecycle
mutex. DPC state uses nonpaged spin-locked helpers. Timer stop/drain precedes
packet detachment/free, and cleanup handles partially initialized streams.
QPC presentation positions are monotonic over pause/resume and rebase after a
stall. Event-mode callbacks complete at most one native packet; timer mode
processes a bounded segmented quantum without packet-complete notifications.
Helper tests run outside WDF.
Every full packet-clock reset advances the PCM gate epoch before purging
requests, including stream ReleaseHardware, FreeRtPackets and cleanup. Reusing
the same ACX stream for Release/Prepare restarts packet numbering under a new
epoch even when polling misses all intermediate states. The new epoch and paused
state are published under the same lock, so fresh requests cannot enter before
the pending purge; admission resumes with RUN. Exhausted epochs close admission
instead of wrapping.

Native allocation admits two packets for event-driven streaming or a single
page-backed ring for timer-driven streaming. Count 1 is no longer rejected at
allocation: segmented position/consumption and PCM-copy helpers are implemented.
Pure host regressions cover event/timer shapes, ring segments and epoch/gate
isolation. Neither allocation admission nor those host tests establish Windows
client compatibility. Bounded event-mode target evidence is recorded above;
one full timer matrix now passes on target, but repeatability and broader
native-mode qualification remain open.

The `vad_native` example keeps exclusive event-driven operation as its default.
Explicit `--exercise --timer` selects no-event initialization and bounded
padding/position-driven polling without falling back to event mode. It reuses
the PCM/gate checks and requires fresh signal-phase clock progress. The updated
timer-capable probe passed 19 host tests before the target runs. Guest event
mode passes 8/8 cases after the packet-offset correction; earlier significant
microphone drops and the later paced improvement remain disclosed. The timer baseline passes 4/4 playback cases
but fails its first microphone threshold. Two half-period pacing attempts still
abort on real long stalls; the repeat reaches case eight after seven PASS
results. The later production-phase run passes timer 8/8; neither that bounded
PASS nor the event result establishes stable repeatability or broad event/timer
interoperability.
The failure-diagnostic version passed 23 host tests and preserves the original
error and revoke/drain. Diagnostic plus pacing passed 25 host tests, scoped
Clippy with warnings denied and a local build; this is not target qualification.
Production child binding without scratch selection is verified in the bounded
run. Fresh-OS installation, long-stall repeatability and sustained audio quality
require separate validation.

See PROVENANCE.md for pinned sources and retained MS-PL notices. Linked WDK
build/static analysis is independent of target audio validation. Test-signed VM
installation and the bounded lifecycle checks are now observed; production
signing, Verifier and full real Windows audio qualification remain separate.
