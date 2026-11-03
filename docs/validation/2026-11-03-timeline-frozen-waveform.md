# Timeline Recording: One Frozen Waveform Association

Date: 2026-11-03, Asia/Shanghai. This is one offline association of the closed
[native timeline acquisition](2026-11-03-render-timeline-native.md), not another
audio run. Analysis completes, but residuals remain unexplained: no waveform
continuity, audio-quality, timing or physical-latency PASS follows.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-timeline-waveform-20261103-1750`.
Remote acquisition root: `D:\MoirenTest\20261103-target1280-timeline-1618`.
Unless specified otherwise, result paths below are under local `waveform-compact`.

## Frozen Inputs And Method

The original `run\process\records\2026-11-03 17-03-30.mkv` is reopened once for
this association. Its sole stereo48k PCM24 stream is copied losslessly in full:
1071104frames/6426624bytes, without trimming, resampling or channel changes.
Actual encoded native output comes from `run\process\cursor\owner-0001-generation-1`
and `owner-0002-generation-1`, each under `native-render`. These are independent
owner ordinals, not two globally distinct native-generation values.

`analysis-plan.json` fixes source frame24000/4096training frames per owner.
The baseline OBS search is [196800,244800); replacement is [297600,345600).
These new-run windows are constructed before waveform inspection from recording
start17:03:30.700 and owner starts17:03:34.603521/17:03:36.789786: a one-second
search on a0.1s grid with0.3s lead. This is a current lifecycle-only choice, not a
retroactively claimed old frozen formula. No previous lag/gain/results are used.
The approved plan SHA256 is
`1B352BB0AC7E10CC028E5D7BDEB008698BE8FEE8BB40E57451E4746CD6273B59`.

`analyze_render.py` is unchanged from the prior analyzer,35619bytes, SHA256
`6FFD7A69A4ECB29817BE5F0E05541A40188C5E8D3BF1DE9A8ACCA5961F72CCC9`.
It fits one integer lag and fixed per-channel gain/DC per owner, then freezes
them. Both search intervals are excluded from both evaluations. Absolute
tolerance remains2.384185791015625e-7, training correlation minimum0.9999 and
peak margin minimum0.01. There is no evaluation refit, local realignment,
threshold repair, owner splice or second window attempt.

## Complete Ledgers, Not A Continuity Pass

`summary.json`, `analysis.json` and `extraction.json` retain all denominators.
Native accepted/present output is97616frames baseline and385568replacement;
rejected, unknown-present, nonfinite and trailing-byte counts are all zero.
These native archive facts do not remove evaluation-unknown regions.

| Owner | Evaluation Domain | Frames | Unknown | Residual | Compatible |
| --- | --- | ---: | ---: | ---: | ---: |
| Baseline | Native-output/source coordinates | 97616 | 48000 | 2827 | 46789 |
| Replacement | Native-output/source coordinates | 385568 | 48000 | 689 | 336879 |
| Baseline | Entire OBS recording | 1071104 | 1021488 | 2827 | 46789 |
| Replacement | Entire OBS recording | 1071104 | 733536 | 689 | 336879 |

Both training fits are accepted. Frozen lags185379/289523 are OBS sample-coordinate
offsets only, not physical delay. The2827/689residual frames are real disagreement
under the frozen model, not an identified product defect, proven loss or explained
Stop tail. `offline_complete=true` describes analysis execution, not audio quality;
`quality_pass` and `waveform_continuity_qualified` remain false and physical latency
is null. The original2Process gap-bound violations, max11.1165ms against527frames/
10.9792ms, remain unchanged and are not overridden by these fits.

Main inspects the actual `alignment.png`; its warning and residual/unknown regions
remain explicit. No old packet855 coordinate or prior association is applied.

## Provenance, Failures And Transfer

Main authorizes one finite offline invocation in `main-offline-authorization.json`;
`offline-transport.stdout.json` records exit0, and result pins report unchanged
original inputs/software with zero native launches. The original229-input closed
manifest is pinned before reading and again at closure, not accepted by row count:
`closed-timeline\pins.json`,117500bytes, SHA256
`572392DEF1556EB4160EF45B454538B47FD3B58F9F89E6C9602FFE208C9E2212`.

Wrapper-v2's exact-manifest regression runs4tests: RED has3failures because a
same-count self-consistent replacement was admitted; GREEN passes4/4. Logs are
`wrapper-closed-pin-red.stderr.log` and `wrapper-closed-pin-green.stderr.log` in
the local root. This changes wrapper identity enforcement, not the analyzer,
windows or thresholds. The unchanged analyzer's28synthetic tests pass locally and
within the single remote offline invocation. Expected negative-fixture refusals
are retained, not reported as real-recording analysis failures.

Preparation failures also remain: an obsolete closed-input path and PS5 metadata
serialization/provider amplification. `metadata-wrapper-failure.json` explains
their local scalar projection; original stdout/stderr are preserved. Neither
failure is an audio acquisition failure or an additional waveform fit.

`download-verified.json` records511327ZIP bytes,19payloads plus the
`payload-pins.json` manifest (20entries), SHA256
`DEAB69BB0B19FB37E49A72221E7C53D39ADB9276F148BCA847A878EF10B3EF04`.
All payload lengths/hashes are verified. MKV, decoded OBS PCM and native samples
remain remote; no raw audio transfer, new acquisition or production change occurs.

## Residual Coordinates And Retained Packaging Failure

The evidence-only locator runs once,18:24:33..18:24:55, on the same raw inputs and
frozen model. `locator-compact/result.json` records Python exit0, complete=true,
unchanged input/software pins, no refit and zero native launches. The source
ledgers exactly retain48000unknown/2827residual/46789compatible baseline and
48000unknown/689residual/336879compatible replacement; the full OBS ledgers above
are unchanged. Main inspects the actual `locator-compact/location.png`.

`locator-compact/summary.json` maps each first evaluated residual to its own
persisted packet and Write event, with zero-based frame coordinates/offsets:

| Owner | Stored / OBS Frame | Packet / Event | Offset / Packet Frames | Stored Before / Accepted After |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 4674 / 190053 | 14 / 28 | 130 / 336 | 4544 / 4880 |
| Replacement | 384879 / 674402 | 1145 / 2290 | 319 / 336 | 384560 / 384896 |

Both linked releases have HRESULT0 and write padding176frames. These are exact
same-owner coordinate associations, not a cause, underrun verdict, playback
proof or Stop-tail explanation. No lag/gain/SRC, threshold or alignment is changed;
quality/continuity remain false and physical latency remains null.

The original wrapper task nevertheless ends Ready/result1 after successful
analysis:
PS5 `Measure-Object bytes -Sum` rejects OrderedDictionary payload pins during
packaging. A separate local synthetic expression reproduces that failure; the
locator is not rerun. At that stage, `locator-terminal-observation.json` records
no remote ZIP/transfer receipt. Nine compact files are individually length/SHA256
verified locally, totaling243871bytes by explicit summation, not the null total field in
`locator-compact-verified.json`. That verification alone did not establish
remote packaging success.

An independent packaging-only recovery runs18:31:58..18:32:06 and exits0;
`locator-package-closure-compact/result.json` records complete=true. The original
task remains result1 and analysis exit0; neither is rewritten. No analysis rerun,
refit or native launch occurs. All257source-input and3software pins are unchanged.
`locator-package-closure-verified.json` confirms the distinct104683-byte ZIP,
SHA256 `4D4734B936005A0291EADD185381CE35389467501BCC2EC8255A585E5D379E4F`.
Its exact nine members total243871bytes and match the original lengths/hashes.
Only compact evidence is transferred; raw audio remains remote. Packaging closure
does not qualify the locator, explain residuals or override the original2gap violations.

### Final Audit And Limits

The independent read-only final audit runs18:44:57..18:45:20, reports23785ms,
exit0/ok=true, and verifies1446pin references across302unique files, including
the unchanged229original inputs. `final-audit-v2/retrieved/completion.json`
records the observed audit exit0; the local transport receipt records SSH exit0.
The1933-byte `final-audit-v2/retrieved/receipt.json` has SHA256
`FDF600AB08E83DBA224F1BCDCE5BB16C425E89DD1C02CD16F16FB3A38074BDC0`.

Native task0, original locator task1, locator analysis0 and separate packaging
closure0 remain distinct; the earlier failure is not rewritten. The audit finds
no owners or running tasks and unchanged protected creation identities, driver,
boot and default/render settings. It performs no audio launch, fit, raw transfer
or cleanup. Evidence closure is complete, not quality or physical qualification:
the residuals, unknown regions and original2gap violations remain. Original S1-S3
qualification and physical latency stay open; no defaults or gates change.
