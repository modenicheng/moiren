# Target1280 Frozen Recorder Association

Date: 2026-11-03, Asia/Shanghai. Production source `2dd538e`; evidence-only
parent `9821b2f`. This analyzes the already-closed
[single target1280 acquisition](2026-11-03-process-target1280.md).
No new audio source, application, OBS, driver exercise or recording starts.

## Question And Frozen Plan

Does the actual encoded output retained by each immutable owner associate with
the independent full OBS recording under the existing fixed model? The native
acquisition's storage/counter pass did not answer this waveform question. This
recording has not previously been fitted. Older inconclusive recordings are
not reopened or refitted.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-target1280-20261103-1520`.
Remote files stay under `D:\MoirenTest\20261103-target1280-1520`.

Before inspecting waveform values, `waveform-package/analysis-plan.json` pins
training source start24000, length4096, and OBS search intervals
182400..230400 for baseline,283200..331200 for replacement; intervals are
half-open. The sole selection basis is OBS Recording Start15:27:30.531 and app
owner start15:27:34.170686/15:27:36.297222. These wall-clock log times select
windows, not exact audio alignment or measured delay. Both4096-frame nominal
training supports lie inside their declared one-second search windows.

The plan SHA256 is
`7756585A08DB524BDCE177451F371E90E6FB250A3638BC38286BB965176A3EF2`.
Independent read-only review finds no launch-safety, coordinate or evaluation
contamination blocker. The full manifest copies these exact training values
and binds separate owner ordinals and original native-render directories.

The prior analyzer and its tests are reused byte-for-byte. It fits one integer
lag plus fixed per-channel gain/DC independently per owner, then freezes them.
Minimum correlation0.9999, peak margin0.01, gain range and absolute residual
limit2/2^23 remain unchanged. Both complete search intervals are excluded from
both evaluation ledgers, including unsuccessful candidate positions. There is
no FIR replay, SRC/interpolation, channel permutation, local re-alignment,
post-result window adjustment or evaluation-driven tolerance. Archive loss or
inconsistency refuses the whole owner association, rather than salvaging a
prefix. Rejected native submissions remain in the denominator and cannot be
evaluated as accepted output.

## Extraction And Device-Free Regression

The existing isolated remote analysis environment is reused, with Python3.10.11,
NumPy1.25.2, SciPy1.9.3 and matplotlib3.7.5. No global installation occurs.
Finite held processes run once with180s outer and15-45s child limits, separate
logs and retained handles. They read closed files only; no audio device is opened.

The unchanged analyzer's28synthetic tests pass in24.548s before actual analysis.
Expected negative-test refusals remain in the original test log, not hidden as
errors in the actual acquisition. Both Python and extraction tool hashes are
recorded and checked. There is no native test or full workspace rerun for this
documentation/closed-analysis increment.

The original MKV is13437558bytes, SHA256
`4CB14FAA7629B63713796E936076BBA128B2C986597D57AB89C214D96016EBD3`.
ffprobe identifies one audio stream:48kHz stereo PCM24. ffmpeg stream-copies
that entire encoded audio stream without trim, sample conversion, downmix,
resampling or inserted frames. Raw PCM has1071104frames,6426624bytes; the
complete recording denominator is retained for each owner's independent ledger.
Container video is not used as a physical reference clock.

## Frozen Result: Residuals Remain

Both training associations succeed uniquely and their full OBS supports are
disjoint. Baseline integer coordinate offset173740frames, replacement277772;
these offsets are **recorder sample-coordinate associations, not physical
time-of-flight or latency**. Correlations are approximately0.999999999963,
peak margins0.72155/0.72330. Training gain is approximately1, DC approximately
-5.9e-8, maximum training residual approximately6.1e-8, within the unchanged
two-PCM24-step limit. High training correlation is not an evaluation pass.

The independent full-owner source ledgers are:

| Owner | Full native frames | Unknown | Residual | Compatible |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 97120 | 48000 | 1179 | 47941 |
| Replacement | 385408 | 48000 | 98282 | 239126 |

Each row exactly partitions its full native archive. Unknown includes the
entire search-contaminated support, not a cropped-away denominator. Compatible
means only pointwise agreement under the frozen model within2/2^23; it is not
a general quality assertion. Residual means disagreement with that model and
recording boundary; it does not identify dropped/duplicated frames, audible
distortion, a driver defect or which component is responsible.

The full1071104-frame OBS ledgers also partition independently:

| Owner association | Unknown | Residual | Compatible |
| --- | ---: | ---: | ---: |
| Baseline | 1021984 | 1179 | 47941 |
| Replacement | 733696 | 98282 | 239126 |

No owner splicing, conditional scoring or conversion of unknown to a pass
occurs. Actual native archive layout, packet HRESULT and byte accounting are
complete with no rejected submission, as recorded previously; successful
ReleaseBuffer does not establish when or whether those samples were presented.
The new waveform evidence therefore does **not** establish continuous clean
output, even though reported Process steady fault counters were0. The3gap
violations and1unlocated master wait timeout from acquisition remain unchanged.
No causal association with residuals is established.

The eight-panel `waveform-once/association/alignment.png` is inspected. It shows
the entire recording, both excluded searches, whole-denominator categories,
the fixed first post-search512-frame view and frozen residuals. It explicitly
states no quality/latency pass. No search, lag, gain/DC or threshold is refitted.

## Container Coordinate Diagnostic

A distinct read-only follow-up tests whether obvious Matroska packet timestamp
jumps explain concatenated raw PCM displacement. It does not transform the
waveform, change the association or re-evaluate residuals.

All1046audio packets are enumerated with exact integer PTS and rational
timebase1/1000 (1ms timestamp resolution, not packet cadence). Every packet
contains1024frames; adjacent PTS increments are21or22ms. Packet bytes sum
to1071104stereo PCM24 frames, matching the
whole stream copy exactly. PTS-minus-concatenated-frame coordinate stays within
16frames absolute; successive timestamp gaps range-16..32frames. There is no
jump exceeding two timebase ticks. Final timestamp end is1071088frames versus
1071104encoded frames, a16-frame timestamp quantization difference.

This rules out a large jump in the observed container timestamp map; it does
not prove the capture path had no sample discontinuity, establish sample-clock
calibration or explain the frozen-model mismatch. The diagnostic plot is
inspected alongside the original unmodified residual block shares. No sample
insertion, deletion, drift fit, second waveform association or audio rerun follows.

## Closure, Transfer And Scope

All228original acquisition input pins and analyzer software pins rehash unchanged.
The waveform compact ZIP498888bytes, SHA256
`C16A1161969DA1A674DAB97C999876C5B8414FDF7EB963E0A5EFBAF9FCAF3965`,
has16payloads length/hash-verified after transfer. The packet diagnostic ZIP
213794bytes, SHA256
`400275DBF878C3DCEEB2BF71A52C7229B1697182100394210D5BC6E0427CBAFA`,
has7payloads likewise verified. Raw MKV, native PCM and extracted OBS PCM stay
remote. Analysis results are new files; prior evidence is never overwritten.

At15:57:03 final remote audit verifies261pin entries, unchanged app binary,
protected user OBS/QQMusic identities, driver/hash/boot, render defaults/format/
volume/mute and bare control bus. No analysis/audio owner or running Moiren task
remains; the original native task stays Ready/result0. No cleanup is needed.
Independent final read-only review confirms denominator closure, extraction
and provenance, and corrects timestamp-resolution wording without changing
the diagnostic. It finds no evidence sufficient to specify a product-code fix.

The original S1-S3 goal remains incomplete. This is concrete waveform evidence,
not a calibrated latency result, successful continuity gate, default promotion
or new product-code fix. Do not infer a production cause from an integer-lag
residual or modify code without a localized fault. Deferred first-packet work
and independent product-development priorities remain unchanged.
