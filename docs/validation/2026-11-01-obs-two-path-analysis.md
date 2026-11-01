# Retained OBS two-path recording: corrected offline analysis

Date: 2026-11-01, Asia/Shanghai. Based on e6076f9, independently of the
unresolved [desktop VAD PCM fault](2026-11-01-live-gui-vad-revision-fault.md).
This increment repairs the measurement tooling and analyzes an existing
recording; it does not start a new source, audio owner, OBS or recording.
S1-S3 and continuous-waveform correctness remain incomplete.

## Actual measurement boundary

The retained recording has exactly two lossless PCM24,48kHz stereo tracks:

1. Python process capture, serving as a pre-route reference.
2. Shared DAWN PRO2 output capture containing the directly rendered Python
   signal plus the Moiren Process Loopback -> unity Bus -> same output path.

Neither track is a calibrated physical DAC/ADC reference. This is not the
VAD playback -> Bus -> MIC route and cannot qualify its failure recovery.
The original remote recording is
`D:\MoirenTest\20261101-clean-comb-probe-1320\records\2026-11-01 13-18-13.mkv`,
55082942 bytes, SHA256
`9BEB95CFDF63354DCA5B02A24A8C93CAC29B430F948AC40EB18082BB912CCCD6`.
Both decoded tracks contain2971648 frames,61.909333 seconds, no nonfinite or
clipped samples. These facts do not establish uninterrupted returned audio.

## Defects removed from the earlier exploratory analysis

- Cumulative-energy cancellation let an FFT numerator produce impossible
  `abs(NCC)>1` in silent tails. Long-double local energy plus an explicit
  numerical error bound now marks unsupported windows unknown. A direct
  centered-window oracle checks signed NCC; silence is not normalized by an
  arbitrary tiny denominator.
- Applying `maximum` to a complex FFT corrupted the old transfer denominator.
  The corrected H1 response is `Pxy/Pxx`, with real input power and a separate
  coherence/support mask. No spectral-delay fit is claimed.
- Restore direct/delayed correlation0.08/0.06, peak/null ratio1.8,
  stereo agreement0.25ms and raw/high-pass agreement0.35ms. Both raw and3kHz
  high-pass peaks must pass. Exact zero disagreement is valid, not missing.
- Fixed direct/returned search ranges and an8ms exclusion could hide a real
  sub-10-ms pair when OBS's common offset is negative or the return is
  stronger. The analyzer now finds two peaks over-120..500ms, orders them by
  time and compares both against remaining competitors outside1ms. A known
  -20/-15ms pair verifies this case. Separation at or below1ms remains
  explicitly unresolved, not evidence of minimum achievable latency.
- A sweep can have strong local peaks and broad whole-window energy. Noise
  statistics additionally require noise-like entropy/concentration in all
  four125ms subwindows. Non-noise candidates, rejected windows and their
  timing jumps stay in the report rather than disappear.
- Do not infer exact source origin or silence contamination from an RMS
  threshold. Recompute source/reference correspondence from changing PCM.
- Compact JSON retains metrics for every window, not embedded full lag arrays.

Earlier provisional numbers, the invalid spectral panel and apparent silence
contamination are superseded by this report, not retroactively passed.

## Result on the unchanged recording

Final analyzer SHA256:
`B7659D4180A5A374F9E293E6C3E09B9B01C7E1EEF8B33B09FD6252F9ADF3A2EA`.

- 120 windows are inspected at0.5s spacing.49 pass raw/high-pass two-channel
confidence;43 also pass the broadband-noise gate, spanning recording centers
18.75..39.75s. Earlier noise windows remain refused by the unchanged strict
confidence requirements. All120 metrics and reasons are retained.
- Relative separation across those43 noise windows: p50=105.466415ms,
  p95=106.225944ms, p99=106.234835ms, max=106.235656ms,
  min=103.998277ms. No adjacent noise-window change exceeds0.35ms. This is
  not proof of no sample loss, dropout or recovery between/inside windows.
- Two adjacent jumps remain among all confident candidates, in the non-noise
  tail. They are not automatically Moiren losses or removed as outliers.
- The representative noise window is21.75s; its106.226179ms separation
  implies9.413875Hz comb spacing. That is algebra from the measured relative
  peaks, not an independent measurement from spectral notch spacing.
- Only43.12% of500..18000Hz bins meet coherence>=0.8 in the selected local
  H1 spectrum. The figure masks unsupported bins and zooms3000..3250Hz;
  this spectrum is diagnostic, not a validated comb-delay inversion.

## Source/reference correspondence and the tail caveat

An independent direct readback compares all64 non-overlapping500ms source
noise blocks (source5..37s) against the reference with one fixed offset:
141483 frames,2.9475625s. After per-channel gain fit, minimum correlation is
0.9999998753367705, maximum block residual RMS8.865303e-6 and maximum sample
residual1.567858e-5. Quantization differs because the saved source is PCM16
while the player wrote its original float waveform. This is strong noise
source/reference correspondence, not bit identity or returned-path quality.

Separate500ms searches at source40.5,41,43 and45s retain the same offset
and correlation about0.99999990. At source47s the strongest retained match
is only0.46839, with offset3.5295625s. The reference itself no longer supports
the earlier source mapping there. The player ignored the blocking
OutputStream.write underflow return and left its client open after the write;
the current evidence does not isolate the tail cause. Do not classify that
tail or schedule-overlap RMS as Moiren-only damage or ongoing contamination.

## Validation, environment and artifacts

`python scripts/tests/obs-two-path.tests.py` passes14 tests on the host and
remote final environment. Red/green checks include silent-tail numerical
failure, signed known delay, unknown/unrelated input, strict gates/zero
agreement, negative-offset low delay, complex response, inserted sample
steps, drift, constant tone, narrow/wide sweeps and invalid lag/order.
Synthetic steps test jump visibility, not automatic diagnosis of real losses.
These are analyzer tests, not another Rust or native audio acceptance run.

Remote analysis uses a task-owned venv under
`D:\MoirenTest\20261101-obs-analysis-validated\venv`, Python3.10.11,
NumPy1.25.2, SciPy1.9.3 and matplotlib3.7.5. System packages are readable,
but installed analysis packages shadow them only inside this venv; original
global NumPy1.26.4 is not uninstalled. The original SciPy/NumPy warning and
existing unrelated global Manim/watchdog/invalid-distribution pip warnings
are retained. No user/global package repair, audio-default, volume, driver,
boot or application-settings change is performed.

Final remote presentation:

```text
D:\MoirenTest\20261101-obs-analysis-validated\analysis5\report.html
D:\MoirenTest\20261101-obs-analysis-validated\analysis5\overview.png
```

Local complete presentation (same filenames, working relative SVG reference):

```text
D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\validated-report-final\report.html
D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\validated-report-final\overview.png
```

Transferred reports/images/metrics are hash-checked against remote output;
the large MKV is not downloaded again. Final `summary.json` SHA256 is
`3DAB658F3415DD6ED6E18AD288F7701DA8B467EA3D8DA10549F3FB9F87A5275F`,
`windows.json` is
`5BFF2E978F17A396AE550A0CEF7F6A6AFA84BE741060843EA3A7BB8A518FF9A3`,
and PNG is
`DA96C0063A66D741625D521FA3430705D893960F465FDCB976DB67D2159A86D1`.
An independent readback recomputes counts, distributions, NCC bounds,
window coordinates, jump counts and artifact hashes. It remains an
independent calculation by the same agent, not an independent human review.

Reproduce without audio activation (paths must refer to retained inputs and a
new output directory):

```text
python scripts/tests/obs-two-path.tests.py
python scripts/obs-two-path.py --mkv <retained.mkv> --ffmpeg <ffmpeg.exe> --ffprobe <ffprobe.exe> --out <new-output-directory> --recording-sha256 <verified-sha256>
```

The next application correctness gate still needs complete source status,
returned-path continuity and clean lifecycle completion. The desktop VAD
`0x8007051A` requires a separately controlled diagnostic-binary reproduction;
this offline Process-only result cannot explain or clear it. No calibrated
physical delay, sub-10-ms pass or smaller default is promoted.
