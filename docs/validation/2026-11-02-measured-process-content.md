# Measured Process replacement: partial content acquisition

Date: 2026-11-02, Asia/Shanghai. One finite remote acquisition answers a new
question left by the earlier ten-second source: does changing content remain
observable after measured-owner replacement? The operational sequence completes;
the graph's strict identity-window admission refuses. This is not a waveform,
native-quality, desktop or physical-latency PASS. `qualified=false` throughout.

## Pinned execution

Remote `LAPTOP-NP49RG6H`, elevated SSH session0, 13:31:37.851-13:32:22.337.
This is not the Limited Interactive desktop session. Audio/recorders run only
there, in `D:\MoirenTest\20261102-measured-content-1320`. The development host
continues the independent widget repair and headless checks, without opening
audio devices or a native desktop window. No OBS/QQMusic operation is performed.

- Application source clean `78967f4`, the existing measured CLI release:
  `D:\MoirenTest\20261102-desktop-lifecycle-1222\bin\moiren-app.exe`, SHA256
  `4C96B3BB7284AC3721616863FE0D83BC13798925E00E85F996BBE5A30E9D51C4`.
  This acquisition does not exercise the later UI-only `6796878` repair.
- Probe SHA256
  `AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`;
  Running SYS
  `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
- Same source Voicemeeter In2 and physical render DAWN PRO2 endpoint IDs as
  the [prior CLI receipt](2026-11-02-current-measured-cli.md). Graph exactly
  Process input -> unity Bus -> DAWN, Engine48kHz/F32/block64,
  Shared336/capacity738/client3/render512/wake64/callback64, no fallback.
- Fresh Compatible10000ms baseline, exact measured1536/capacity16384/jitter96,
  then20000ms of the confirmed replacement owner. Empirical packet480/gap532
  frames; no lower candidate, queue increase, gate relaxation or default change.

Source PID19900/creation134380711004405413 and app PID25924/
creation134380711008395629 are exact identities. Baseline generation1 drains/
reaps before app generation2. Owner2 is observed at13:31:51.2156856; held source
and graph Process-tree recorders launch afterward and join at13:32:03.5391575,
before app shutdown. Native generation1 is backend-local; app generation and
the confirmed active request, not native generation alone, establish replacement.

The indexed-v2 source lasts40s/1920000frames,39markers of1024frames at0.025.
It stays alive through app/recorder joins and submits the complete schedule
before normal stop. Source/app exit0. Both12s recorders deliberately exit1
under the recorder's unqualified-report contract, with complete persisted
artifacts and no reported failures. Stored source575040frames and graph575232
frames are actual counts, not padded to requested duration. Readiness is not
proved by process launch. Wall ordering/submission receipts do not align audio
origins or prove individual marker presentation.

## Results and retained limits

The final replacement-owner bridge retains1999packets/959520captured/published
frames. Drop, unexpected-silent, nonfinite, underrun, discontinuity, timestamp,
reset, packet/gap-bound violation and recovery counters are zero. Render writes
960640frames, with zero drop/silent but **one wait timeout retained**, not located
within run versus teardown. Startup priming3072 and prime-discard384 remain
separate. Fill966-1989/final1343 and correction+28.384ppm are adaptation
observations, not calibrated drift or residence; oldest frame age remains null.

Closed offline analysis uses the unchanged Rust detector and .7NCC/.05margin/
.0001RMS gates. All39source codes compete; no identity is assigned from order,
and no schedule, PCM, support component or threshold is edited.

| Capture | Result per channel |
| --- | --- |
| Source output |12support windows,12unique codes11-22; both channels agree |
| Graph output |12support components; first padded window673frames is shorter than1024; entire channel batch refused before detector invocation |

The remaining graph components are1183-1184frames. None is omitted, widened,
fitted or re-recorded to obtain a pass. The short component is not established
as packet loss, a first-packet defect, source identity or sustained corruption.
Its cause remains unclassified. Source codes outside the12s subwindow are
outside-or-unresolved, not inferred missing content. The graph visibly contains
changing nonzero activity but its identity correspondence is **not established**.
This leaves the content question incomplete, without creating grounds for
production audio changes or another unchanged soak. First-packet work stays
deferred; the earlier Process18/19 and VAD17/19 refusals remain unchanged.

## Evidence and tooling corrections

Host root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-measured-content-20261102-1320`.

The original no-audio selfcheck refuses the oversized historical aggregate
report before launching any child. A narrow corrected check reads only the
existing small recorder reports, then verifies PS5 parsing/functions, three
atomic receipt generations and a held `--help` child. It passes before the
single native acquisition; the failed receipt is retained, not overwritten.

Original offline analysis fails before NumPy/detector/output on Python3.10
parsing the log's spaced timezone. The first correction's unit check then
rejects PowerShell's seven-digit fractional receipt before analyzer launch.
Both failures remain immutable. A separate `analyze-time100ns.py` changes only
timestamp parsing: explicit offset timestamps become exact integer100ns UTC
ticks, preserving the seventh digit without float conversion/truncation. The
AST-only tests cover real literals, UTC equivalence,100ns distinctions and
naive/unsupported-precision rejection. This is operational wall ordering, not
clock calibration or an audio-time origin estimate.

The corrected remote run uses isolated Python `-I -B`, pinned virtualenv,
single-thread numerical libraries and output-local plotting cache; it exits0,
charges123641856correlation samples, and preserves every raw input hash.
Graph admission refusal remains explicit, not zero detector misses. The first
generated plot's empty graph panel could obscure admission, so a separate
closed-summary plot labels **NOT ANALYZED** and shows all support lengths.
It reruns no detector and changes no original artifact. Both plots remain.
`content-admission-overview.png` is177742bytes, SHA256
`A9934620D68291DFA3B5441295B875DEFBD1BA2246F4A6DA074276277BA24100`, verified
after download and visually reviewed. Raw PCM/large histories stay remote.

The13:40:47.739read-only final audit finds no native/analysis owners, unchanged
raw-manifest lengths/hashes, unchanged protected OBS12648/QQMusic15096 creation
identities and pinned app/SYS. Native closure separately confirms unchanged
default endpoints/render controls and no forced cleanup/errors. All eighteen
preexisting dirty-file hashes and both Python cache directories remain intact;
main stays clean. No push or memory update. Current desktop/UIA acceptance and
waveform/native/physical qualification remain open; do not promote defaults or
claim a path winner/physical delay from this diagnostic.
