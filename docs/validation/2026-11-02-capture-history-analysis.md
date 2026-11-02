# Native capture packet-file analysis integration

Date: 2026-11-02, Asia/Shanghai. Parent: `182feba`.
This follows the [bounded packet-history acquisition](2026-11-02-capture-packet-history.md)
increment. It closes the concrete file-format gap between `record-process` and
the existing PCM/indexed analyzers, without altering production audio or any
correlation/quality threshold.

## Exact scope

The opt-in `capture.packet-history.jsonl` now starts with a version1
`moiren_native_capture_packet_history` header containing exact schema3 capture
aggregate metadata. Subsequent records keep the previous raw native offer
fields unchanged. A self-contained header is necessary to distinguish a complete
prefix from a truncated file or intentionally omitted packets. An aggregate
alone still cannot replace the actual records.

`PacketTimeline::read` recognizes this framing only at the first record and
otherwise preserves the original measurement-sink format. There is no permissive
shape guessing, unframed-native import, synthetic epoch or automatic conversion
of old aggregate-only captures. Header, aggregate, summary and packet schemas
deny unknown and duplicate fields. Explicit null QPC stays null; a missing QPC
field is malformed, and native zero remains zero.

The reader checks both observed and stored frame frontiers, exact storage
prefix behavior, declared PCM length/format, packet count/capacity, prefix
truncation, omitted suffix frames, whole offered/stored/dropped totals, and
flags/counter consistency. A suffix that cannot explain aggregate silence,
overflow, discontinuity, timestamp or position-gap counts is refused. The same
byte/packet/1024-byte line caps apply before unbounded reading; the header counts
toward bytes, not the packet cap.

Valid but incomplete storage, omission or saturation remains explicit invalid
coverage; malformed or contradictory files return an input error. Existing
packet sequence, flags, QPC interval and position diagnostic refusals are reused
unchanged. Timestamp projection for internal legacy diagnostic arithmetic does
not serialize a fabricated capture stamp: raw nullable records are retained in
startup/fault observations. Native-history evidence uses schema2 with the exact
header and scope; old sink evidence stays schema1 with its previous fields.

Full record retention means **GetBuffer offers before Release**, not successful
Release, native completion, source/sample age, hardware time or physical
qualification. A complete header never overrides a failed capture report.
Process zero positions remain unknown. First-packet investigation stays deferred;
retaining existing first-packet diagnostic issues is not a new investigation.

## Regression evidence

An actual unsupported-format RED fails five focused cases before implementation:
the prior reader refuses the new header as an unknown `capture` field. A second
behavioral RED shows an inconsistent omitted suffix being accepted as parseable;
the corrected reader refuses it. Both failure receipts are retained.

Coverage includes original sink compatibility, native/unknown/null/zero/error
time, raw flags and positions, sequence/QPC refusals, partial/zero PCM storage,
omitted suffix accounting, saturation, bad versions/rates/channels, wrong
frontiers/totals, missing/extra/mixed records, duplicate fields and resource caps.
One initial GREEN attempt fails because the mutated position fixture did not
update its actual two gap counters; the fixture is corrected to events2/frames214,
not by weakening production validation.

An end-to-end file test creates PCM using the production indexed-v2 source,
records actual recorder packets, persists through the real create-new writer
and invokes the existing `analyze-indexed-pcm` CLI. Exact complete input resolves
the original three identities. Omitted packets, unknown time and truncation keep
the identities diagnostic but refuse complete acceptance. Qualification remains
false, source/recorder origin unknown, and no delay is fabricated. A separate
existing PCM-delay analysis keeps a known23-frame synthetic offset only for
complete storage and invalidates all windows after storage loss.

Checks use the existing clean export plus only this increment's four code/test
files, not the 18 preserved dirty paths. A separate bounded read-only code review
checks this increment; it does not review or modify the other drafts.

## Software verification

Fresh final example regression passes678tests with one ignored, including
13capture-history tests and the real recorder-to-indexed-CLI roundtrip.
The focused recorder suite passes13tests. Strict example and workspace
all-target Clippy, workspace formatting and the release probe build exit0.
The pre-review resumed example run passes674/1ignored in14.76s; the final
post-review run passes678/1ignored in14.85s. Workspace fmt exits0. Full workspace
regression passes2178/8ignored across71suites and measured UI passes20tests.

The separate static review finds three concrete strict-reader gaps: enabled
position-gap aggregates could be ignored for all-zero positions, omitted offers
could claim impossible overflow counts, and omitted frame totals lacked the
native u32-per-packet bound. Three added regressions fail behaviorally before
the fix. The reader now invalidates declared position gaps, checks omitted
overflow feasibility against remaining storage and nonempty bounded offers,
and performs overflow-safe u128 size checks. The recorder roundtrip fixture now
uses genuine increasing positions instead of declaring a gap-enabled zero
timeline clean. Process capture's actual gap-disabled zero positions stay
unknown, unchanged. An exhaustive small-offer test independently enumerates
possible omitted suffixes and checks acceptance against the arithmetic. One
initial overflow fixture incorrectly assumes omitted sizes are known; it is
corrected because an unknown9+7 suffix can legitimately overflow twice where
the original8+8 suffix overflows once. No production gate is weakened.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Receipts use `s1-packet-format-`, including `probe-resume-1102.log`,
`workspace-clippy-1102.log`, `clippy-final-1102.log`,
`fmt-resume-1102.log`, `workspace-resume-1102.log`, `ui-resume-1102.log`,
`review-red-1102.log`, `review-final-1102.log`, `review-clippy-1102.log`,
`review-fmt-1102.log`, `workspace-clippy-complete-1102.log` and
`release-complete-1102.log`. This is software regression,
not native waveform or physical qualification.

The final release rebuild exits0 in16.11s, SHA256
`AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685`.
It is not substituted into the failed-attempt deployment below. All four code
files match the tested clean export, and all18original dirty paths remain
byte-identical; `s1-packet-format-code-audit-1102.json` retains those pins.

## One remote acquisition attempt, incomplete

Only one bounded10s source-only Process capture feature attempt is launched,
not another graph stability run or parameter scan. The target is
`LAPTOP-NP49RG6H`; no host audio is opened. Its separate deployment is:

```text
D:\MoirenTest\20261102-packet-format-0935\e2e_latency_probe.exe
7485952 bytes
SHA256 D777F272BCAA0492F997E73095969BC800FAB8121B4D3173993AFA9A8E7E3A61
```

This deployment predates the three control-side reader review fixes; it is
retained unchanged as the exact failed-attempt binary, not labelled final code.
The source opens the explicitly selected
48kHz stereo Shared Float32 endpoint `Voicemeeter In 2 (VB-Audio Voicemeeter
VAIO)`, ID `{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}`,
with1056native buffer frames. No graph/application owner, driver, default,
volume or OBS setting is changed. Task lifecycle is `task`, with a3min ceiling.

The task launches09:32:42.053 Asia/Shanghai. Its runner operates09:33:12.809
through09:33:15.378. Source26544/creation100ns134380567934872201 reaches Ready
but exits with `invalid source token` before `running.json`. The runner's
`Publish` helper created an empty `activate` file; the existing source protocol
requires exactly one byte `1` and atomic publication. This is a harness protocol
error, not an observed Windows/device defect. The source never calls native
Start, acknowledges zero source frames and submits no scheduled nonzero frames;
only1056silent preroll frames were released. No retry is launched.

Recorder23548/creation100ns134380567943854348 joins after target exit:
status `target_exited`, elapsed0.7629066s, `recording_complete=false`, artifact
persisted, native Stop succeeds. It retains74actual480-frame offers and35520
frames of all-zero stereo PCM, with a557-byte aggregate and13421-byte framed
history. History storage is complete for this **partial capture interval**;
that never means the requested10s capture or source playback completed.
Both owned processes exit, cleanup reports no forced kill/error, and the task
ends Ready/result1. OBS12648/start1793505454949 and QQMusic15096/
start1793504036422 remain unchanged. Final read-only audit finds no owned probe,
unchanged defaults and no endpoint volume/mute changes.

## Offline readback of retained native evidence

One remote offline Python3.10.11 task uses the existing isolated numerical
environment with `-I -B`; no audio is opened. It uses streaming SHA256 rather
than unavailable Python3.11 `hashlib.file_digest`. Inputs are hash-checked before
and after. The production `analyze-indexed-pcm` CLI reads the actual74-row file
and a fixed2048-frame first recorder window with unchanged0.7NCC/0.05margin/
0.0001RMS gates. It exits1 as expected: no source identities, no estimated
source origin, no delay and `qualified=false`. Packet evidence schema2 reports
the actual stored coverage, with74unknown zero positions, no fabricated epoch,
and no timestamp/flag issues. The absence of packet flags does not turn the
all-zero samples or failed source startup into a complete recording.

The0.407s offline task produces a visually reviewed two-panel PNG: the retained
PCM is flat zero and raw packet QPC differences are10ms. These are diagnostic
packet intervals, **not end-to-end latency**. Only compact analysis/summary/PNG
files are downloaded; raw PCM stays remote. Lengths and SHA256 match:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `summary.json` | 4849 | `4D9AD732DD3468D61C4920FDF1034112B57E8CA42DB6AB9786666453E4357CC0` |
| `analysis.json` | 3680 | `E922F10844B2E5B5777CB1C526621F2FE9F43B66E3A61A5D22C26B936C8C8E56` |
| `partial-packet-evidence.png` | 72249 | `71DFD038C8390420F7E52CF35E5399E21F9ED95F0854F61850B9A63E4F62F99A` |

Remote artifacts remain below `run\partial-offline` in this deployment; compact
host copies are below `s1-packet-format-partial-offline` in the evidence root.
The native `result.json`, source final, capture report, aggregate and history
are separately pinned by `s1-packet-format-final-audit-1102.json`.
The earlier read-only SSH handle stalls; a separate bounded read-only inspection
retrieves the already-terminal task's files, without relaunching it.
The original stalled observation handle is subsequently interrupted and exits1;
this does not stop or restart any remote audio process.

The subsequent [corrected native roundtrip](2026-11-02-capture-history-native-roundtrip.md)
uses a new hash-pinned deployment and one-byte atomic controls. It completes
the requested capture, resolves5/5codes on both channels and verifies the full
retained source PCM bit-for-bit. This does not overwrite the incomplete attempt
above or turn source-only diagnostics into graph/physical qualification.

## Qualification boundary

This is acquisition-to-analysis plumbing, not a new detector, epoch journal,
rate/servo model, parameter scan or lower-default proposal. Old raw packet-only
files from the previous increment remain strictly unsupported, rather than
retroactively assigned a header. Existing aggregate-only VAD captures and their
refused identities are unchanged. Neither S1-S3 completion nor physical latency
follows. No push or memory update.
