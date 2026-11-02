# Current VAD -> Bus -> DAWN exact336 functional recording

Date: 2026-11-02, Asia/Shanghai. Source: clean committed `1267f8d` export,
excluding the18preserved dirty files. No production code or default changes.

## Result and stopping boundary

One bounded native recording establishes current application admission and
changing PCM on the exact VAD Playback -> unity Bus -> DAWN PRO2 route, at
48kHz/F32/Engine64, Shared requested/observed336 and render target512.
It is **not an overall PASS**: the runner retains `Quit deadline`, forced
owned-app termination and exit-1. Whole-waveform quality, matched VAD/Process
comparison, physical latency and lower-default qualification remain open.

No second audio run, smaller-target scan or longer soak follows this result.
The user's first-packet deferral remains in force. Absent physical calibration
hardware does not block independent product development. A bounded committed
source audit finds no further necessary original S1-S3 software-integration
slice; add code only for a demonstrated defect, not another unknown epoch.

## Exact native setup and provenance

- Target `LAPTOP-NP49RG6H`; one limited interactive scheduled task,
  `Moiren-VadDawn336-20261102-0812`; lifecycle `task`.
- Deployment `D:\MoirenTest\20261102-vad-dawn-exact336-0812`; run
  `runs\functional20`. Task launched08:06:37; native runner08:07:08-08:09:02.
  The directory/task suffix0812is an identifier, not the actual launch time.
- Application SHA256
  `A836C1CF83E977650C7B8ECC4D4A99B24C47569ACF62C42317864F260953B043`;
  probe SHA256
  `3C7D74309CF0EC2C846D34969F8B3D1FC1EB830A29E713666EE0A6B6AE7310EE`.
  Both release builds exit0from committed export tree
  `27df5b23299e606404c763dc8efbcceed9ac0c9d`.
- Installed running driver SHA256
  `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`;
  before/after path, hash and boot identity unchanged. No driver installation.
- Physical sink is exactly
  `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`.
  Only one owned Playback export; no MIC sink. Project bytes
  `4d6f6972656e533100000000000000a5` resolve by raw PnP parent/stable suffix
  under `ROOT\MEDIA\0001`, not a friendly-name match.
- Isolated APPDATA/TEMP. No host native audio/UI/process operation, global
  volume/default/timer change, user OBS action or QQMusic action.

The live-vad probe's explicit-period parser cannot request336. The existing
desktop physical-VAD runner was adapted with the existing exact Shared-period
preferences instead; no parser widening or silent512/default substitution.
The deployed runner, launcher and source/binary manifests are hash-verified.

## Requested versus observed

Native UIA and application logs independently confirm:

| Quantity | Actual evidence |
| --- | --- |
| Graph | VAD Playback -> unity Bus -> exact DAWN physical Master |
| Engine | 48000Hz, F32,64frames |
| Output | WASAPI Shared, Client3, requested/observed336frames, no fallback |
| Native capacity | 738frames, not the requested period or E2E delay |
| Render policy | target512, wake64, callback64frames |
| VAD policy | Compatible; no explicit input target or recovery override |
| Prepared VAD boundary | capacity4800, pre-transfer content target2448native frames |
| Complete modeled demand |704Engine frames,706native transfer,32FIR, cap8 |
| Running snapshot | occupancy2541native frames, applied+136.575ppm |
| Boundary counters | Xrun0; soft0before source, soft32after recording |
| Master/input bridge snapshot | dropped0/silence0frames |

The32soft events are retained, not erased, converted into PCM loss or claimed
to be wholly startup. Virtual-clock rows are Unknown on this physical-Master
route. Compatible delivery bounds and negotiated packet fields remain Unknown
in this UI snapshot; absence is not invented native timing. The retained
provider logs observe Pcm16/480-frame client packets and dropped/silence0at
their explicit query points, not continuous all-fault qualification.

## Bounded source and independent recordings

One exact owned source process emits indexed-v2 deterministic1024-frame
markers at nominal1s spacing, amplitude0.025, with intentional inter-marker
silence. It is not continuous white noise. The source schedule is60s to
cover readiness/recording; the runner deliberately stops after the20s
recorders and diagnostics, rather than submitting the whole schedule.

Source PID25880/creation100ns134380516707183264is checked against its held
native process identity. The source runs26.0356s, acknowledges1249440source
frames, submits1250496native frames including1056preroll, and exits0with
joined worker/native Stop and no failure. `schedule_submitted_complete=false`
is retained as the planned early-stop fact, not relabelled completion.

Two capture-only Process-tree recorders start before source activation:

| Stored PCM | Source process | Application process |
| --- | --- | --- |
| Frames |959040|959280|
| Duration |19.980s|19.985s|
| Nonzero channel0frames |19456|28368|
|10ms RMS energy islands |19|19|
| Peak absolute |0.0250000004|0.0483581722|
| Maximum stereo difference |0|0.0000849254|
| Artifact dropped/overflow/discontinuity/timestamp-error |0/0/0/0|0/0/0/0|

Both reports retain recording_complete/artifact_persisted, native Stop success,
no native failure and expected unqualified recorder exit1. Position-gap
detection is false for Process recording, so its zero gap counter is not
continuity proof. Delivery bounds are unset; zero bound-violation counters
cannot prove a delivery contract. Intentional zero PCM does not necessarily
carry WASAPI's SILENT flag; zero silent_frames is not uninterrupted sound.

Remote isolated Python (`-I -B`, existing analysis environment, single-thread
numeric settings) generates `analysis\summary.json` and
`analysis\functional-pcm.png`. Source/application axes each use independent
stored-frame zero; no cross-recorder alignment, marker-identity association,
clock fit or delay is inferred. The changed application waveform/peak and
stereo difference are visible, not silently normalized away. Energy islands
show changing signal presence only, not lossless marker correspondence.
This capture is application output before a calibrated physical return;
neither Windows process loopback nor a7ms period proves physical latency.

## Exit failure and cleanup

The source stops before application Stop. The native owner cleanup finishes
and is reaped at08:08:20.873. The stopped snapshot has no active native timing.
UIA invokes Quit at08:08:28.889but the process does not exit within15s.
The retained failure screenshot shows the Quit button partly hidden under
the fixed settings footer although UIA reports Enabled/Offscreen=false and
Invoke returned. The runner's viewport check used Save.top rather than the
actual footer top. Therefore this run does **not** establish that the
application's Quit handler executed, nor a new audio-owner deadlock.

The later owned-window close attempt logs GPUI errors0x80040102/0x80070578;
the runner finally terminates only its exact held owned-app process.
Result.Completed=false, Error=Quit deadline, FinalExitCode=-1are authoritative.
This automation/exit limitation is retained. The later
[stopped-app viewport control](2026-11-02-desktop-quit-viewport.md) fully exposes
Quit and exits normally with code0 on the same executable, without an audio
owner. That separate result does not replace this run's Completed=false/exit-1
or prove every post-audio shutdown path. No audio rerun or speculative production
Quit change follows.

After cleanup: owned MMDevice absent, no Moiren/probe processes, task Ready
with result1, render volume/mute and default roles unchanged, driver/boot
unchanged, and user OBS12648/start1793505454949and QQMusic15096/start
1793504036422unchanged. Two stalled read-only JSON helpers were identified by
exact PID/start and decoded command, then stopped; no unrelated PowerShell
process was touched. Raw PCM remains remote. Compact downloads are length/
SHA256-verified and the PCM/failure screenshots visually reviewed.

## Software checks and receipts

Fresh tests against the clean1267f8dexport, not dirty probe/VAD source:

```text
cargo test --locked -p moiren-windows-audio --lib zero_refill
  5 passed
cargo test --locked -p moiren-windows-audio --test latency_marker --test latency_marker_rate --test latency_qualification
  19 + 8 + 24 passed
cargo test --locked -p moiren-app --features gpui-test measured_ui_tests
  20 passed
```

All three commands exit0:76tests. This is focused regression evidence only;
the earlier workspace2171/8ignored and strict Clippy receipts remain in the
[identity increment](2026-11-02-process-root-identity.md), not a new full-suite
run for this documentation-only change.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Retained build/deployment/regression receipts use prefix `s1-vad-render-`.
Important payloads:

- `result-1102.json`:242552bytes, SHA256
  `B4BC58403608DDB0ECB2D081FB7E4E157B29281A001D544C2C19F5B20EBD0379`.
- `summary.json`:5967bytes, SHA256
  `C8FD7D9B738A79B8D441670D8DBF23E9F88FD3F3F542D87D7204BC50BA1A3BA5`.
- `functional-pcm.png`:223802bytes, SHA256
  `78AE6378DFF8B66CE14ADA514AED7F35FCF5FDDF976E4F373FE9712018254D74`.
- `final-1102.json`, `closure-1102.json`, before/running/retired UIA JSON,
  failure screenshot, frozen runner/launcher/analysis script, source and
  deployed-binary manifests. Original18dirty-file hashes remain unchanged.

Neither this result nor the earlier unmatched Process recording is a
same-condition VAD/Process latency A/B, sub10ms claim, default promotion or
proof that sub10ms is impossible. S1-S3 qualification stays incomplete.
