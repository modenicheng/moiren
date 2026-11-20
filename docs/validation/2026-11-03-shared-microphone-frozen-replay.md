# Shared VAD microphone: frozen dual-FIR recording replay

Date: 2026-11-03, Asia/Shanghai. Source checkpoint: `d58a65f`.

## Result and scope

One remote **offline** analysis of the already closed
[Shared VAD Playback -> unity Bus -> VAD MIC recording](2026-11-03-vad-source-mode.md)
finds an exact sample match under one frozen recorder-to-projection mapping.
All 327552 retained stereo recorder frames equal the decoded PCM16 projection,
including 327027 nontraining frames. Both channel gains are exactly 1, DC 0 and
maximum/RMS absolute error 0. All six pulse-support windows are compatible;
there is no per-window reanchor, rate fit, phase fit or interpolation.

This explains this recording's content and peak transformation conditionally
on its archived prepared route and encoding. It is **not** a new acquisition,
continuous-delivery qualification or physical latency measurement. The original
native collection remains refused: 4 discontinuity flags and 2 device-position
gaps totaling 2208 frames. First-packet investigation stays deferred. No quality
gate, buffer, default or production path changes.

## Archived inputs and reconstruction

Original native root:
`D:\MoirenTest\20261103-vad-source-mode-0230\native-run`.
The input witness is PCM16; the independent Shared MIC recorder is Float32.
The pinned collection's `/graph/admission/current/microphone/shape` is exactly
`["Stereo", "Some(Pcm16)", 480]`. The parent verifies that condition before
requesting PCM16 output projection; the helper does not verify it itself.
There is no retained native output-submission witness.

Unlike the earlier [nominal physical-output replay](2026-11-03-vad-playback-frozen-replay.md),
this recording includes both SRC boundaries: 5190 callbacks each, 10380 total,
six chunks/7629581 bytes. The archive validator checks every checkpoint,
sequence-one coverage, finalized/writer-finished status, and schema 2 prepared
route identity: source 0 -> unity Bus 1 -> sink 2, revisions 1/1, owner 1,
invalidated=false.

The external helper reuses unchanged production FIR/codec snapshots and the
unchanged reconstruction library. Its adapter now separates metadata direction
from sample encoding and supports an explicit projection-only invocation; it
does not manufacture sink metadata or an output witness. Its receipt preserves
source hashes and genuine RED/GREEN logs.

Reconstruction processes 5182 paired callbacks and stops **before** sequence
5183's first Playback epoch/reset barrier at Engine timeline 331648. It retains
21696 startup-zero Engine frames and 32 explicit priming-muted frames. Of 312480
published input frames, 310018 are consumed and 2462 remain unconsumed. The modeled
output prefix contains 331688 stereo PCM16 frames; later callbacks are not
invented or carried across the barrier.

## Frozen comparison and full denominator

The first component trains the single integer map:
`projection_frame = recorder_frame + 2208`, with gain 1/DC 0 on both channels.
Five subsequent components retain that map. All 327552 recorder frames lie
within the modeled prefix; 2208 projected leading frames and 1928 trailing frames
have no recorder comparison. This stored-frame offset is **not source-clock
delay**, and its numeric equality with the position-gap total does not establish
causation, lost audio, or the absence of lost audio during silence.

Every recorder frame is counted, including startup, training and silence:

| Category | Stereo frames |
| --- | ---: |
| Informative fixed-map compatible | 8192 |
| Uninformative signal | 318720 |
| Anchor training | 525 |
| Unsupported block context | 115 |
| Waveform residual | 0 |
| Outside/unknown/generation/nonfinite/clipped/partial tail | 0 |
| Total | 327552 |

The raw-bit comparison includes training, silence and mixed-context frames;
all 327552 match, with 0 unequal/0 incomparable frames. Excluding training leaves
327027 matching frames. These arithmetic facts do not upgrade 318720 silent/
uninformative frames or 115 mixed-context frames to informative identity proof.
The 64-frame gates retain 128 informative-pass blocks, 4980 uninformative blocks
and 10 training-overlap blocks. No quality/identity flag is promoted, and
physical latency remains null. Original native flags/gaps are shown separately
on the reviewed plot, not erased by waveform agreement.

## Execution result: closed children, failed outer receipt

Remote analysis root: `D:\MoirenTest\20261103-shared-mic-replay-0730`.
Task: `Moiren-SharedMicReplay-20261103-0730`, task lifecycle, 300s outer limit.
Compute is bounded 210s, with separate final audits inside 285s; held children,
isolated TEMP/MPL and the existing isolated Python environment are used.
No audio, desktop, OBS or device operation is started by this analysis.

All 11 held child commands exit 0, with no forced child termination or cleanup
error. However, **the outer task ends 1**: the wrapper assigns a returned
`WindowsPath` into its result and fails JSON serialization. `run-result.json`
is truncated at `"archive":` and remains unchanged. It is not a complete JSON
receipt and is not reported as task success. Independent verification decodes
only its already complete command array, then validates closed child outputs,
before/after original-input fingerprints and safety receipts separately.

The future wrapper correction is external-only in `run-next.py` and its tests.
A real Path-return mock reproduces the serialization failure before repair;
19 tests then pass. That correction is **not remotely deployed or rerun** and
does not repair or supersede the original task 1. The retained complete output
summaries and plot remain usable at the stated diagnostic scope.

The adapted Rust helper passes 33 tests in debug and release, independently
rerun by the parent. The adapter passes 5 tests, original orchestration 18,
held-runner 16, frozen-map 29 and archive-validator 25 tests. The remote task
independently runs 18+16+29+5 tests with exit 0. Strict helper Clippy fails inherited
unchanged `manual_is_multiple_of`/`items_after_test_module` warnings; a run with
only those two lint categories allowed passes. No unrestricted strict helper
Clippy pass or fresh production-workspace test result is claimed.

## Closed evidence and final safety audit

Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-shared-mic-replay-20261103-0730`.
Only compact JSON/log/scripts/PNG transfer; original raw PCM and SRC chunks
remain remote. The compact ZIP contains exactly 34 payload files plus manifest,
116417 bytes, SHA256:
`AB5624F5409A972643FAB1101486472C0680997B1F04AA49E2639DA950FCE847`.
Every entry's size/hash, duplicate/extra-file exclusion and the actual PNG are
verified. Key extracted files are:

- `archive/output/summary.json`: frozen association and full-frame accounting.
- `archive/projection/summary.json`: reconstruction/barrier and witness limits.
- `archive/output/shared-mic-frozen-accounting.png`: reviewed waveform,
  preserved original recorder flags/gaps and complete category strip.
- `independent-output-verification.json`: separate closed-output audit, task 1.
- `final-remote-audit.json`: independent final pins/process/driver audit.

At 07:38:19+08:00, all 8 original inputs, 14 deployment pins, 34 closed outputs and
the ZIP remain unchanged. The task is Ready/result 1; no test audio/replay/Python
owner remains. Original OBS 12648 and QQMusic 15096 creation identities are
unchanged. Driver state is Running, on-disk SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`,
boot FileTime 134377986540731770; this is not a newly measured loaded-image hash.
Unchanged protected process identities do not prove uninterrupted user audio.
No host-native process is operated. A hung read-only preflight reader was
terminated only after exact held identity/path/creation/command-hash matching;
its separate failure/cleanup receipts are retained and final absence verified.

Independent bounded review finds no contradiction in the frozen mapping,
denominator, projection boundary or failed-wrapper distinction. The final host
verification reruns all 112 Python tests and both 33-test Rust configurations,
verifies every ZIP entry and all 27 preserved dirty/cache hashes, and checks
165 relative documentation links. Only three evidence documents enter the
commit; production code is unchanged from the prior integration checkpoint.

No unchanged native rerun or background soak follows. Independent development
and a purposeful, finite, isolated remote test may proceed in parallel under
the [scheduling instruction](../vad-low-latency-goals-2026-10-28.md#2026-11-03-development-and-test-scheduling).
Retained-prefix/desktop acceptance, sustained native delivery and calibrated
physical qualification remain separate open gates; this result does not make
S1-S3 complete.
