# Bounded local linear fit on retained PCM - 2026-11-02

## S1 problem and root fix

The preceding warm-control recording's held-out affine diagnostic exceeded its
180-second subprocess limit without producing a fit result. That was not a
waveform verdict. This slice fixes its offline computation rather than opening
another audio client, weakening gates or repeating a stability scan.

Profiling a64-second8k stereo reference finds0.856of0.929seconds inside670
`np.interp` calls. Every candidate prediction passes the whole strided stereo
channel and a recording-length sample index, even though only a half-second
window is queried. A known70-second48k stereo fixture takes8.5579seconds for
one window with the preceding implementation. This unnecessary dependence on
recording length is the identified performance defect, not an audio-path bug.

The dense unit sample grid permits exact local two-neighbor linear gathering.
`predict_linear` now reads only the neighbors of the queried positions and
preserves fractional phase and the prior boundary refusal. It does not clip,
round, extrapolate, change the seed grid, select different windows, change
optimizer bounds/tolerances, or modify training/held-out separation and gates.
The existing caller validates finite stereo PCM; optimizer bounds guarantee
monotonically increasing queried positions. No production bridge, servo,
admission, RT path, default, driver or app binary changes.

## Regression and performance evidence

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Two actual RED failures precede implementation: a window prediction cannot
submit a recording-length interpolation grid, and the local predictor must
match the dense oracle. Final tests additionally cover strided million-frame
coordinates and both `nextafter` neighbors of an integer. Existing signed gain,
fractional phase, positive/negative drift, held-out-only corruption, training
independence, drop/repeat, two-path mix, silence, periodic ambiguity, stereo
disagreement, finite/boundary and rate-limit refusals remain unchanged.

`s1-local-fit-red-final-0448.log`:2failures/13pass before the fix.
`s1-local-fit-neighbors-0501.log`:16pass after it. Final all-nine-suite rerun
passes147tests (`s1-local-fit-final-manifest-0509.json`). These are offline
software tests, not device runs. Rust code is
unchanged and the preceding workspace/GPUI receipt is not relabeled as rerun.

`s1-local-fit-benchmark-0450.json` compares one fixed fit with before/after
implementations on the same deterministic data in the same process:

| Stereo reference | Before seconds | After seconds | Ratio |
| --- | ---: | ---: | ---: |
| 5s,8k | 0.0448 | 0.0403 | 1.11x |
| 64s,8k | 1.0191 | 0.0393 | 25.91x |
| 70s,48k | 8.5579 | 0.1843 | 46.44x |

All three preserve the compatibility verdict and exactly identical offset,
correction, gain, held-out correlation, held-out NRMSE and local maximum NRMSE.
These timings describe this host fixture only, not a general speed guarantee
or Moiren audio latency. The resource regression is structural, not a flaky
wall-clock pass threshold or a universal allocation proof.

## Retained remote analysis

Remote base: `D:\MoirenTest\20261102-s3-warm-control-0337`.
No native audio/UI process is launched. Analysis reads the already closed
recording's two3283968-frame PCM tracks and all28previous raw-analysis windows.
It does not rerun acquisition, crop the control, choose successful subsections
or rewrite the original failed capture/raw receipts.

Tools archive SHA256:
`07759C0B90A7EE74A8327BE0D6D026C0F69F1E23F02560AEA105CCEADD409374`.
The15-test target regression passes; the full28-window analysis with rendering
completes in17.4154seconds within the unchanged180-second subprocess limit.
Before/after SHA256 and length checks prove the original capture receipt,
raw summary/plot and both PCM tracks are unchanged. Final16-test and fixed
first/middle/last dense-oracle verification are recorded separately in
`s1-local-fit-equivalence-0505.stdout.log`: all16target tests pass and complete
output dictionaries are exactly equal for windows0,14,27. They include a
refused first window, not just successful fits. The preceding dense code takes
13.5453,8.2206,8.9299seconds for those three windows, respectively. No all-window
dense equality or general bitwise-equivalence claim is inferred from three
known fixtures and three retained windows.

Twenty-one of28windows meet all existing local affine-model gates, versus
five supported by the original equal-rate raw/highpass diagnostic. Seven
windows remain refused; no refusal is erased. Observed fitted corrections
change through the recording, so this result supports a local drift/phase
explanation for many raw-NCC mismatches but does not establish actual production
cursor history or explain every waveform anomaly. Two channels copied from
one source are not independent hardware references.

The21model-compatible offsets span95.6148..95.8837ms,
p50=95.8397,p95=95.8827,p99=95.8835ms. These are selected uncalibrated OBS
relative offsets, not a whole-run/physical latency distribution. The complete
control still fails its original correspondence gate; isolation=false,
physical_qualified=false and waveform_continuity_qualified=false. The original
harness wait failure and five gap-bound violations remain. Neither a path
winner, zero-fault verdict nor default promotion follows.

`analysis-linear-local-0452/overview.png` shows raw versus held-out correlation,
fitted local rate and model-compatible offsets, with qualification warnings.
It is downloaded and visually reviewed. Raw media/PCM stays remote. Verified
compact artifacts:

| Host artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s1-local-fit-summary-0456.json` | 58207 | `F0BE06AD6052C07FBE67F8B8943ECDE2913E06C4B76A0FF11D1F2B2B872D0B2D` |
| `s1-local-fit-plot-0456.png` | 262365 | `CAEC4B323927AD4D10FD964C812D8C31382561136D58DAD37B901B5B3B221134` |

The separate execution/equivalence receipts are also length/hash verified:
`s1-local-fit-receipt-0456.json`,1861bytes,
SHA256 `281C866643E5F2112112BF178C02B884F5395BC8D53687F58830C612A03CE43E`;
`s1-local-fit-equivalence-0511.json`,711bytes,
SHA256 `94888B4D2B4D61418C876765F4F51E36577A2A17E1CD6C857324B55EB56A36A1`.
Final remote audit03:53:42+08:00 (`s1-local-fit-final-audit-0511.stdout.log`)
finds no native or analysis owner; original OBS12648/QQMusic15096 identity is
unchanged. The operation opens no audio clients and preserves the failed-run
evidence, rather than using a successful fit to rewrite the acquisition.

The eighteen existing dirty files remain hash-verified and excluded. No push,
memory update, host process/audio operation or new stability scan. S1-S3 remain
active and incomplete; physical calibration remains unavailable, not a reason
to block further software development.
