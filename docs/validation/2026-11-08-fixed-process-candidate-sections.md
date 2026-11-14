# Fixed Process return candidate: section refusal

Date: 2026-11-08, Asia/Shanghai. This follows the
[matched waveform checkpoint](2026-11-07-matched-waveform-checkpoint.md).
S1 correspondence and S5 quality/latency acceptance remain open.

## Retained inputs and scope

The November7 frozen Process return PCM is
`D:\MoirenTest\20261107-matched-waveform-2255\analysis-v4-output\process-return-decoded.f32le`:
5,570,560 bytes, 696,320 stereo frames, SHA256
`BE807CBBCFC117881474C804C370473C7B2DC25004144BAFA4F09D988A58BF05`.
The bounded read-only target extraction verifies the full file hash before
and after reading the six already selected 1,024-frame candidates. It retains
49,152 bytes of stereo Float32 PCM, not another recording or onset search.

Evidence root:
`D:\coding\moiren-evidence\20261108-094031-pipeline-next`.

| Artifact | SHA256 |
| --- | --- |
| `frozen-process-return-fixed-sections.json` | `B9F3190823B1AECF2BAC6128F8FBF70CF2D790A0A21F6459EBE89395B41DD97E` |
| `fixed-section-moments.json` | `A329E86FE1290696085D4AAFC0296E330ED6FA7504BD88120D663A98D92BABA6` |
| `scripts/indexed-section-diagnostic.py` | `404D41EF162079EBA648E6272C66C3A0FF010FE5F7C8083225AB17509218F92A` |

At09:55, `fixed-section-moments-rechecked.json` records independent recomputation
of all12 retained channel/candidate rows. Every full confidence agrees with
the retained Rust detector within1e-12 and every section verdict agrees
exactly. Global means, Float32 input samples, four equal sections, correlation
threshold0.7 and RMS threshold0.0001 remain unchanged. There is no resampling,
phase/gain fit, threshold relaxation, native audio operation or latency claim.

## Identity1 candidate

Both channels retain the same candidate at recording frame178607.
Full confidence is0.7997766973012247, but the fourth quarter fails the
independent section gate:

| Quarter | Frames relative to candidate | Aligned correlation | Recording RMS | Section gate |
| --- | --- | --- | --- | --- |
| 1 | 0..255 | 0.9073679897061487 | 0.0186436255468082 | Pass |
| 2 | 256..511 | 0.8617470437187025 | 0.018589048435231972 | Pass |
| 3 | 512..767 | 0.7489241864605517 | 0.01700633637094741 | Pass |
| 4 | 768..1023 | 0.6728417113392814 | 0.017758805492012246 | Fail |

All four RMS values exceed the unchanged minimum. This narrows the recorded
refusal to waveform disagreement in quarter4 at the retained onset; it does
not identify a silent quarter, establish that the source marker was lost,
attribute the distortion to a particular component, or prove another onset
would qualify. The detector's `missing` status and `sections_match=false`
remain unchanged. Identities0,2,3,4,5 keep their original accepted onsets.

The next correspondence investigation must explain the retained waveform
under the declared capture path. This diagnostic cannot substitute for an
accepted matched pair, VAD's direct-Removed cleanup gate, or calibrated
physical measurement. The historical OBS-copying acquisition runner remains
retired.
