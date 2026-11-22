# Shared code 6 nominal-template refusal: bounded diagnostic

Date: 2026-11-02, Asia/Shanghai. Inspected committed source: `fb4da3c`.
This is one remote offline analysis of existing closed recordings, not another
native acquisition, a production fix, or a quality/latency qualification.

## Question and fixed scope

The retained [Process CLI recording](2026-11-02-process-cli-packet-capture.md)
resolves 18/19 graph codes, and the preceding
[VAD recording](2026-11-02-matched-graph-packet-capture.md) resolves 17/19.
Both refuse the strongest candidate code 6. Codes are zero-based, so this is
the seventh scheduled code, not the sixth. A refused strongest candidate is
not an accepted identity, and no identity is assigned from recording order.

The predeclared diagnostic examines only code 6 and neighboring accepted
controls 5/7, both channels, source and graph captures on each route. It
reproduces the existing nominal integer-offset correlation, global means,
four-quarter checks, valid-onset count and onset margin. RMS 0.0001,
correlation 0.7 and peak margin 0.05 remain unchanged. It does not rerun the
all-19-code identity competition or modify its retained results.

The fixed-seed scalar xorshift32 generator is independently reconstructed
from committed `latency/marker.rs` and `e2e_latency_probe/source.rs`. All three
generated source codes match the actual source-capture samples bit-for-bit
on both routes/channels. Every reviewed full-window best offset, correlation,
section verdict, valid-onset count and onset margin reproduces the original
Rust result; numeric differences are below 5e-12. The dirty worktree detector
and SRC experiments are neither built nor used as production truth.

## Observed refusal mechanism

Channel 0 is displayed; both channels are checked in the retained summary.
Quarter correlations use the full-window means, matching the Rust detector.

| Graph candidate | Global NCC | Quarter1 | Quarter2 | Quarter3 | Quarter4 | Onset margin | Valid onsets |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Process code 5, accepted | 0.835603 | 0.705450 | 0.813533 | 0.878972 | 0.929114 | 0.316824 | 1 |
| Process code 6, refused | 0.717688 | 0.821342 | 0.769775 | 0.693165 | 0.580230 | 0.010715 | 0 |
| Process code 7, accepted | 0.871992 | 0.830735 | 0.838476 | 0.901941 | 0.914748 | 0.421233 | 1 |
| VAD code 5, accepted | 0.793911 | 0.798182 | 0.795120 | 0.796326 | 0.786420 | 0.281518 | 1 |
| VAD code 6, refused | 0.740310 | 0.781016 | 0.766098 | 0.732662 | 0.680964 | 0.127566 | 0 |
| VAD code 7, accepted | 0.895517 | 0.933866 | 0.883936 | 0.900210 | 0.862995 | 0.560942 | 1 |

Process code 6 fails both the section floor and onset margin. VAD code 6 fails
the section floor despite adequate onset margin. Inspecting adjacent integer
offsets does not recover a valid onset; the original full-window search
already found zero valid onsets throughout each supplied support window.
This explains why the detector refuses, not why the native waveform changed.
It does not establish a dropped packet, frame loss, a specific SRC defect or
an absence of real damage. The separate VAD code 1 refusal is not reanalyzed.

## Independent fixed interpolation counterexample

No recorded graph samples are fitted, resampled or corrected. Independently
generated exact controls 5/6/7 are each transformed at predeclared phases
`0, 0.25, 0.5, 0.75, 1` using
`y[n] = (1-phase)*x[n] + phase*x[n-1]`, then converted to Float32.
Zero padding retains the one-sample tail. This deterministic two-tap filter
inserts or drops no source samples. Intermediate phases change waveform shape
and frequency response; phase 0 is identity and phase 1 is a retained-tail
one-sample shift. The intermediate-phase controls are not bit-exact waveform
preservation; no claim about invertibility or lossless information recovery
is made.

At phase 0.5 all three controls fail the unchanged single-code section/onset
gates. Their global NCCs are 0.702772, 0.717137 and 0.698562; minimum quarter
NCCs are 0.648940, 0.693097 and 0.672936. Adjacent-onset margins are approximately
zero. The other four fixed phases satisfy those single-code gates. These are
not all-code identity qualification results.

This supplies a bounded counterexample to interpreting nominal-template
refusal as proof of dropped samples. It is not a model fit or replay of the
actual native SRC: the recordings lack source-to-render interpolation cursor
and epoch provenance. Similar scores do not establish equal causes. Real
waveform damage and the actual source of these refusals remain unresolved;
neither 18/19 nor 17/19 is upgraded to a continuity PASS.

## Execution, isolation and artifacts

The held Python process runs only on authorized target `LAPTOP-NP49RG6H`,
using the existing isolated environment with `-I -B`, prefix-checked NumPy
and matplotlib, single-thread numerical settings and an output-local plotting
cache. Computation takes approximately 2.1 seconds; the bounded runner exits 0
normally. No audio client, source, recorder, driver/device control or GUI is
started or changed. Existing OBS and QQMusic creation identities match before
and after; neither process is stopped. A final read-only owner audit follows.

All 24 hashed inputs are unchanged after analysis. Four raw PCM files remain
remote, with original native receipt hashes. Only 773650 bytes of compact
JSON/log/PNG artifacts are downloaded and verified by exact length and SHA256.
The first runner invocation exceeds the Windows command-line limit before
remote execution; the same runner is transferred as a script and runs once.
Two download syntax attempts fail before transfer; a compact SSH bundle then
succeeds. There is no analysis rerun, replacement output or hidden recording.

Remote output:
`D:\MoirenTest\20261102-code6-diagnostic-1138\output`.

Verified host evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-code6-diagnostic-20261102-1138`.

- `analyze.py`, `intent.json` and `remote-run.ps1` retain the fixed procedure.
- `receipt.json` retains exact held-process identity, normal exit and protected
  process identities. `download-manifest.json` pins compact artifact hashes.
- `output/summary.json` retains both channels, all reviewed offsets/quarters
  and 15 fixed synthetic controls; before/after input manifests match.
- `output/nominal-quarter-review.png` shows actual quarter and adjacent-offset
  scores. `output/fixed-interpolation-controls.png` shows independent controls.
- `preserved-before.json` and `verification.json` retain original 18 dirty-path
  preservation and the final no-owner audit. Both existing Python cache
  directories are left untouched.

Independent read-only reviews verify the numerical and qualification wording.
One wording finding is corrected: phase 0 is identity and phase 1 a retained
shift, so waveform-shape/information cautions are scoped to intermediate
phases rather than asserted for every control. No new execution follows.

## Stopping boundary

No demonstrated production defect justifies a code patch from this evidence.
Keep the detector, quality gates, Compatible defaults and native verdicts
unchanged. Do not add another fit, journal, re-recording or unchanged soak to
this investigation. First-packet work and the withdrawn fail-fast experiment
remain deferred/withdrawn. Missing physical calibration remains a separate
qualification limit, not a reason to block independent development.

Future remote long tests may run alongside development when they answer a
specific new regression or acceptance question. Pin their binary/configuration,
bound their duration and load, and retain normal completion/cleanup evidence;
offline numerical analysis must not contend with a native acquisition. The
preceding clean integration checkpoint is historical evidence, not rerun by
this documentation-only increment. No push or memory update occurs.
