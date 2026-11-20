# Shared-source waveform recordings and analysis boundaries

Date: 2026-11-03, Asia/Shanghai. Repository baseline: `54790e7` on
`feat/minimal-io-latency`. These are finite remote acquisitions and offline
analysis, not another stability soak or an S1-S3 acceptance PASS.

Local evidence roots, under
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`:

- Process: `s1-matched-waveform-ready-20261103-0505`.
- Separate VAD-only attempt: `s1-vad-waveform-joined-20261103-0515`.

Their remote roots are respectively
`D:\MoirenTest\20261103-matched-waveform-ready-0505` and
`D:\MoirenTest\20261103-vad-waveform-joined-0515`.

## Frozen route and acquisition scope

The clean probe is built from `50c97e5`, 7,612,416 bytes, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
The application remains the clean `e4de0d7` package, SHA256
`1217C07CAB24BAFF8B57E6E7BC5536C82B4F9A7F913892389B0B50F922D0B128`.
No production application, driver, default or buffer policy changes here.

Both source MMDevices actually negotiate Shared480, 48 kHz stereo F32,
native capacity1056, Client3 without Windows auto-conversion or fallback.
Both graphs use unity Bus and the same DAWN PRO2 output, Shared336,
native capacity738, render target512/wake64/callback64, Engine F32/block64
and unchanged Compatible input policies. Source paths remain different.

The deterministic source releases288000 scheduled frames with1024-frame
indexed codes at12000/60000/108000/156000/204000/252000. Source release
completion does not prove playback, capture or continuous delivery.

Return recording uses isolated portable OBS WASAPI output capture of DAWN,
monitoring disabled, gain1, offset0, one lossless stereo48k PCM24 track.
It is not a DAC, cable, microphone or calibrated physical measurement.
Stereo duplicates are not independent reference/return measurement channels.

Actual VAD raw provider input is **Pcm16**, despite the source MMDevice's
F32 negotiation. Metadata format1 and1537920 bytes for384480 stereo frames
establish this boundary; format2 would denote F32. Do not claim a bit-exact
route, identical native transfer formats, the location of conversion, or a
faster source path from the common source negotiation.

## Process: data retained, pair not completed

The scheduled native task ends Ready/result1. The source27196 releases the
full schedule, stops and joins with exit0. Application24408 runs12001ms,
joins cleanly, exit0. Independent reference recorder24552 retains430080
F32 stereo frames and896 packets, recording_complete/artifact_persisted true,
without a recording failure; its unqualified probe exit1 is retained.

OBS2200 reaches recording active, Stop and inactive. Acquisition data are
complete, but WM_CLOSE does not join within12s and the exact held child is
terminated, exit1. Therefore route complete=false and the sequential pair
stops without launching VAD. Later VAD evidence does not upgrade this pair.

OBS logs show recording Stop05:06:12.559, websocket module unload05:06:12.765
and a closing-handshake timeout05:06:17.766. A new evidence-only runner closes
the owned websocket after confirmed recording inactivity, before WM_CLOSE.
Unresolved recording transitions retain connection ownership through cleanup.
Two regression failures become15passing pure checks. The executed Process
runner and its failed cleanup evidence are not overwritten or rerun.

Joined Process counters report zero drops, silence, nonfinite samples,
discontinuities, timestamp errors, underruns, resets and recoveries.
Startup priming3904 and prime discard352 are separate. Its queue history
contains9023 events with8767 overwritten: it is not a complete owner replay.
These counters alone do not establish waveform correctness.

## Separate VAD: normal cleanup and retained raw witness

The VAD-only native task ends Ready/result0, complete=true. Observation spans
05:16:43.950-05:17:26.603. Graph25604 retains its expected unqualified exit1,
native_error=null, full source schedule, normal source Stop/join, Engine Closed,
no fault/cleanup error and VAD Removed. OBS20216 exits0 after inactive,
websocket close and WM_CLOSE; no forced termination occurs.

The decode-before-carrier raw witness stores384480 stereo Pcm16 frames in801
480-frame packets, all published, with no excluded epoch/stale packets.
Its worker completes/joins and seal_reason is identity_changed. This is not
an Engine-output or physical-output recording.

The last Running Engine snapshot has target2448/capacity4800, xruns0,
short-read callbacks0, initial-prime muted32 and recovery-prime muted0.
The post-close checkpoint separately retains boundary epoch1 and xruns1;
provider post-close telemetry retains epoch_resets2/recovery_events2.
Do not erase these terminal observations or label them steady-state faults
without phase evidence. No continuous waveform-quality PASS is claimed.

The actual SRC scalar history exports6062 checkpoints, history_complete and
stream_complete true, to `run/vad/graph/src-trace.json` and its chunk stream.
Raw PCM and scalar streams remain remote. They have not been replayed here.

### Fresh offline scalar readback

A subsequent finite180s remote task reads this already-closed history without
launching audio or another acquisition. It uses the committed `54790e7`
archive/phase tools, exported independently of all dirty overlays. The actual
remote archive tests pass25 and phase tests pass16; task ends Ready/result0.
Source summary/chunk lengths and hashes remain unchanged across readback.

The archive validates all6062 callbacks in3 chunks/4468035 bytes, zero missing
or unretained callbacks, and writer_finished=true. There are39 unavailable
polls, not39 missing callbacks. The phase validator admits one continuous
scalar segment, sequences89-6051 inclusive,5963 callbacks. Sequences1-87 have
no processing step;88 transfers64 with32initial-prime muted frames;
6052 crosses a boundary epoch and6053-6062 have no processing step. Retained
xrun/discontinuity flags appear on98 callbacks; these are not98 distinct
underrun incidents. No startup or terminal observations are deleted.

The admitted segment's processing step spans1.0000066327520205 through
1.0002720510787084. This is SRC diagnostic state, not measured audio latency,
waveform integrity or proof that the unresolved markers are harmless.
The archive has no prepared-route stamp, so no route or source-to-recorder
mapping is manufactured. The new `src-scalar-diagnostic.png` is remotely
generated, verified after transfer and visually inspected.

The separate read-only session1 desktop observation ends Ready/result0,
audio_launched=false and ui_action_sent=false. The input desktop is Default,
but the foreground is the Windows default lock-screen interface. No new
retained-prefix desktop attempt, key/click, unlock or foreground manipulation
is performed. Service acceptance is not substituted for the missing desktop
result. Unlocking the remote interactive session is the prerequisite for a
future unobscured owned-window acceptance attempt.

## Analysis and real figures

The unchanged nominal Rust full-six-code matcher retains thresholds
correlation0.7, peak margin0.05 and RMS0.0001. No threshold reduction,
identity-by-order, SRC fitting or cross-origin latency subtraction is used.
Both channels give the same qualified identity sets:

| Boundary | Stored frames | Unique identities | Unresolved identities |
| --- | ---: | --- | --- |
| Process independent reference | 430080 | 0,1,2,3,4,5 | none |
| Process OBS return | 849920 | 2 | 0,1,3,4,5 |
| VAD raw input reference | 384480 | 0,1,2,3,4,5 | none |
| VAD OBS return | 874496 | 0,1,3,5 | 2,4 |

Nominal detector refusals are **not lost-frame counts**. Several return peaks
have high whole-code correlation but fail section consistency. VAD also has
an earlier unrecognized nonzero transient, with different stereo samples;
its provenance is unknown. It is retained, not assumed contamination, an
audio defect, or an ignorable startup event. Independent stored origins have
no established source-to-recorder mapping, so no latency is reported.

The first Process analysis fails before output because Process metadata has
integer `packets`, whereas VAD metadata has a packet list. A new v2 analyzer
corrects only schema handling, passes24synthetic tests and completes remote
analysis with unchanged input hashes. The VAD v2 return analysis is preserved;
its F32-only reference decoder correctly refuses the Pcm16 raw witness.
New v3 supports declared little-endian signed i16 /32768 decoding without
resampling, gain fitting or threshold changes: four new REDs become28passing
tests. Remote v3 analysis finishes result0 with unchanged input hashes.
Admitted correlation work is21368832 for Process and439762944 for VAD, below
the unchanged aggregate1e9 cap.

Real reference/return PNGs are generated remotely, transferred with verification
and visually inspected. They show the six reference identities and nominal
return refusals, using separate recording-frame axes rather than a fabricated
common clock. Local paths relative to the roots above:

- Process: `analysis-archive/analysis-v2-output/{reference,return}-waveform-identities.png`.
- VAD: `archive/analysis-v3-output/{reference,return}-waveform-identities.png`.

## Provenance and stop rule

The isolated native Python environment's33-file non-pyc SHA256 fingerprint is
`CD24FA8C556890903C8D023CFB6F681E04DC0281631FAC3437D08F4893B5CC95`.
Analysis uses the existing isolated numpy1.25.2/scipy1.9.3/matplotlib3.7.5
environment, with executable and module ancestry verified. Compact archive
length/SHA256 verification preserves directories and all included entries
(the file counts exclude each archive's own manifest):

| Archive | Files | Bytes | SHA256 |
| --- | ---: | ---: | --- |
| Process preanalysis | 11 | 23842 | `FABC02FB25CFFF6CFEF9E3C5A67016812BC420371707125B925C48580A6FB9D6` |
| Process analysis | 18 | 240435 | `9BE08D814A9A8FE0621822A9F6834D564D7350F4877ECBB5BD8D17121A3B4696` |
| VAD compact evidence | 25 | 286245 | `18F0B135903C483680609FC5641F471E4DC836FD1EEA163351C4B2C3A146959A` |
| VAD scalar readback | 18 | 120498 | `6624F63C680E0E94E8B234E8353677F6E92AC5493B7C9CD8912537009B389023` |

All native testing stays remote. Only exact held owned processes are cleaned
up; external OBS12648 and QQMusic15096 retain their creation identities.
Both acquisition audits have no remaining owners or audit_error, unchanged
driver hash/boot, default roles and render settings. All27 original dirty/cache
files remain excluded from builds and commits.

Fresh `final-fresh-audit.json` independently rechecks terminal task results,
protected creation identities, no test audio owners, driver hash/boot,
default roles, render settings and compact archive hashes. Its first
read-only invocation refuses a null optional catalog `after` phase before
writing an output; the corrected audit requires matching null phases and
compares the populated `before` inventory. No audio test is relaunched.

S2/S3 software implementation status does not acquire a new gap from these
recordings. Ordinary measured desktop owner replacement already has native
evidence; retained-prefix desktop Stop/join -> Evaluate -> conditional Apply
remains open, as does physical qualification. First-packet work stays deferred.
Do not repeat unchanged soaks, relax conservative gates, enlarge buffers or
claim a winner. Isolated finite remote tests may run alongside independent
development when answering a new concrete question; absent multichannel
physical hardware does not block that development.
