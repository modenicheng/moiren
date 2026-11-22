# Fixed-candidate section diagnostics

Date: 2026-11-02, Asia/Shanghai. Baseline `0d9a933`, low-latency branch.
This follows the [frozen-component diagnostic](2026-11-02-measured-component-identities.md),
not a new acquisition. The previous turn made progress by recording native
live/closed-prefix refusal acceptance. No previous run is restarted here.

## Question and boundary

Two retained graph windows have strongest candidates that clear global NCC
but fail section agreement. This analysis asks which unchanged quarter gates
explain that refusal at the already recorded candidate/onset. It does not
search another onset, fit rate/phase/gain, infer identity from source order,
change detector thresholds or establish a production audio defect.

The complete candidate's1024-point means are reused in every256-point quarter,
matching the original Rust detector. Signed correlation and correlation aligned
by the original single global polarity are distinct; each quarter requires
the original0.7 threshold and both centered RMS floors of0.0001. A large global
score cannot waive a failing quarter. Onset/identity margins and the original
raw verdict remain authoritative; fixed-onset metrics do not revalidate them.

All46 existing channel-window observations are included:24 source and22 graph.
These are not46 independent audio events, since both channels intentionally
carry the same sample. Each graph channel's original673-point component0
remains NOT ANALYZED, without a padded template or invented score. Strongest
candidates11/13 remain unaccepted candidates, not recognized identities.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-section-diagnostic-20261102-1600`.
Remote raw PCM stays in `D:\MoirenTest\20261102-measured-content-1320\run`;
the new isolated output is `D:\MoirenTest\20261102-section-diagnostic-1600`.
All14 original raw inputs and9 prior diagnostic outputs are pinned and must
remain unchanged. Only compact JSON/plots/logs are downloaded. No native
audio, GUI, recorder, OBS, device or driver operation is part of this analysis.

## Results

The single isolated offline run is16:02:18.014-16:02:21.805+08:00,
PID20884/creation134380801380314339, exit0 without forced cleanup. Python
computation takes3.563s within the90s held-process guard. No Rust detector or
native audio process is launched. The fixed comparisons cover47104 sample pairs and
184 quarter metrics, not a new all-codebook search. Every full confidence
agrees with its frozen raw score within1e-10 and every section-agreement
verdict agrees exactly. The original onset/identity margins are retained.

Both channels agree exactly within each recording, without becoming independent
references. All source quarters have NCC1.0. The nine previously accepted graph
windows retain all quarter NCCs at least0.831046. The two previously refused
graph candidates explain their disagreement as follows:

| Original component / unaccepted strongest candidate | Fixed recorder onset | Full NCC | Q1 | Q2 | Q3 | Q4 | Failed quarters |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 / 11 | 48950 | 0.711124 | 0.966211 | 0.875015 | 0.658886 | 0.326252 | Q3, Q4 |
| 3 / 13 | 145023 | 0.865244 | 0.640430 | 0.857072 | 0.953691 | 0.984110 | Q1 |

All these quarters retain both RMS values above0.0001. The refusals are
therefore explained by correlation-shape disagreement at the frozen onset,
not a silent-quarter RMS-floor failure. This is a detector-gate explanation,
not a cause attribution: fixed-template disagreement cannot distinguish
resampling, phase evolution, filtering or local sample damage. Original
strongest-candidate selection is not an independent validation, and the two
missing statuses stay missing. Unknown independent recording origins still
prohibit a route-delay inference. No sample-loss or physical-delay conclusion
is added, and no threshold is lowered to accept these candidates.

## Tool verification and artifacts

`scripts/indexed-section-diagnostic.py` is a127-line standard-library helper
for exact indexed-v2 generation and fixed-candidate moments. It rounds input
samples to f32, accumulates ordered f64 means/moments, retains global polarity
and reports undefined zero-energy correlations as null, never as zero/PASS.
Every output has `qualified=false` and `identity_accepted=false`. Bounds,
strict types, finite/unclipped input and equal-length admission precede work.

One focused regression genuinely fails on the missing helper assertion before
implementation. Afterward all11 tests pass, including contiguous xorshift
golden vectors, f32 rounding, gain/DC/inversion, erased quarters, global means,
remainder sections, null finite JSON and input/limit refusal. The parent
independently reruns11 helper,9 planner and16 related spacing tests, all
successfully. These are36 software checks, not a Rust workspace/CI/native rerun.

The reviewed `output\quarter-ncc.png` heatmap includes every analyzed channel
window and explicitly labels the two short graph components NOT ANALYZED.
`output\refused-fixed-waveforms.png` overlays raw, unscaled samples against
the exact nominal reference at the two frozen onsets. Neither image performs
alignment, gain normalization or phase/rate search. Null correlations are
masked and labelled undefined, rather than plotted as zero. Their SHA256s are:

- Heatmap: `1C6697A035A025BAA5DC8DE9A4BBB1EA0A1E7B020A1E1A051608322593B05043`.
- Waveforms: `6A31F82F5D50875CA936C95D24733B797D7065CC6FED0C79E3AEF3EDB930C63A`.

Helper SHA256:
`404D41EF162079EBA648E6272C66C3A0FF010FE5F7C8083225AB17509218F92A`.
Analyzer SHA256:
`B3E091C20901132E712ECE14540A48C107E7A9BB762898D00777EBC33C1F0FD0`.
All7 downloaded artifacts are size/SHA256-verified; ZIP937510bytes, SHA256
`6AA188DFF22A529A97197371EF4219DF9FA3841C2C4B23672048D56F0F4501DA`.
`parent-assertions.json` separately verifies score/verdict parity, channel
agreement, refusal accounting, quarter gates and all qualification flags.

Prelaunch review identifies two harness-only issues: null metrics in plotting
and lost output on timeout. Both are corrected before execution; a headless
held-child timeout selfcheck verifies stdout/stderr retention. The first
packaging attempt then fails on PowerShell5 `Measure-Object` over hashtables.
Its failure is retained and scalar byte summation fixes packaging only; the
analysis process is not rerun and no earlier output is overwritten.

The16:04:12 read-only audit verifies all23 original/prior input hashes, no
native/analysis owners, unchanged OBS/QQMusic creation identities and the last
native task still Ready0. The helper is also hashed before/after the analysis.
The original27 dirty/cache hashes remain intact and main remains clean.
The original waveform refusals, unlocated historical render timeout, native
lower-setting quality and physical qualification stay open. First-packet work
remains deferred; defaults and audio/RT/admission behavior do not change.
