# Application encoded-render evidence collector

Recorded 2026-11-03, Asia/Shanghai. Baseline `99fc4ec`.
This connects the [native render hook](2026-11-03-native-render-evidence-hook.md)
to the actual backend owners and production Process monitor. Storage, native
submission, independent recorded waveform and physical latency are separate
verdicts. S1-S3 and physical acceptance are not declared complete.

## Product scope

- `--render-evidence-frames N` is explicit diagnostic opt-in, requires a fresh
  `--cursor-trace-directory`, admits 1..131072 frames, and must also cover the
  negotiated complete native packet within the existing 8 MiB storage budget.
  It changes no saved setting, audio target, native period or default policy.
- Both precision factories carry the option through measured owner replacement.
  Backend readers are claimed once from an immutable retained session. Reused
  backend-local generation/output labels cannot redirect an older reader.
- The native factory reads metadata before installing the writer last. Normal
  stop, parked cancellation, preparation failure and unwind return it through
  native join. Only the control-side joiner publishes its terminal tally; close
  or native destruction alone is not enough. Publication is caller-asserted
  lifecycle provenance, not authenticated proof of successful Stop or playback.
- Each owner's `native-render` directory contains `stream.json`, `samples.bin`,
  `packets.jsonl` and `final.json`. Encoded container bytes, packet sequence,
  frame count, signed ReleaseBuffer HRESULT and stored-frame offset are exact.
  Rejected native submissions remain evidence, never a quality success.
- The independent archive worker uses bounded 4096-frame batches/pending data,
  a finite terminal drain and 512 MiB per-owner disk budget including receipts.
  CreateNew refuses overwrite. Sequence/offset/layout/tally errors, lost or
  unobserved evidence, saturation and IO failure refuse completeness. A failed
  payload remains owned; partial physical writes are not committed or retried.
- Render setup/finalization failures retain already-created input PCM and cursor
  owners and their independent final receipts. Required-but-missing evidence is
  explicitly incomplete, not silently relabelled as disabled instrumentation.

No native audio/UI or existing-process manipulation runs on the development
host. First-packet investigation remains deferred. No buffer enlargement, gate
relaxation, unchanged soak or missing physical equipment blocks development.

## Software verification

Tests are executed from an exact baseline archive plus explicit intended source
overlays, excluding 27 preserved dirty/cache files. Initial API/CLI RED receipts
are retained. Actual library linking catches a test-only publication wrapper
left enabled in non-test builds; it is narrowed to `cfg(test)`. The standalone
RT shadow then catches a module-relative-path mismatch; the same explicit path
convention as adjacent observation modules fixes it without changing mechanics.

The integrated focused audio suite passes 42 tests with one remote native test
ignored. This includes nine real backend attachment/lifecycle cases for both
precisions, exact negotiated layouts/caps, explicit preparation refusals,
same-generation replacement and join-only terminal evidence. CLI passes 95
tests including 21 archive tests and required-but-uninitialized finalization.
The corrected standalone RT harness passes 249 tests, zero failures.

An independent archive review reports no confirmed finding. A separate review
finds that render initialization failure could stop independent PCM/cursor
collection and miss the replacement owner. The repair retains the session before
render setup, keeps the original error sticky, explicitly refuses required but
missing render evidence, and aggregates sealed-owner failures without truncating
later owners. Unsealed persistence failures remain terminal. The reviewer
rechecks this bounded repair without another confirmed finding.

The isolated workspace run passes 2425 tests, with 10 ignored and zero failed,
across 71 harnesses. Its application harness was compiled before the last parent
archive repair; the final-source 95-test CLI suite independently covers that
repair. All-target workspace check and strict Clippy, both with
`moiren-app/gpui-test`, exit zero against the final source. A separate final fmt
check exits zero. The final release build exits zero. The additional final-source
`cargo test --locked -p moiren-app --features gpui-test -j1` exits zero at
13:32:41: 757 passed, four ignored, zero failed across 13 harnesses, including
all 20 measured-UI tests. The library harness alone passes 698 tests; CLI95 is
included. This separately closes final-source app/headless coverage; the earlier
full-workspace result is not relabelled as that run. The workspace run also
covers the five zero-refill regressions and the latency-marker, marker-rate and
qualification integration harnesses requested by the user.

## Evidence locations

Host:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-render-collector-20261103-1243`.

Prepared remote native root:
`D:\MoirenTest\20261103-render-collector-1243`.

Separate remote device-free test root:
`D:\MoirenTest\20261103-render-collector-software-1306`.

The device-free full-library attempt outlives its original 110-second wrapper
deadline. Its later closed log stops mid-test without a terminal summary and
the process is absent. No exit code, full-library pass or cause is inferred;
the incomplete attempt and log/hash remain. No relaunch-for-green follows.
Protected OBS/QQMusic creation identities remain unchanged. This is not evidence
of a native audio fault, and does not replace clean-source host verification.

## Actual application acquisition

One distinct remote native acquisition runs on `LAPTOP-NP49RG6H`, from
13:19:26 to 13:20:11 on 2026-11-03. The scheduled task finishes Ready/result 0.
The app, deterministic source and isolated portable OBS all exit zero without
forced cleanup. Production monitor reports run/cleanup complete and no failure;
stderr is empty. No second acquisition or background audio soak follows.

The chain is the deterministic indexed-v2 source on an isolated Voicemeeter
endpoint, captured by the app's Process input, through a unity Bus to DAWN PRO2.
Independent OBS captures DAWN output as stereo 48 kHz PCM24 in MKV. This is a
recorder/software boundary, not DAC output or a calibrated physical loopback.
The 20-second excitation remains active through app join. The app measures a
two-second Compatible predecessor and then runs its confirmed measured
replacement for 8001 ms. Engine64/f32, Shared336, native capacity738, render
target512, wake64 and callback64 remain fixed. The admitted input replacement
is target1536/capacity16384/jitter96, not a new default.

The exact new app binary is 34305024 bytes, SHA-256
`1ACB06D18E00CC3B7F7444D3338C7F9DEAE7A93A1645D0BD9D684BE7865DC6C5`.
It is available at
`D:\MoirenTest\20261103-render-collector-1243\moiren-app.exe` on the target;
the matching host build is in the evidence root's `native-package` directory.

Both owner directories retain complete input PCM, cursor and encoded-render
receipts. Local generation/output labels are identical; directory ordinal is
the owner identity, so their evidence is never concatenated.

| Owner | Release attempts | Encoded frames | Input PCM frames | Cursor events |
| --- | ---: | ---: | ---: | ---: |
| Predecessor | 289 | 97120 | 95040 | 2908 |
| Replacement | 1145 | 384896 | 383520 | 11940 |

Accepted, stored and persisted encoded counts agree per owner. Both summaries
have zero rejected, dropped, overflow, reader-abandoned, invalid, unobserved and
pending-abandoned evidence; no saturated/exhausted counters or archive errors.
These are submission/storage results, not audible-continuity verdicts. The input
and render frame counts describe different boundaries and are not equated.

## Independent waveform result

Remote-only offline analysis losslessly extracts MKV audio stream 1 using codec
copy: no cropping, resampling, downmix or channel permutation. The full recording
contains 1075200 stereo frames. Original MKV, derived PCM, source receipts, tools,
manifest and analysis outputs have before/after length and SHA-256 pins. The
frozen analyzer's 28 synthetic tests passed before actual analysis. Its actual
run exits zero, meaning accounting completed, not waveform quality passed.

The training/search plan is fixed before waveform evaluation, based only on
OBS/app lifecycle timestamps, at `analysis-plan.json`, SHA-256
`2AB8F07668A3A2440B728E7AA4144EB8760F4E2122701DA4CF4A373B55660261`.
Each owner uses source frames [48000,52096), with respective OBS searches
[144000,240000) and [240000,336000). Both complete search intervals are excluded
from both evaluation ledgers. The fixed model permits only integer association
plus per-channel gain/DC and the unchanged two-PCM24-step residual limit.

- Predecessor training associates uniquely: correlation 0.9999999999624334,
  peak margin 0.6702387354943646, recorder offset 185508 frames, and training
  maximum absolute residual about 6.044e-8. This offset is not latency.
- Its whole recorder support [185508,282628) lies inside the union of excluded
  searches. Therefore it has **zero independent evaluation frames**, despite
  the successful training association. The plotted 512-frame preview is also
  unscored; it cannot establish an evaluation pass.
- Replacement training does not associate: maximum correlation about 0.07431,
  peak margin about 0.001638. It is refused rather than refitted, searched again,
  granted a wider window or scored with a weaker threshold. The cause is not
  established by this result; it is not proof of dropped/corrupted audio.
- Each owner's full OBS ledger remains 1075200 unknown, zero residual and zero
  compatible frames. Native source ledgers likewise retain all 97120/384896
  frames as unknown. Zero residual here means no eligible evaluation, not no
  waveform error. Quality, continuity and physical qualification remain false.

The visualization is inspected at the host evidence root's
`actual-compact\alignment.png`, and remains on the target at
`D:\MoirenTest\20261103-render-analysis-1312\actual-render-1330\alignment.png`.
The 364379-byte compact ZIP and all 12 payloads are length/hash verified;
ZIP SHA-256 is
`94A2D6C50BF1C834E89377C7F0469D8E400A460F601C8641A872AF1FE2F16511`.
Raw MKV/PCM stay remote. This closes the actual app collector acquisition but
leaves the independent waveform verdict inconclusive; it does not block
unrelated product development or justify another unchanged audio run.

The final read-only remote audit at 13:28:44 confirms no owned processes remain,
the task is Ready/0, protected OBS/QQMusic creation identities are unchanged,
default endpoints and all 12 render endpoints' volume/mute/mix formats are
unchanged, and the same running driver/boot and bare Moiren root bus remain.
All original/derived/analysis pins still match. No process cleanup is needed.
