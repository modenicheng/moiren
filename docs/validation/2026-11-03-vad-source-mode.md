# Explicit owned VAD source mode

Date: 2026-11-03, Asia/Shanghai. Tested clean baseline: `5e1cefb` plus exactly
three hash-pinned example-source overlays. This is an original S1 experiment
control, not full S1-S3 acceptance or a change to production defaults.

## Why this slice exists

The owned deterministic Process source already opens Shared, while the owned
VAD source previously always opened Exclusive. The original S1 comparison asks
for matched application mode, rate and buffer. Opening an arbitrary Shared
MMDevice instead would bypass the VAD stable-ID/control-root resolution. This
slice makes the mode explicit without bypassing that identity contract.

`live-vad --vad-source-mode shared|exclusive` selects exactly one initializer.
Omission preserves Exclusive. There is no fallback after a selected open fails.
Shared still resolves the exact VAD stable ID and control root, then uses the
existing exact 48 kHz stereo mix-format initializer and its default native
buffer. Shared plus `--vad-minimum-period` is refused before native setup.
Duplicate, unknown, case-variant and Process-route options are refused.

Requested mode appears separately in the manifest, source and MIC-route intent.
Only prepared native info records actual `shared_vad_mmdevice` or
`exclusive_vad_mmdevice`; a dry run or preparation refusal does not invent an
actual stream. Generic explicit-MMDevice Shared remains a distinct identity.
Matching mode alone does not match buffers, periods or latency. This does not
claim to repair the prior Exclusive exact-format refusal (`0x88890008`).

No library/driver algorithm, RT ordering, timing budget, target/capacity,
recovery policy, default, quality gate or first-packet policy changes.

## Source and software evidence

Local immutable evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-source-mode-20261103-0230`.
All27 pre-existing dirty/cache files are excluded from the tested export.

```text
baseline: 5e1cefb5bf2942ffcbd86d8ff66144f94ce38f06
baseline archive SHA256:
F979F68C3DA3F0C6500F1A6B34EB9C3E446E7E90D7F0986BD9C17DD3E7DCC568
overlays:
live.rs        A125F9FC33AC8E5A1317C51B39CC488B6B52BC958DA80D77621969FB079F12DB
live_native.rs 31244502A89B56B225CF9A9C4D914B2CCF1FD72137751EF13ECB0F938247722B
vad.rs         A5A458931AE81E10C405944F98904E53786BBA920F4DEEB48BFAD852DA260E80
e2e_latency_probe.exe: 7,597,056 bytes, SHA256:
68F98691883D41211ED71D125FC828FFD083490103AC3EE97119224A0FFAD248
```

The new Shared dry-run regression first fails with the old unknown-option
behavior. Six focused tests then pass: request/route persistence, default and
invalid combinations, exact selected-opener dispatch without fallback,
stable-root target identity and refusal before native owner setup. The full
probe example passes715 tests/1ignored both default and with
`probe-tools,vad-probe`. Feature-on strict example Clippy, including tests,
release build and isolated workspace formatting pass. Runner pure/static
tests pass40 checks each on Windows PowerShell5.1 and PowerShell7, independently
rerun by the parent. A first GREEN compile had an invalid test-only constructor;
its log is retained separately from the actual behavioral RED.

The clean exported workspace also passes2352tests/9ignored across71 harnesses,
including `moiren-app/gpui-test` and `moiren-windows-audio/vad-probe`. These are
portable/headless checks, not host native audio or UI operation. Feature-on
all-target workspace `cargo check` passes as well.
Feature-on all-target workspace strict Clippy also passes. Launcher closure
regressions pass seven pure cases on each PowerShell version: a missing audit,
unknown/nonzero wrapper exit, incomplete audit, unconfirmed child or remaining
owner cannot trigger a second catalog child. The initial review found that
failure-path overlap; it was corrected and independently reviewed before any
task launch.

## Remote experiment contract

Remote-only root: `D:\MoirenTest\20261103-vad-source-mode-0230`.
The native question is deliberately distinct: can explicit Shared open the
stable/root-bound source and retain actual dispatch, normal Stop/join and an
independently joined raw input prefix? It is not an unchanged stability rerun.

The route is VAD Playback -> unity Bus -> VAD MIC, Compatible settings,
48 kHz/F32/stereo and Engine64. The indexed source is six seconds/288000frames,
with six1024-frame amplitude0.025 markers at12000,60000,108000,156000,204000,
252000. Input raw prefix is capped at480000frames; collection at20seconds.
Recorder schema2 uses Shared, ring16384, PCM16MiB, metadata2MiB, max3000packets,
preroll250ms/postroll500ms. No smaller input target/capacity, source minimum,
output-raw capture or recovery override is introduced.

The create-once selfcheck must pass with the same intent hash before one native
attempt. Both use Limited Interactive session1 ScheduledTasks with240s outer
limits. The unchanged collection helper retains its direct child handle,
90s deadline and bounded termination/pipe drains. There is no independent150s
supervisor. A task timeout alone cannot establish child closure or stable
artifacts; exact receipts and an independent final owner audit are required.
Endpoint catalogs are captured before/after using a separately held8s child.

Preflight at02:30:39 confirms no test/analysis owners or running Moiren task,
bare healthy bus, unchanged OBS/QQMusic creation identities and running driver
hash,26.8GB free memory and151GB free disk. New distinct endpoint IDs and all
binary/config/runner pins are retained in the immutable intent. No host native
audio/UI or user-process manipulation occurs. Raw PCM stays remote.

Audit completion is separate from functional collection and quality. Expected
probe exit1 means unqualified, not runner failure; helper errors/timeouts,
forced termination, unstable outputs and manifest mismatches remain refusals.
Functional collection additionally requires actual Shared dispatch, exact
prepared identity/root, normal source Stop/join, completed source/collection
and cleanup, plus a nonempty joined raw prefix. No partial/refused result may
be promoted to waveform, sustained, lower-target or physical qualification.

## Actual remote result

Selfcheck launches once at02:36:29; its worker runs02:37:01-02:37:09.
Requested Shared fields are present, no native PCM is collected, and source
actual mode remains unavailable as expected. Selfcheck, worker and task exit0.
The native task launches once at02:37:31; worker runs02:38:02-02:38:38.
Probe PID20196 exits1 after22.758seconds, without timeout or forced termination.
Worker/task exit0 means a complete retained-evidence audit, not audio success.

Actual source is `shared_vad_mmdevice`, Float32, native buffer1056frames.
Prepared stable ID, MMDevice, adapter and root match the owned VAD request.
The source acknowledges all288000scheduled frames, with native preroll1056
and314496total submitted frames. Native Stop succeeds and the worker joins;
no source failure or event timeout occurs. Submission is not playback/delivery.

The independent Playback input witness retains312480PCM16/stereo frames,
1,249,920bytes over651packets, worker completed/joined. It seals at
`identity_changed`, before its480000frame cap; this is not a complete source-to-
recorder mapping or proof of post-seal continuity. The MIC sink retains
327552Float32/stereo frames/684packets,2,620,416bytes. Sink incomplete/missing
packets, sequence gaps, nonfinite samples and ring drops are zero.

**The collection quality gate refuses:** MIC evidence has4discontinuity flags
and2device-position gaps totaling2208frames. Their retained positions are:

| MIC packet | Stored frame | Flags | Position gap frames |
| --- | ---: | ---: | ---: |
| 1 | 0 | 1 | unavailable (first packet) |
| 3 | 960 | 1 | 0 |
| 4 | 1056 | 1 | 1824 |
| 20 | 8736 | 1 | 0 |
| 21 | 8832 | 0 | 384 |

These are within the first0.184seconds of the MIC's stored-frame axis, not a
calibrated common timeline. The flags/position counters alone do not establish
physical lost audio, its cause, or steady-state waveform continuity. They are
not waived or recategorized to force a pass. First-packet work stays deferred;
no unchanged rerun, larger buffer or weaker gate follows this result.

Source submission and cleanup complete, but `collection_complete=false`,
`quality_passed=false`, and the retained native outcome remains
`native_refused_or_failed_partial_evidence_retained`. Both live providers
report zero drops, recovery events and epoch resets; shutdown snapshots and
the input identity seal remain separate evidence, not overwritten with zeros.
Shutdown retains Playback epoch_resets2/recovery_events2, MIC1/1 and the
invalidated Playback publication with one segment break; these are not erased
or described as live faults, nor attributed as causes of the MIC gaps.
Engine closes with no cleanup error; both exports reach Removed/Closed.
Final independent audit finds no native/analysis owners and only the healthy
control bus. Protected OBS/QQMusic identities, driver hash, default roles and
render volume/mute are unchanged. Neither native delivery nor S1-S3 passes.

All56compact JSON/log/script artifacts are individually length/SHA256 verified
inside the closed ZIP, preserving relative paths. Raw PCM stays remote.
Archive1,472,541bytes, SHA256:
`717FBC10E0F56B653A4F8DDE01C74E5AAFC4E2DF302A5A178C008EDCA471742E`.
This successful Shared open is a distinct mode observation, not evidence that
Exclusive exact-format preparation has been repaired or that either route is
the lower-latency winner.

## Offline visualization and next boundary

The four exact PCM/metadata artifacts are analyzed remotely, without another
audio run. The isolated existing Python environment uses `-I -B`, local cache
directories and one retained60s child deadline. Actual analysis takes12.088s,
exits0, drains normally and rechecks every original size/SHA256 after plotting.
Fourteen synthetic-only tests pass locally, independently rerun by the parent.
No real PCM is copied to the host. Only summary660416bytes and plot224446bytes
are transferred; both hashes are verified and the full plot is visually reviewed.

```text
remote: D:\MoirenTest\20261103-vad-source-mode-0230\descriptive-analysis
local:  <local evidence root>\analysis-captured\independent-stored-time.png
plot SHA256:
C19243F79DD61E27FC364C6E2FF2BF85254249218A98404C8A049299C2A70F49
summary SHA256:
FCA5D76F126FC7D0D89680933E83D6DB043E641CDA4BE94D123B25F913B11A77
```

The five panels show full-retained-input/MIC10ms RMS on distinct stored-time
axes, opaque input driver-timestamp deltas, MIC packet QPC spacing, and raw
MIC flags/device-position adjacency deltas. All651input publications are
retained as `published`; the identity seal remains explicit. Each waveform
has six visible RMS bursts, but this descriptive plot does not identify the
indexed markers, align source and recorder, certify signal continuity or
estimate delay. MIC peak0.04833984375 versus input0.025054931640625 is retained
without an invented clipping/distortion diagnosis or marker-quality verdict.
`association=not_attempted_distinct_stored_origins`, `latency=not_estimated`
and `quality_passed=null` describe the offline analysis; the native collection's
separate quality result remains false.

Final02:49:01 audit verifies all56closed compact files and all4raw inputs
unchanged, both tasks Ready/exit0, no owners, healthy bare bus and unchanged
protected processes/driver. Nothing is left running as an unchanged soak.
Independent S1-S3 development may continue; the remaining acceptance work is
matched period/buffer and waveform evidence, admitted lower-target behavior,
retained/desktop acceptance, sustained delivery and calibrated physical
qualification when suitable equipment is available. First-packet work is not
reopened by this slice, and no success is inferred across these boundaries.
