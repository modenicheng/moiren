# Continuous Process cursor archive

## Scope

This S1 increment extends the actual interpolation suffix with a bounded
control-side collector and an optional independent CLI persistence worker.
It changes no audio samples, servo settings, recovery policy, admission limits
or defaults. S1-S3 are not complete. In particular, retained native PCM and its
accepted/offered-frame correspondence, render/OBS association and calibrated
physical latency are still separate work.

`InterpolationTraceCollector` validates snapshots, monotonic frontiers, suffix
sequences, overlapping payloads and adjacent non-overlapping span boundaries.
Missing sequences, malformed history, saturation and pending-buffer exhaustion
are sticky refusals. Draining cannot heal them. Writer closure and completed
reads are required for structural completeness; priming and failed read tails
remain on the requested-output timeline without fabricated interpolation spans.

Pending storage is limited to 1..131072 events, plus two native-sized history
windows. Chunks transfer ownership, not successful persistence. The native
collector privately pins the original bridge and process identity; replacement
registries cannot redirect its late reads. An unavailable final poll is not a
successful closure and can be retried.

The Windows monitor opt-in is `--cursor-trace-directory PATH`. It refuses an
existing directory, never overwrites files and leaves desktop settings alone.
A separate diagnostic thread polls every 2 ms and persists about every 50 ms,
using a 4096-event pending limit and at most eight retained owner sessions.
Writes use create-new, flush and `sync_all`; counts advance only after success.
On failure, the failed chunk remains owned, later pending storage stays bounded
and the final receipt reports failure. Diagnostic disk stalls cannot block the
service or RT threads, but can lose diagnostic ring coverage; this is refused,
not a continuity guarantee. Explicit finish and Drop stop/join the worker.

Per-owner `origin.json`, `chunk-*.json` and `final.json` separate native identity,
retained history and successful file writes. A root `summary.json` records
worker failure; the CLI preserves its normal post-join report before returning
a diagnostic failure. All receipts remain `qualified: false`. Neither a file
flush nor an owner pin proves graph continuity, PCM identity, native transport
cleanup or physical latency. Process exit/crash/power-loss recovery is not
claimed by this diagnostic.

## Regression and native discovery fix

An initial compile RED precedes archive implementation. The persistence tests
fail for missing `ChunkPersistence`, then pass with success-only accounting and
failed-payload retention. The 21 archive tests exercise an actual bridge,
44 missed events out of 300, capped pending storage, changed drained overlap,
invalid frontiers, writer lifecycle, terminal priming and cross-window order.
Three native tests exercise exact process/generation matching, late writes
after replacement, fresh closure and owner identity across reused generations.
Six app tests cover CLI parsing, new-file persistence, failure retention,
existing-directory refusal, ownerless stop/join and missing replacement-owner
failure receipts.

The first remote run exposed an actual discovery defect in this new feature:
application service generations 1 and 2 each used backend-local native
generation 1. Generation-only deduplication silently archived just the baseline.
Its directory and failed coverage remain preserved; it is not full-run evidence.
The fix compares retained immutable session identity (`same_owner`), and uses
an archive-local owner ordinal plus native generation in directory names.
The missing identity API has a recorded RED, followed by the passing regression
and the corrected two-owner native run below. No audio policy is changed.

Adversarial review identifies two further false-completeness paths, both with
recorded RED/green tests. Previously completed reads may not gain new events in
later snapshots; in-flight and later-read spans also respect the previously
requested frontier. A disk stall may hide an entire short-lived replacement
owner from discovery, so the final worker requires exactly the monitor workflow's
expected owner count (one ordinary, two measured). Missing whole owners now
refuse even if an archived baseline is structurally complete. These conservative
refusals do not stop audio. The pre-review feature smoke below is not relabeled
as testing these later checks; the reviewed binary has a separate fresh run.
Follow-up review also requires paired frontier deltas: requested frames cannot
grow without a new nonempty read, and added requested frames must cover every
new read. Its reproducer fails before the fix and passes afterward. The same-read
fixture models a genuinely in-flight read rather than an impossible frontier.

Final software verification after the paired-delta fix: workspace2143passed,
8ignored across69targets; measured headless UI20passed; strict workspace
all-target Clippy, fmt and `git diff --check` pass. The eighteen preserved dirty
files remain byte-identical and excluded. Receipts are
`s1-cursor-{workspace,clippy,ui,fmt}-final-delta-1030.log`.
The final validator tightening has focused/native synthetic regression coverage;
the remote reviewed binary described below predates only that tightening.

## Isolated remote feature smoke

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Native/device work runs only on `LAPTOP-NP49RG6H`; no development-host audio or
process control occurs. User OBS12648 and QQMusic15096 are preserved. Source,
portable OBS, PCM extraction and analysis stay remote. Only compact JSON and
PNG artifacts are downloaded and independently length/SHA-256 verified.

Original discovery-defect run:
`D:\MoirenTest\20261102-050910-s1-cursor`.
Corrected run:
`D:\MoirenTest\20261102-051848-s1-cursor-fixed`.
Usable app: `bin\moiren-app.exe`, SHA-256
`EB3F37CACE829A5EB6A620520B0B65BC5873C9701E1516CA0802B02D2CE66361`.
This is a source-manifest build including the eighteen preserved dirty paths,
not a clean-commit binary. The corresponding manifest is
`20261102-051848-source-manifest.json` in the evidence root.

Route: Python -> separate Voicemeeter In 2; Process Loopback -> unity Bus ->
DAWN PRO2. OBS retains independent Process-reference and DAWN-output tracks.
Shared output period336, render target512/wake64/callback64, Engine64 at48k.
Baseline10s precedes a measured target2048/capacity16384/jitter96 owner.
The owner runs60001ms with a46s/2208000-frame source and zero source underflows.
No driver, routing control, default role or saved desktop setting is changed.

| Archive owner | Events | Chunks | Requested frames | Completed/started reads |
| --- | ---: | ---: | ---: | ---: |
| 0001 / native generation1 / Compatible | 14930 | 191 | 481024 | 7516/7516 |
| 0002 / native generation1 / measured2048 | 89932 | 1141 | 2881088 | 45017/45017 |

Every persisted chunk is independently reread remotely. Both owners start at
event1, have zero missing/unretained events, monotonic spans, exact phase/step
bit strings, closed writers and final counts matching actual persisted events.
Transient unavailable polls are28 and147, respectively; no event gap results
in these bounded runs. These are per-owner timelines, not a common OBS clock.
Plots decimate cursor display only; the sequence audit reads every event.

The final measured owner captures/writes2880000 frames with zero dropped,
silent, nonfinite, discontinuity, timestamp-error, underrun or reset counts.
Its final delivery has zero packet/gap bound violations, maximum gap11.1310ms;
the measured admission bound is537frames. This is one empirical configuration,
not a reduced default, zero-fault waveform proof or sustained qualification.
Native render-padding sample is unavailable and remains null.

Whole fixed4s pre-owner control has reference NCC1 and return RMS/peak0.
Only one of four preselected affine waveform windows passes unchanged gates;
the other three residual refusals remain visible. The accepted window's OBS
offset is72.0461ms, not physical latency or a whole-run percentile. Continuous
cursor history is now available, but is not yet joined to native PCM/OBS; no
claim that it explains those refusals or that the audio is damaged follows.

The recording `records\2026-11-02 05-19-50.mkv` is82.304s,73220606bytes and
stays remote. Analysis takes6.453s. The native and offline tasks both finish
Ready/result0; source, monitor and owned OBS exit0 without forced cleanup.
Final audit finds no owned process and173.58GiB free. User process start times
are unchanged. First-packet investigation remains deferred.

Verified compact artifacts (remote and host lengths/hashes match):

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `analysis-summary-0715.json` | 10800 | `2AADFBF3506272E83FC884096038AC79DCDC8753671A6D9620BBFD6EAB94708A` |
| `analysis-overview-0715.png` | 265209 | `A1B7102E7AE9F1F7CFD9EC276F2D633EB0B5F5F3ED0339B9325BE3AAAB372F95` |
| `cursor-independent-audit.json` | 242826 | `9012EBA72E2C89103ECC1AFA87D301755F58ECF6B3A5669762FFA35B079F8A6A` |

The host copies are prefixed `20261102-051848-`. The PNG has been visually
reviewed: full recording envelope, all four window refusals/verdicts, the single
accepted diagnostic offset and both owner-local cursor curves are visible.

## Remaining work

The fresh post-review native run is
`D:\MoirenTest\20261102-053049-s1-cursor-reviewed`, app SHA-256
`D53D9509757DC0DF79D353C8017ECA562937A0EFCACD0F90569DF615E8BA8AED`.
It includes completed-read/expected-owner checks, not the subsequent paired-delta
validator tightening. Its source manifest is `20261102-053049-source-manifest.json`.
Both expected owners are observed:14910events/190chunks and89932events/1142chunks,
with zero sequence gaps, unretained frames or IO failures. Both independent
chunk audits agree with final read/frontier counts; transient unavailable polls
are31/141. The normal60.001s run, cleanup and both tasks finish successfully;
the owned source/monitor/OBS exit0 and all user process start times are unchanged.
The46s source completes2208000frames without source underflow. Delivery retains
three gap-bound violations (maximum11.3135ms against535frames), despite zero
drop/silent/nonfinite/discontinuity/timestamp-error/underrun/reset counters.
All four unchanged affine waveform windows are refused. No diagnostic offset,
physical latency, zero-quality-fault PASS or fitted explanation is claimed.
The control still has full reference NCC1 and return RMS/peak0.

Its `records\2026-11-02 05-31-53.mkv` is82.112s/73047370bytes and stays remote.
Offline analysis takes6.344s. Verified host JSON/PNG copies have prefix
`20261102-053049-`, and the second PNG is visually reviewed; its empty offset
panel is expected because no window passes. SHA-256/bytes:

- Summary10624: `F23DB3954337E05FBE894BCED76A30B5439E6FB3B93DC90D12028D1809F66F50`.
- PNG266324: `ABA1A6E99A69FE6927A7FBB0BBD7F56B01DA4F7EC4A051CD0CDAF3CD761F99A8`.
- Cursor audit242827: `F530130BE50EE3CD59B63B570A435DB61C00E4C6783B2BE884EEE641D18D6145`.

Do not replace these later refusals with the preceding run's more favorable
native counters or its single accepted window. The recording is a feature
smoke, not a stability promotion. Final compact audit finds no owned process.

Implement opt-in bounded native PCM retention and accepted/offered-frontier
mapping before associating these actual interpolation events with render/OBS
samples. Preserve loss/epoch/startup refusal controls and all current gates.
Do not substitute repeated stability scans or smaller-buffer experiments for
that software work. Physical A/B qualification is still unavailable and is not
a blocker for independent development.

The subsequent [native PCM increment](2026-11-02-process-native-pcm.md) now
implements that bounded offered/accepted correspondence and independent actual
SRC replay. Its evidence remains separate from this earlier cursor-only run;
render/OBS association and physical qualification are still unproven.
