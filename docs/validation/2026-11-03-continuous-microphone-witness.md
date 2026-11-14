# Continuous Shared VAD microphone waveform witness

Date: 2026-11-03, Asia/Shanghai. Planning baseline: `de427d5`.
This implements an original-S1 closed-output analysis surface and acquires one
purposeful remote continuous-signal recording. It does not change DSP,
Compatible settings, quality gates, source mode defaults or driver behavior.
First-packet investigation remains deferred, and S1-S3 remains incomplete.

## Question and implementation

The preceding [sparse Shared MIC replay](2026-11-03-shared-microphone-frozen-replay.md)
matched every recorded sample, but 318720 frames were uninformative. Repeating
that sparse recording cannot establish informative waveform agreement across
those intervals. The existing input/output collectors already retain actual
PCM, so adding another native hook, larger capture or new RT path is unnecessary.

`scripts/vad-output-frozen.py` consumes an explicitly hash-pinned closed report,
collection, actual post-encode MIC witness/metadata and independent F32 recorder/
packet metadata. It reuses the unchanged `process-replay-frozen.py` association
and full-recording accounting, with no new correlation thresholds or DSP model.
Uniform unlabelled windows partition the whole recorder, independently of
signal amplitude. The first uniquely training-compatible window freezes one
integer offset and stereo gain/DC; later windows cannot search or refit it.

Only actual joined, completed-worker, current accepted WRITE records with
observed matching-byte completions and explicit terminal sequence/QPC fields
provide known comparison support. Failed/fenced/unresolved records retain their
bytes and coordinates as unknown; no packet compression or zero substitution
occurs. Zero-fill, worker outcome, excluded epoch packets, all recorder flags
and position adjacency deltas remain separate observations. Missing timestamps
do not become zeros, and echoed host WRITE QPC is never a sample clock.

Imported detached/embedded metadata, requested MIC ID and prepared/resolved
recorder identities must agree. A `matched` label alone cannot override a
sibling MMDevice/adapter/root identity. Imported identity agreement is still
not authenticated native execution. Nonfinite/clipped samples or a missing
anchor retain an explicit refusal and the complete unassessed denominator.
Every result remains unqualified with physical latency null.

The input manifest schema is `{"schema_version":1,"files":[...]}` with exactly
six entries, each containing `name`, `bytes`, `sha256`:

```text
report.json                 collection.json
vad-output.metadata.json    vad-output.raw.bin
measurement.f32le           measurement.packets.jsonl
```

Names are exact leaves; duplicate/unexpected entries, invalid hashes, size
changes, per-file/aggregate caps and duplicate JSON keys refuse. The CLI verifies
pins again after plotting, and requires a fresh output directory separate from
the input tree. It never opens audio, runs a source or issues device/UI actions:

```text
python -I -B scripts/vad-output-frozen.py --input-dir CLOSED_RUN --pins PINS_JSON --microphone-id STORED_BYTE_HEX --output NEW_OUTPUT
```

## Software verification

The new adapter begins with 16 genuine missing-implementation assertion REDs.
Subsequent regressions reproduce boolean-schema aliasing and forged sibling
recorder identity before repair. Independent review then identifies completion
labels without terminal sequence/QPC being treated as known support; a genuine
assertion RED precedes the mask correction. Missing stamps now remain unknown.
Visual review finds indistinguishable category colors; a separate plotting RED
precedes distinct category colors. Original data, statistics and first plot
remain unchanged; only a separate plot is generated with the color correction.

The final 24 tests cover PCM16 `/32768`, F32 signed zero, unknown/failed worker
support, packet coverage/shape, uniform windows, original flags/gaps, fixed-map
held-out corruption, missing anchors, nonfinite denominators, pinned input
mutation, complete CLI flow, imported identities and category colors. Existing
frozen-map 29, archive 25 and cascade 16 tests also pass. The remote isolated
environment runs the pre-color 23 adapter tests plus 29 frozen tests successfully;
the color-only change is separately tested on the host and used only for the
new plot, not a rerun or alteration of the completed numerical analysis.

The evidence-only native runner reuses the previous held-child collection helper
byte-for-byte. Its actual RED/GREEN and PS5/PS7 runs pass 124 pure/static checks
plus 7 post-catalog checks each; the parent independently repeats PS5. No
top-level native operation runs in these software tests. Source-mode/identity,
both joined packet ledgers, task ordering and functional/quality separation are
covered without creating an intent or inventing a binary in the worker.

## Exact single remote acquisition

Evidence roots:

```text
remote: D:\MoirenTest\20261103-continuous-mic-0750
host:   D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-continuous-mic-20261103-0750
```

The unchanged clean `50c97e5003ca28fa17fdb6156ca04647f32e3c03` probe is reused:
7612416 bytes, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
Its crates/driver/Cargo source has no difference from planning baseline
`de427d5`; preserved uncommitted overlays are excluded. No rebuild or fresh
full-workspace Cargo result is claimed for this Python/evidence-only increment.

The source is six contiguous seconds: 12 distinct indexed-v2 blocks of 24000
frames at amplitude 0.005, 288000 scheduled frames, starts 0..264000 by 24000.
There is no scheduled interior silence. Stereo duplicates are not independent
reference channels. Requested source mode is Shared; no period, low target or
recovery override is added. Route is Playback -> unity Bus -> MIC at 48k,
Engine F32/block64, unchanged Compatible settings. Both actual raw witnesses
are capped at 480000 frames. Recorder schema2 Shared/ring16384/PCM16MiB/
metadata2MiB/3000packets/preroll250ms/postroll500ms is unchanged.

Remote preflight finds no test owner, a healthy bare bus, matching protected
identities/driver and resources above unchanged 8GiB/15GiB guards. The fresh
root has distinct endpoint IDs. Limited Interactive session1 tasks are bounded
240s, with a directly held child90s plus existing bounded drains. A matching
completed selfcheck is required before native task registration; both cases
are create-once. No host audio/UI or external OBS/QQMusic is operated.

The selfcheck worker runs 07:57:21-07:57:29 +08:00 and ends task0. The one native
worker runs 07:58:34-07:59:09; probe15136 exits1 normally after21.6558s, no
timeout, forced termination or cleanup error. Native source is actually
`shared_vad_mmdevice`, F32/stereo, capacity1056. It acknowledges all288000
scheduled/nonzero frames; native Stop and worker join succeed with no source
failure or event timeout. Submitted frames are not delivery proof.

| Retained boundary | Shape | Frames | Packets |
| --- | --- | ---: | ---: |
| Actual Playback before decode | PCM16 stereo | 313440 | 653 |
| Actual post-encode MIC WRITE | PCM16 stereo | 331200 | 690 |
| Independent MIC recorder | F32 stereo | 329856 | 688 |

Both witness workers complete/join and seal at identity_changed. All690output
packets are accepted/current and observed completed with exact byte counts and
terminal stamps; bridge zero-fill and excluded epoch packets are0. These remain
WRITE/completion observations, not native consumption. Recorder missing/
incomplete packets, sequence gaps, nonfinite samples and ring drops are0.

**Original quality refuses:**3discontinuity flags at stored frames0/960/1056;
one position gap of1344frames at1056. Native error retains that failed
collection, so both collection_complete and runner functional_complete are
false. Preroll/source-completion/postroll and cleanup complete separately.
Worker/task0 means closed evidence audit, not native quality success. No first
packet is waived, no fault is attributed to Windows without evidence, and no
unchanged acquisition follows.

## Actual-output comparison and deterministic reconstruction

The finite isolated remote analysis exits0 in8.551s without forced cleanup.
Three initial uniform windows do not train; window3 freezes
`output_frame = recorder_frame + 1344`, both gains exactly1/DC0. Of41windows,
35are compatible; the initial3and trailing3remain refused, not hidden or
relabelled. The full-recording audit separately compares every frame:

| Category | Recorder frames |
| --- | ---: |
| Informative fixed-map compatible | 282432 |
| Uninformative signal | 41280 |
| Anchor-training category | 6144 |
| Residual/unsupported/outside/nonfinite/clipped/partial tail | 0 |
| Total | 329856 |

All329856stereo frames match the actual PCM16 output decoded to F32 bit-for-bit,
with zero mismatches and zero absolute/RMS error on the compared support.
The mapped interval `[1344,331200)` is wholly witnessed. This offsets stored
origins; neither1344frames nor its equality with the device-position-gap total
is a physical delay or causal loss conclusion. Silent prefix/tail intervals
remain uninformative even when their bytes match.

All attempted training intervals total16384frames, so raw nontraining comparison
covers313472frames, all equal. The6144anchor-training **category** excludes
training samples assigned to other categories; it is not the raw training
denominator. Independent review explicitly checks this distinction.

A separate finite offline replay uses the unchanged dual-FIR/codec helper,
SHA256 `87F629D562740DD7BF366859B09601A15239BE8746FAE4D6D645AB0D510B9E94`.
It reads actual input bytes and archived steps/cursors, then compares against
actual encoded output without search/fit. It validates5209callbacks per boundary,
replays5200 and stops before5201's first Playback barrier at timeline332800.
Startup21888Engine-zero frames and32explicit muted frames remain. It consumes
310981input frames, leaving2459unconsumed, and projects332915output frames.
All331200actual output frames match: mismatch_samples0, maximum PCM16 LSB0.
The1715projected FIFO-tail frames are not observed output and are not dropped
samples. Helper RMS statistics use different full-array denominators and are
slightly different; only the common331200-frame prefix is claimed exact.

Together these results explain input -> transformed encoded content -> recorded
content for this retained continuous-signal run. They do not independently
validate perceptual filter quality, certify sustained stability or establish
source/recorder clock alignment, physical latency or a VAD/Process winner.

## Closed artifacts and safety

Raw PCM and scalar chunks remain remote. Native compact ZIP2014106bytes,
62payloads, SHA256:
`9B821200C52D8BA41E3E51812EB02A91E0D69698991A19A02C2FBE6B200819A1`.
Analysis compact ZIP238599bytes,34payloads, SHA256:
`81377FD05C3A687693CAD09070132AE9833C122EA2FDB01E4881BEE80596E63D`.
Both manifests/every payload are length/hash verified, with duplicate/extra/
unsafe-path exclusion. Initial local ZIP verification refuses Windows separator
lookup before extraction; normalized entry lookup then verifies the same archive,
without remote regeneration or another acquisition.

`analysis-closed/actual-output-analysis/summary.json` is the original numerical
result. The original monochrome-category PNG remains retained. The separate
reviewed `analyzed/actual-output-frozen-colored.png` uses unchanged summary/raw
pins, SHA256
`09154E54C28A0AEDB9848C7C7BDA6C0A2CADA0DE33BB745E67EC2CD22F55E81C`.
It shows continuous recorded energy, preserved early flags/gap and complete
frame categories; a color correction does not retroactively change analysis.

Final08:07:29+08:00 audit rechecks62closed native files,6analysis inputs,
6cascade inputs,13deployment pins and binary. Both tasks are Ready/0; no test
owner remains, both exports are removed and only the healthy bare bus remains.
Protected OBS12648/QQMusic15096 creation identities, default roles and render
volume/mute remain unchanged. Process identity does not prove uninterrupted
user audio. Driver Running/on-disk hash and boot remain unchanged; no new
loaded-image measurement is claimed. All27preserved dirty/cache hashes remain
unchanged. No host process is stopped, no push occurs, and no background soak
is left running. Independent S2/S3 work and purposeful remote testing remain
parallel-capable; physical/retained-desktop acceptance stays open.
