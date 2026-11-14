# Retained Process PCM: held-out linear-clock diagnostic - 2026-11-02

## Scope and model

S1 requires changing-signal analysis with clock/drift handling. The Process
bridge in `clock_bridge/source.rs` uses continuous linear interpolation,
sampling before phase advance, with correction updates every32 output frames.
It is not the separate32-tap physical/virtual boundary FIR. Fractional phase
and rate drift can therefore reduce equal-rate noise NCC without proving lost
PCM. This diagnostic tests that explanation; it does not declare it the cause.

`scripts/obs-linear-clock.py` reads the closed two-track acquisition described
in [the original receipt](2026-11-02-isolated-process-waveform.md). It neither
opens an audio client nor modifies the recording, original summary or original
raw/control refusals. All28 original half-second windows remain present.

Each channel fits one affine source cursor, signed gain and DC using only the
outer quarters. Coarse correspondence and objective normalization also use
training samples only. The middle half, with16-frame guards, is held out.
There is no per-sample time warp. Fixed gates are held-out NCC>=0.99,
NRMSE<=0.05, local64-frame-scale NRMSE<=0.1, stereo offset disagreement<=0.25ms,
stereo rate disagreement<=20ppm and coarse competitor ratio>=1.8. Search is
bounded to+/-2000ppm; a fit at>=1999ppm magnitude refuses. That search bound
is an explicit diagnostic assumption, not an observed OBS clock envelope.

Epoch association remains unknown: unobserved resets can lie inside a fitted
window. A local affine fit is not the production servo's actual changing cursor
history. Physical and whole-waveform continuity qualification are always false.

## Actual retained-PCM result

The final package runs13 synthetic tests and then offline analysis remotely in
the existing isolated analysis venv. No new native acquisition is involved.
Output: `D:\MoirenTest\20261102-s3-isolated-waveform-0128\linear-analysis-0230`.

- Only4of28 windows satisfy every model gate;24 remain unresolved or
  nonaffine. Held-out NCC alone is insufficient: local/overall residual gates
  still refuse windows whose correlation looks high.
- The original pre-Moiren control/reference refusal remains. Isolation is not
  established. Neither the original6 selected raw windows nor the4 model
  windows form a whole-run latency distribution.
- The plot shows improved local waveform correspondence for some windows and
  changing fitted rate. This supports investigating fractional SRC/drift, but
  does not establish lossless output, actual servo state, a path winner or a
  physical latency figure. No threshold or production setting changes.

The three-panel PNG shows raw versus held-out correlation, fitted local rate
and only model-compatible OBS-relative offsets. It is visually reviewed; raw
PCM and MKV remain remote. Final PNG is byte-identical to the earlier reviewed
model report. Earlier analysis `linear-analysis-0211` is preserved separately.

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| Final tools ZIP, `s1-linear-tools-final-0227.zip` | 18950 | `90F8DE53E6E79E7FEC12E8C40EA3570E7D9D35F6D98831F75A6AA5BAD0433197` |
| Final summary, downloaded `s1-linear-summary-final-0208.json` | 58127 | `F786CB4AF5963326EAD3F89693D78C6FCB52E8F02166FB90E4FE2FE636BF173A` |
| Final PNG, local `s1-linear-plot-0217.png` | 255328 | `F11F1AFD6E8D1942D91C60E57C563892FF32FE785FBB620DC4096BD4D8DB8FF6` |

## Regression, review and parallel work

Synthetic RED/GREEN evidence covers missing implementation, rate-bound
saturation and held-out-only corruption. Review identifies held-out energy
leaking into objective normalization; the new intercepted-objective regression
fails before training-only normalization and passes afterward. Tests also use
an independent scalar sample-before-advance oracle and a piecewise32-frame
rate-change source. Known delay/drift, negative gain/offset, silence/tone,
drop/repeat, holes, unrelated/mixed signal, stereo disagreement, truncation and
nonfinite input retain refusal coverage. Final narrow read-only review finds
no remaining substantive defect; it does not substitute for execution.

All9 Python suites pass144 tests locally; the final13-test suite also passes
on the target. `cargo fmt --all -- --check` and `git diff --check` pass. This
Python-only slice does not relabel earlier Rust runs as fresh workspace tests.
All18 preexisting unrelated dirty files match the preceding source manifest
and remain excluded. Verification receipts: `s1-linear-all-and-preserved-0209.json`
and `s1-linear-analyze-final-0230.stdout.log` in the evidence root.

In parallel, an independent bounded native operational run tests measured
Process target1344, preserving Shared336/render512/Engine64/capacity16384/
jitter96 and runtime admission. It may refuse if its fresh measured minimum
exceeds1344. It is not an isolated recording or a physical test; its terminal
result is [recorded separately](2026-11-02-process-input1344.md). No smaller
default or weakened reserve follows.

S1-S3 remain incomplete. First-packet investigation stays deferred and missing
physical calibration does not block useful application development.
