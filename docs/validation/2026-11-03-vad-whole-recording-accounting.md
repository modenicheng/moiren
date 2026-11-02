# Entire VAD recorder denominator with one frozen map

Date: 2026-11-03, Asia/Shanghai. Baseline: `763118e` on
`feat/minimal-io-latency`. This continues the
[archived Playback diagnostic](2026-11-03-vad-playback-frozen-replay.md), without
another native recording, parameter scan or unchanged stability soak.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-frozen-coverage-20261103-0615`.
Remote computation root: `D:\MoirenTest\20261103-vad-frozen-coverage-0615`.
Inputs remain in the two preceding waveform/replay directories; raw PCM and
scalar archives stay remote.

## Why the additional accounting is necessary

Six compatible informative pulse windows are not the entire recording.
The committed `process-replay-frozen.py` now provides a bounded all-frame audit
using the existing immutable owner, integer shift, generation and channel
gain/DC. It cannot call association/search, move an origin, interpolate,
adjust rate/phase, refit or rewrite inputs. Existing association behavior and
thresholds remain unchanged.

Every decoded recorder frame stays in the denominator. A contiguous partition
retains outside-replay regions, unknown FIR support, generation mismatches,
nonfinite/clipped samples, silence, anchor training, informative residuals,
unsupported block context and partial tails. Complete 64-frame blocks use
unchanged NCC/NRMSE gates only when informative and supported. Training and
silence do not become independent informative passes. Detail exceeding4096
runs refuses instead of truncating the denominator.

Raw stereo representation equality is separate from numerical equality and
fixed gain/DC residuals; signed zero and encoding differences remain visible.
Nontraining counters exclude the outer quarters of every attempted training
window up to and including the selected anchor. No global quality, marker
identity, native master-submission witness or source/physical latency follows.
The master stage remains a nominal projection because the prepared route and
native submission witness are absent.

The existing finite held-child runner is reused byte-identically. Computation
has210seconds, final input/safety audits reserve time within285seconds, and
the single scheduled task is bounded at300seconds. Audits execute on both
success and failure. No audio/UI/player/OBS/driver configuration is launched
or changed on either machine. All27 preexisting dirty/cache files are excluded
from the clean source export and this slice.

## Software verification and provenance

The new full-recording API has a genuine behavioral RED before implementation:
the audit incorrectly reports zero training frames versus256. Follow-up tests
cover immutable input/anchor behavior, signed zeros, encoding mismatch, unknown
and generation support, nonfinite data, both shift signs, chunk boundaries,
tails, training-context conservatism and actual4096-run refusal.
Final focused suite:29 passed, comprising17 audit cases and12 original cases.
The original associate/search/evaluate implementations and gates are unchanged.

An independent clean `763118e` export receives only the two intended script
files. Five relevant suites pass14+32+21+29+26=122tests. Three pure orchestration
and adapter suites pass17+16+3=36tests. Test I/O and process launches are mocked
in orchestration tests. The initial broader local regression correctly fails
because the new isolated Python environment lacks matplotlib; after installing
and pinning numpy1.26.4/scipy1.12.0/matplotlib3.8.4 in that environment, the
entire five-suite run passes. No global Python environment is changed.
The remote analysis retains its preexisting numpy1.25.2/scipy1.9.3/matplotlib3.7.5
environment; host software passes do not substitute for its actual execution.

Independent helper and wrapper reviews find no material issue in the final
pinned snapshot. The deployed helper/test SHA256 values are respectively
`BAF2F0FE44F05D748E1924A622403021DA95299513BEFC4D3A94159B1EDF1F3D`
and `CB17F0B4A1901A67442E5A972620537A3FDEFEBAD16459E161FFBD11ED0D7D7C`.
The13-entry deployment is37945bytes, SHA256
`EDB37D6A8677F4E34AE26FE9B58D9CC73A32958A32890684F26EFED7F135F0E6`.
Seven original raw/projection/mask/generation/summary inputs are length/hash
pinned before deployment, including the prior frozen association report.

## Entire actual recording

The single remote task finishes Ready/result0 in28.281seconds; all nine held
children exit0 without forced termination. The pinned remote runtime passes
29 frozen-helper,17 orchestration,16 held-runner and3 adapter tests. Analysis
completion means full accounting, not an audio-quality PASS. Both unconditional
final audits succeed and all seven original input hashes remain unchanged.

The fixed map partitions all874496 stereo recorder frames into25 contiguous
runs. No window crop, omitted interval, padded tail or new fit is used:

| Category | Recorder frames |
| --- | ---: |
| Outside the retained replay | 487248 |
| Unknown replay/FIR support | 5615 |
| Supported but uninformative signal | 372416 |
| Independently informative fixed-map compatible | 8192 |
| Anchor-training samples | 730 |
| Strict 64-frame waveform-residual blocks | 256 |
| Unsupported/mixed block context | 39 |
| **Total** | **874496** |

Generation mismatch, nonfinite/clipped input or recording, and partial-tail
categories each contain zero frames in this recording. They remain supported
refusal/accounting categories, not suppressed checks. The earlier unknown
stereo transient is outside this frozen replay interval; it is neither erased
nor retroactively explained by the later pulses.

Known support is381633frames. Of these,372649 stereo frames are representation-
equal to the raw nominal projection and8984 are not; most equal frames are
silence, so this is not evidence for continuous informative delivery.
Nontraining known support is380826frames, with372535 bit-equal and8291 unequal.
Applying only the already-frozen gain/DC yields channel maximum absolute
errors6.704e-8 and7.170e-8 across all known finite unclipped support, including
silence/training/tails. No independent parameters are estimated by this audit.

Four complete64-frame blocks still fail the unchanged informative gates at
recorder starts417472,465472,511936 and559936. These256frames remain residuals,
despite the small absolute error. Whole-pulse compatibility does not erase a
short-window refusal, and small error alone neither identifies its cause nor
establishes a perceptual or physical bound. No lower gate, production fix,
reanchor or rerun is introduced to turn them green. The128 independently
informative compatible blocks are not a global waveform-quality PASS.

The remotely generated `archive/output/all-frame-coverage.png` is verified
after transfer and visually inspected. It shows the entire stored recording,
the earlier unknown transient, six pulse regions and the full classification
strip. The24 payload files, excluding the manifest, are92289bytes, SHA256
`FB241F026454B158BB593F80BDAA59DFA016C69B599682B10C57626D6B6CAF83`.
Every included entry has its exact manifest length/hash and no extra entry;
raw audio is not transferred.
The independent post-transfer audit rechecks all seven original inputs, all
deployment pins, the compact archive, protected OBS/QQMusic creation identities
and loaded driver hash/boot. No owned audio/replay/analysis process or running
Moiren task remains. All27 original dirty/cache hashes remain unchanged.

## Stopping boundary

This closes the selective-window accounting gap for this archived diagnostic,
not native or physical qualification. The missing prepared route/native
submission witness, outside/unknown regions and strict local refusals remain
explicit. S1-S3 is not complete, lower defaults are not promoted, and the
user's unavailable multichannel physical equipment is not a development
blocker. First-packet work stays deferred. Do not repeat this unchanged
recording or add another fit; proceed with independent application work and
only newly justified, finite, isolated remote tests.
