# Source-process capture before physical graph control

Date: 2026-10-31, Asia/Shanghai. Physical target: `LAPTOP-NP49RG6H`.
Installed driver remains `6d9e32f` / `oem201.inf` / `3.1.1.607`; holder
`ac93295`, source/recorder probe `0fd3fbf`, app `6827f02` remain unchanged.
This is a new source-side discriminator, not another recording of app output,
an exact S1 fixture, a continuity pass or calibrated physical latency.

## Question and unchanged boundaries

The [joined app control](2026-10-31-app-exclusive-joined.md) retains all-zero
Process recording of app output while frame counters advance. Such counters
do not show nonzero input. The next experiment records the selected changing
source process itself, without a Process monitor or DAWN/physical render client.
The existing VAD service holder still runs Playback -> Gain0.5 -> Microphone;
"before graph" identifies the observed source-side Process tap, not graph-free
execution or physical ordering of Windows audio internals.

Directions remain Playback/virtual speaker -> Moiren Input/Source and
MIC/virtual microphone -> Moiren Output/Sink. The source renders only to the
exact service-owned Playback, selected by root/stable ID/direction/confirmed
revision and matching native identity. No friendly-name/default selection or
settings, driver, privacy, autologin or volume changes occur.

## Native collection

`source-process-before-graph1` uses the frozen, independently reviewed
`run-source-process-control.ps1`, SHA-256
`71990E5E5E41DF941A8D33E8CDB4896C4B26D65CC3EB01E2F92892E1860F6740`.
It starts one Shared source, waits for its Running receipt, then invokes the
existing `record-process` for6seconds/max480000frames with source PID11920
and creation134378710409097725. Requested, selected and native-capture process
identities all match. The recorder returns/join is observed before source Stop;
the source remains alive and Stop publication starts13.2655ms later.

The source's20second schedule has20 contiguous48000-frame marker windows.
Source start-before through Stop-after spans6503.45ms. Final source elapsed is
6.3201223seconds,303360acknowledged/nonzero-source frames plus1056silent
native preroll =304416submitted frames. The schedule intentionally remains
incomplete. This excludes schedule exhaustion/source exit as explanations for
this capture; submission still does not prove actual played PCM or establish
sample-by-sample source/recorder clock mapping.

The recorder completes6.0019147seconds,599packets/287520stereo frames and
persists2300160bytes. Independent host scans find every byte zero:575040
finite samples, peak/RMS0 and no adjacent-sample changes. Raw PCM SHA-256:
`38C55B4F67ADB02BDC9BE453452426660B81AE74A2519DD37E516BECFADC391D`.

`recording_complete` and `artifact_persisted` are true; failures are empty,
native status completed and Stop succeeds. Recorder exit1 is its explicit
always-unqualified contract, not a failed process-identity check or a signal
pass. Artifact drop/overflow, discontinuity, timestamp-error and SILENT frame
counts are0. `silent_frames=0` means no SILENT flags, not nonzero PCM.
One timeout wake remains. Position-gap detection is disabled and delivery
bounds are null; zero violation counters cannot qualify timing or continuity.

The unconsumed compatibility bridge separately drops287520frames. Its drop
does not explain the earlier independent native artifact; that tap is copied
before bridge publication. `buffer_frames` remains null, and the retained
implausible raw buffer-size query is not interpreted as native capacity.

## Cleanup and interpretation

Collector exit0, source/holder exits0, recorder expected exit1; no forced
termination or remaining owned process. Holder reports passed,60582ms total,
60003ms idle,59timeline windows and no recorded timeline glitch/epoch change.
Recursive independent audits of its complete report, seven checkpoints and
all14progress snapshots find no nonempty error/graph_error/cleanup_error.
Service joined/control released are true; endpoint_absence_confirmed remains
false, while final PnP separately sees only the healthy original root.

Installed SYS hash remains
`AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268`;
boot remains `2026-10-30T09:50:49.659845+08:00`. No build overlaps collection,
no cache is cleaned and no target artifact is deleted.

This source Process recording is all-zero without starting the proposed
Process graph/physical-render experiment. Therefore that app processing graph
and DAWN Exclusive output are not necessary conditions for this observation.
A nonzero Process positive control is still missing; no sole driver, Session0,
source, format or Windows API cause is established. Do not repeat equivalent
headless source/recorder controls or lower buffers to hide the failure. An
independent capture implementation or client mapping boundary is a different
discriminator; it must retain its own format/identity/lifetime limitations.
S1-S3, waveform continuity, a stable smaller default and calibrated sub-10-ms
physical output remain unachieved.

## Verified archive

Host evidence:
`D:\coding\moiren-evidence\20261031-source-process-control`.
Remote originals:
`D:\MoirenTest\20261031-source-process-control\runs\source-process-before-graph1`.

`source-process-control-20261031.zip` contains39payload entries plus its
manifest,3596293payload bytes excluding the manifest and150911ZIP bytes.
It retains the complete run and frozen runner, not app/driver binaries or
host-only analysis. Target and host check each member's length/hash, entry
count and aggregate bytes. The host compares all39members to its separately
downloaded originals and verifies the embedded manifest. Originals remain.

- ZIP SHA-256: `781A2DFE553688D7360B7A695EE2B1CD6B575CDC00DA9BA151A629E3293A33ED`.
- Manifest SHA-256: `A986DB3859D7ECC96D2E3526C87CE3335AD26A38E972FBBD43B19476456DEF21`.
