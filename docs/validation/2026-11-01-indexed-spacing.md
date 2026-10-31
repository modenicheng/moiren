# Frozen indexed coordinates: constant-affine incompatibility

## Question and result

The [complete-support model diagnostic](2026-10-31-both-minimum-src-batch.md)
finds all29 original marker identities in the retained both-minimum recording,
but does not reconstruct the source-to-recorder mapping. This follow-up asks
a narrower question before adding a waveform/clock correction: **can one
positive constant-affine mapping contain every frozen model-body interval?**

The answer is **no for those model intervals**, even allowing each complete
reported one/two-frame interval. This excludes explaining all these particular
coordinates by a single origin shift plus constant rate. It does not prove
the physical signal has a nonlinear clock, identify servo/SRC history, or
establish actual delay, lost frames or continuous quality. Model-dependent
coordinate biases and time-varying processing remain possible explanations.

No PCM correlation is repeated. The source schedule and all29 immutable global
and individual model results are inputs; no interval, model grid, threshold,
window, code identity or tolerance is widened after seeing the outcome. This
is not a native acquisition or a replacement for the original nominal refusal.

## Exact interval method

Let source marker starts be `x_i`, and model coordinates belong to the closed
intervals `[l_i, h_i]`. For a constant mapping `a + b*x_i` with `b > 0`, each
pair with `x_j > x_i` requires:

```text
(l_j - h_i) / (x_j - x_i) <= b <= (h_j - l_i) / (x_j - x_i)
```

All pair bounds are intersected using exact integer fractions. The condition
is necessary and sufficient: eliminating a common intercept from
`max(l_i - b*x_i) <= min(h_i - b*x_i)` yields those bounds. No intercept is
fitted or exported; floating-point absolute origins and least-squares
detrending are not involved. The tool records the strongest conflicting pair
witnesses and every adjacent spacing/residual interval in source-identity
order. Recorder window order never assigns source identity.

For the29 frozen codes there are406 pair constraints:

| Constraint | Frozen coordinates | Required slope |
| --- | --- | --- |
| Lower, source24 ->28 | Source1176000 ->1368000; recorder[1184026,1184027] ->[1375999,1376001] | `b >= 47993/48000` |
| Upper, source0 ->1 | Source24000 ->72000; recorder[32843,32844] ->[80757,80758] | `b <= 9583/9600` |

The lower bound exceeds the upper by exactly `13/8000`. These are
model-coordinate slope constraints, not calibrated clock drift. The first
adjacent source interval is48000 frames versus a recorder interval of
[47913,47915], residual[-87,-85]. The last is48000 versus[47993,47996],
residual[-7,-4]. Neither residual is an end-to-end latency or a loss count.

## Tool and verification

`scripts/vad-indexed-spacing.py` accepts only an exact indexed-v2/48kHz/stereo
schedule with3..64 complete nonoverlapping markers and the existing complete
batch schema. Every global Unique identity must be present exactly once and
agree with its individual unchanged-model result, coordinate interval and
score. Per-call work must be within256million; the exact charged sum must
match the explicit aggregate cap, bounded at7billion. No correlation work is
performed by this tool. Coordinates/counters have strict integer bounds,
input reads are capped at4MiB each, duplicate JSON keys/nonfinite constants
refuse, and output is create-new with input length/SHA-256 pins.

Fourteen initial tests fail because the diagnostic does not yet exist. These
are missing-implementation assertions, not a previously incorrect native
algorithm. The first implemented run has13 passes and one failure: the test
expects an unreduced fraction where `Fraction` correctly reduces it. That
expectation is corrected, not the algorithm or evidence. Fifteen tests then
pass; the final expanded suite passes16. Coverage includes exact interval
boundary feasibility, nonconstant and reversed mappings, large recorder
translations, permuted window/source order, complete identity refusal,
strict schema/type/work checks, source/global/individual consistency, bounded
reads, input pins and create-new/refusal behavior. A separate direct
intercept-intersection formulation checks64 small synthetic interval sets
against the pairwise result without relying on floating-point fitting.

The final checks pass:

```text
python scripts/tests/vad-indexed-spacing.tests.py       # 16 passed
python scripts/tests/vad-capture-qpc.tests.py           # 17 passed
python scripts/tests/vad-clock-rebase.tests.py          # 15 passed
python -m ruff check scripts/vad-indexed-spacing.py scripts/tests/vad-indexed-spacing.tests.py
cargo fmt --all -- --check
git diff --check
```

CI gains an offline spacing-test step beside the two existing evidence tools.
No Rust, driver, model, deployed probe, buffer or default changes occur; no
Cargo test/Clippy, remote CI or native result is claimed for this script-only
slice. Original model/packet/runtime diagnostics remain authoritative for
their own scopes.

## Provenance and next action

Repository base is `132b1e0`. Host evidence root is
`D:\coding\moiren-evidence\20261101-indexed-spacing`. The pre-analysis hypothesis
freezes ten source/evidence inputs and the exact interval method. The script
is tested and pinned before one finite invocation against the retained model
report. Its exit0 means successful diagnostic computation, even though
`constant_affine_compatible=false`; all output keeps `qualified=false` and
`native_execution=false`. Result SHA256 is
`EF5C7309CCFF89C25F740C40C54ABF9DEC03111DC025D3B2F0415D4019649CEC`.

A separate53-check arithmetic/integrity audit recomputes all406 constraints,
the conflicting witnesses and28 adjacent intervals, and checks preserved
input hashes and output scope. The script is byte-identical after execution;
later additions only strengthen tests. Original PCM, packet metadata, report,
collection, source and batch files remain unchanged. Nominal14/29 identity
refusal,9686 packet-QPC diagnostic violations,14 unknown boundary polls and
incomplete runtime quality are neither erased nor converted to zeros.

For subsequent S1 waveform/clock work, a single global phase/rate correction
is insufficient to reproduce all the frozen model coordinates. The next
discrimination needs time-varying mapping evidence or an independently
validated explanation of model-coordinate bias, not another global fit or
post-outcome tolerance. This result alone does not select a production fix.
Native tail-fault verification, lifecycle attribution, whole-waveform quality,
sustained lower-budget delivery, matched VAD/Process comparison and calibrated
physical acceptance remain open. S1-S3 and the overarching goal stay active.
No native replay, remote/audio process, physical render, driver/system-control
write, push or subagent work occurs; unrelated `gate.log` is preserved.

The subsequent [known production SRC center control](2026-11-01-src-clock-trace.md)
demonstrates the time-varying possibility on a fixed synthetic two-stage
servo/FIR fixture with an independent center/sample oracle. Its unchanged
model intervals contain all three known centers while their spacing excludes
a constant map. This does not reconstruct this historical recording or select
between its actual mapping and model-coordinate bias.
