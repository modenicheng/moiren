# Actual Process SRC replay to independent OBS association

## Scope and invariant

This increment extends [native PCM retention](2026-11-02-process-native-pcm.md)
with an offline, falsifiable association between its fixed actual SRC output
and an independently recorded OBS return. No native audio behavior, queue,
servo, default, admission gate or recovery policy changes.

`scripts/process-replay-obs.py` reaudits the original native PCM/cursor archive
and independently reconstructs all owners before fitting recorder coordinates.
It does not fit a replacement production phase, step, generation or reset.
Every preselected old recording-coordinate window is retained, including old
affine refusals. Every owner is considered: two compatible owners refuse as
ambiguous, rather than selecting the better score.

The coarse search uses a training-only patch. Two bounded passes cover every
possible owner-frame origin, including block boundaries, then measure the
strongest competitor outside the same one-millisecond exclusion radius. A
distant repeated phrase therefore remains ambiguous. Only a local two-second
crop reaches the existing affine recorder fitter. Its training/held-out split,
optimizer, bounds and all `obs-linear-clock.py` gates are unchanged.

The reported recorder mapping is separate from actual Process SRC history.
Both interpolation neighbors must have a valid actual-replay sample for every
sample of the selected window; zero-filled UNKNOWN/priming storage refuses.
Stereo location and rate must agree under the existing gates. An associated
window is not whole-run continuity, native-owner lifetime, Engine/render epoch
proof, acoustic delay or calibrated physical latency. Every report keeps
`qualified:false`, `physical_qualified:false`,
`waveform_continuity_qualified:false` and `physical_latency_ms:null`.

## Evidence binding and bounds

The manifest names the source archive, a hash-pinned decoded float32 return
WAV, a hash-pinned original summary, the hash-pinned original recording and a
hash-pinned FFmpeg/FFprobe executable pair. The old summary's recording path and byte count
must identify that recording. Before any replay allocation, FFmpeg independently
decodes both complete tracks to streaming SHA256 values; they must match all
PCM bytes in the supplied reference/return WAVs. FFprobe checks exactly two
lossless stereo tracks at the same sample rate as the WAV. Reference track0
cannot be substituted. The hash-pinned control is independently reassessed
against the current recording's fixed first60seconds, using the original
isolation gates. Historical control flags alone never establish current
isolation. This is file/track consistency, not authentication or calibration.

Source and derived storage have separate512MiB budgets, each aggregated across
all owners, not512MiB per owner. All receipts are size-bounded; one to eight
owners and one to128prespecified windows are accepted. Coarse FFT work uses
two-second blocks plus template overlap; full-recording float64 conversions
are not needed. WAV/replay arrays are read-only memmaps and are closed after
plotting. No raw media is downloaded for analysis.

All output is create-new and outside immutable source evidence. Plot errors
retain a numeric refusal receipt. Nonfinite JSON tokens and exponent overflow
are rejected before original-control data can enter a report; JSON is serialized
before opening its destination. Original native faults and old control/waveform
refusals remain in their source receipts and cannot be converted to PASS by this
tool.

Manifest schema:

```json
{
  "schema_version": 1,
  "archive_directory": "D:/run/cursor-trace",
  "recorded_wav": {"path": "D:/run/analysis-track2.wav", "sha256": "SHA256"},
  "prior_summary": {"path": "D:/run/analysis-summary-0715.json", "sha256": "SHA256"},
  "recording": {"path": "D:/run/records/run.mkv", "sha256": "SHA256", "return_audio_index": 1},
  "decoder": {"path": "D:/ffmpeg/bin/ffmpeg.exe", "sha256": "SHA256"},
  "probe": {"path": "D:/ffmpeg/bin/ffprobe.exe", "sha256": "SHA256"},
  "reference_wav": {"path": "D:/run/analysis-track1.wav", "sha256": "SHA256"},
  "control_wav": {"path": "D:/run/control-float.wav", "sha256": "SHA256"}
}
```

Run in an isolated analysis environment, not the application/audio process:

```powershell
python -I -B scripts/process-replay-obs.py --manifest manifest.json --output-directory NEW
```

Exit0 means all selected windows associated, still unqualified. Exit2 preserves
an explicit refusal; it does not request another native run or relaxed gates.

## Software regression

The initial missing numerical API, missing archive-driver/bounds/finalization
helpers, missing recording binding and JSON-overflow rejection all have retained
RED receipts. Twenty-one tests pass after those repairs. They cover fractional
recording phase/rate and gain, fixed non-affine actual replay, held-out-only
corruption without parameter retuning, dropped/repeated/zero/mixed samples,
UNKNOWN neighbors, repeated phrases, cross-owner ambiguity, stereo disagreement,
global block-edge peaks, malformed data and exact original gates.

End-to-end tests construct the Rust archive wire format, independently replay
real32-frame spans, build a two-track MKV with FFmpeg, decode/hash the return,
associate and render. A distinct reference/return fixture rejects a track swap
and an unrelated prior recording; same PCM under a wrong WAV rate also refuses.
Fresh control validation prevents relying on a historical receipt for different
recording content. Plot failure retains JSON. Separate source
and derived aggregate-cap refusal is tested before allocation.

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Final related suites are `s1-replay-obs-final-*-1102.log`: native PCM32,
linear-clock16, isolated12 and two-path14; Ruff check/format pass. These are
software tests, not native device qualification. The new suite passes21 after
the final per-window refusal repair (`s1-replay-obs-window-refusal-green-1102.log`):
one malformed numeric window cannot erase other preselected-window results.

An independent review finds and closes two actual evidence-integrity gaps:
independently correct WAV/prior hashes did not bind their recording/track/rate,
and exponent-overflow JSON could break final serialization. The repair freshly
probes/decodes/reassesses recording/control evidence and rejects nonfinite
numbers before retaining them. The512MiB source and derived limits are separate
aggregate budgets, not a claimed combined512MiB total.

The independently exported staged tree also passes new21/native32/linear16/
isolated12/two-path14tests, Ruff check/format and repository fmt-check. Exact
receipts are `s1-replay-obs-scoped-*-1102.log`. Only Python and validation
documents change; Rust/driver/runtime sources are byte-unchanged by this
increment. The eighteen preexisting dirty files remain outside its scope and
retain their earlier hashes.

## One remote offline association

No new native audio run occurs. The original
`D:\MoirenTest\20261102-060101-native-pcm-smoke` recording, archive and old
summary are reused immutably. A new dedicated Python3.10 environment at
`D:\MoirenTest\20261102-actual-replay-env-20261102-064700-54e2c8f1`
disables system site packages and user-site/environment imports (`-I -B`).
All third-party imports and installed distributions resolve within that
environment: NumPy1.25.2, SciPy1.9.3, matplotlib3.7.5. The prior shared venv
exposed an external Manim SciPy even under `-I`; it is not modified or used.
Pinned binary-wheel install and freeze are retained; freeze SHA256
`733CB9A8A78D7C53BA5766305D6165B0529A7ED80F0BADC5BE0DCDBD881C9B42`.

The one bounded offline task completes in25.765s, exit0. It freshly reaudits
51790514source bytes and creates110939904derived bytes, within separate budgets.
Both complete OBS tracks independently decode/hash to their WAV payloads;
48kHz/stereo/lossless probing passes. Fresh full-control isolation remains
`bounded_control_clear`. All four unchanged recording-coordinate windows
uniquely associate with `owner-0002-generation-1`; baseline owner1 is refused
as ambiguous/unmatched in each case.

| Recording time (s) | Prior nominal-source affine verdict | Actual-replay held-out NCC | NRMSE | Maximum local NRMSE |
| ---: | --- | ---: | ---: | ---: |
| 38.559521 | Refused | 0.999998838 | 0.001524492 | 0.003861702 |
| 47.559521 | Refused | 0.999999702 | 0.000771943 | 0.003546401 |
| 57.559521 | Compatible | 0.999998920 | 0.001469641 | 0.003851317 |
| 65.559521 | Compatible | 0.999998229 | 0.001882244 | 0.003857422 |

Both channels agree and all fitted neighbors are valid. Existing gates remain
NCC>=0.99, held-out NRMSE<=0.05, local NRMSE<=0.1, plus the unchanged uniqueness,
stereo and rate bounds. This substantially supports the explanation that the
old local nominal-source affine model missed actual changing SRC history in
those two windows. It does not prove the entire stream is undamaged or resolve
the native eight delivery-gap violations. The original two refusals remain in
the old receipt; they are not rewritten. UNKNOWN/priming3584/3264frames,
prime-discard352frames, native gaps and lack of physical calibration persist.

Remote output:
`actual-replay-obs-output-20261102-065245234-34410a22` beneath that original run.
Only compact JSON/PNG are copied to the host directory
`20261102-actual-replay-approved-20261102-065245234-34410a22` under the evidence
root, with remote/local lengths and SHA256 independently rechecked:

- Summary21734bytes: `7EE85A66EB7ED577EA34308B139DE1329DB1EBD39B031CC130A54AF24B4ED9E1`.
- PNG335760bytes: `09257A8DA6253F0369CEDE8007F3083285504D779911E1C8C3A287C09BE974B9`.

The PNG is visually reviewed. The full source archive's1347-file manifest hash
remains `611D1362B3992C74A43C9F93BFBFCFA0C6112B5369C02AAB9642ED5D98B9316D`.
User OBS PID12648 and QQMusic PID15096 start times are unchanged; no owned
offline process remains. The task is not restarted. The exact executed script
is SHA256 `D1DC1D88BF3190EDBA148CDBAE0984F871849247305FA5F458BC28989908156C`.
Final code adds only per-window numeric-exception refusal preservation after
that run; this diagnostic failure delta has a RED/green regression and is not
claimed exercised remotely. No queue/default, first-packet policy or physical
PASS changes. S1-S3 remain open at their remaining native/physical boundaries.
