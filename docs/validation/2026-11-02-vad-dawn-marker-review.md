# Existing exact336 VAD/DAWN marker identity review

Date: 2026-11-02, Asia/Shanghai. Source probe: clean `1267f8d`.
This is one remote offline review of the closed
[VAD -> unity Bus -> DAWN functional captures](2026-11-02-vad-dawn-exact336-functional.md),
not another native recording, stability scan, lower-target experiment or
production change. The original failed native Quit verdict remains unchanged.

## Result and next-action boundary

Both source channels uniquely resolve all 19 supplied windows as original
indexed-v2 identities 0 through 18, with best NCC exactly 1. Both graph-output
channels resolve 16 of their 19 supplied windows. Three remain missing under
the unchanged nominal-template detector; no identity is assigned from energy
order. There are no ambiguous windows or duplicate resolved identities across
batches. This advances the earlier energy-islands-only evidence without
turning it into a continuous waveform or physical latency PASS.

| Capture | Channel | Windows | Unique | Missing | Ambiguous |
| --- | ---: | ---: | ---: | ---: | ---: |
| Source process output | 0 | 19 | 19 | 0 | 0 |
| Source process output | 1 | 19 | 19 | 0 | 0 |
| Moiren graph output | 0 | 19 | 16 | 3 | 0 |
| Moiren graph output | 1 | 19 | 16 | 3 | 0 |

The missing graph windows' strongest candidates are not accepted identities:

| Strongest candidate code | Channel0 NCC | Channel1 NCC | Refusal |
| --- | ---: | ---: | --- |
| 2 | 0.736639564 | 0.736627210 | Four-section coverage fails |
| 9 | 0.695015954 | 0.695006048 | Global floor and section coverage fail |
| 12 | 0.711206707 | 0.711188095 | Four-section coverage fails |

Source capture contains the exact codebook, whereas graph output does not
uniformly match an unresampled nominal template. This alone does not identify
the cause as actual loss, SRC, servo, filtering, duplication or a recorder
effect. Do not call these three dropped markers, lower the correlation or
section gates, or alter production behavior based on this evidence alone.
First-packet investigation stays deferred. No additional native run follows.

## Exact unchanged diagnostic

The existing deployed `analyze-indexed-pcm` command is used without a source
or binary change. Every window competes against all 59 exact source codes.
The original schedule is neither truncated to the visible 19 codes nor
renumbered according to detected-peak order. Unobserved schedule codes include
unrecorded time and the source's deliberate early stop, not proved drops.

Windows use only each channel's exact nonzero support: split after more than
32 zero frames and pad 64 frames on both sides, following the
[existing identity review convention](2026-10-31-indexed-marker-analysis.md).
No energy-derived identity labels or source/recorder clock origin are supplied.
The nominal48k/stereo/F32 detector retains correlation0.7, peak margin0.05 and
RMS0.0001; global NCC, four-section coverage, codeword and onset competition
remain enforced. All input PCM is validated before window search.

Source channels each fit one batch; graph channels each require three. Each
batch stays under the existing 256,000,000 Cartesian-sample cap. All four
channels together charge 1,669,837,824 samples under the separately predeclared
7,000,000,000 total cap. Complete ordered windows are accounted for across
batches, and accepted identity duplicates are checked globally. Separate-batch
reports do not establish whole-schedule identity completeness.

There is **no retained per-packet JSONL timeline** in these Process recordings.
Their actual `capture.metadata.json` is aggregate schema2 metadata, not a
packet timeline. It is passed unchanged to the detector, which explicitly
retains `invalid_packet_metadata` and its parser error. No synthetic timeline,
timestamp interpolation or silent aggregate-to-packet conversion is created.
All eight detector commands exit1, and every raw report retains
`identity_set_complete=false`, `qualified=false`, and unknown native epoch.
The identity component still searches the admitted PCM after that explicit
metadata refusal; this is verified from the detector source, not inferred
from a successful wrapper exit. Wrapper exit0 means the finite analysis and
artifact generation completed, not that the recording passed quality gates.

## Remote execution and visual output

Target: `LAPTOP-NP49RG6H`. Lifecycle: `task`, parent ceiling330s, child45s,
internal work deadline255s. The one owned Python process is PID26592 /
creation FILETIME134380540377977581. Actual completion08:47:25.198; analysis
elapsed5.125s. Parent exit0, stderr empty. No app, source, recorder or device
is opened. Existing isolated Python3.10 environment uses `-I -B`, single-thread
numeric settings, output-local matplotlib cache, and verifies NumPy/matplotlib
imports remain inside its prefix. No package install or shared environment edit.

Remote output:
`D:\MoirenTest\20261102-vad-marker-review-0848`.
`marker-identities.png` shows accepted/refused NCC, only resolved code indices,
and complete 10ms RMS tracks. Independent stored-frame axes are explicit;
there is no cross-recorder subtraction or physical reference. The plot is
visually reviewed. Raw PCM stays remote; compact JSON/config/results and PNG
alone are downloaded and length/SHA256-verified.

Every original PCM, metadata, report, source schedule, native result and probe
is hashed before/after unchanged. A final remote read-only audit confirms the
owned Python and all Moiren/probe processes absent. User OBS12648 /
start1793505454949 and QQMusic15096 / start1793504036422 remain unchanged.

## Verification and receipts

Fresh unchanged-detector regression against the clean export, excluding all
18 preserved dirty paths:

```text
cargo test --locked --quiet -p moiren-windows-audio --example e2e_latency_probe --features vad-probe indexed_pcm
  45 passed, 0 failed, 616 filtered out
```

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Intention, runner, launcher, completion, detector tests and final audit use
prefix `s1-vad-marker-`. Downloaded compact outputs are in
`20261102-vad-marker-review-0848` under that root.

| Payload | Bytes | SHA256 |
| --- | ---: | --- |
| `summary.json` | 71936 | `51B727938AFC5323AB2F92F2849BA2D900D883218570C42BFD9213B93E6EB8AB` |
| `marker-identities.png` | 186141 | `365F20E65C4B19604C87068C35D0333C0F01280D0CB2DCBB805062A8CC47289B` |
| `artifact-manifest.json` | 5986 | `22BE993214C155A12F73F4AB94F2E9D2E9616677B8AB97949C18375ADFEA17E8` |
| Each before/after input-hash JSON | 1513 | `895E271D47D0F7A5B0D0164C5208CAECEF547B5BE1D95279EE7AFE43AADA9926` |

Runner SHA256:
`8E40EB3D8110BE39EF01A32080B7B530D25D02558CE9CB86F1E7FEC17219E6CF`.
Original probe SHA256:
`3C7D74309CF0EC2C846D34969F8B3D1FC1EB830A29E713666EE0A6B6AE7310EE`.

No production, driver, default, gate, cache-directory or unrelated-draft
change. No push or memory update. Full waveform continuity, matched VAD/Process
comparison and calibrated physical qualification remain open; S1-S3 remains
active without blocking unrelated development on unavailable physical gear.
