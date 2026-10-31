# Independent SRC scalar archive readback

## Scope and frozen hypothesis

The [streaming retention slice](2026-11-01-src-stream.md), base `01ca7e2`,
distinguishes collector coverage from writer completion. Its completion flags
and flush receipt are not independent persisted-byte readback. Before using
saved coordinates, a separate reader must check what was actually serialized.
This is a prerequisite for future time-varying mapping work, not that mapping.

The pre-edit hypothesis, alternatives, input hashes and limits are frozen in
`D:\coding\moiren-evidence\20261101-src-archive-verifier\hypothesis-freeze.json`.
Trusting writer flags alone and loading the whole up-to-32-GiB stream are
rejected. This slice adds a host-only Python standard-library reader and an
opt-in real-owner test fixture. No production audio, RT instrumentation,
collector, writer, servo, resampler, operating target, capacity, default or
quality gate changes.

## Reader contract

Run from the repository root, with explicit paths:

```text
python scripts/vad-src-archive.py --summary RUN/src-trace.json --chunks RUN/src-trace.chunks.jsonl --output RUN/src-readback.json
python scripts/tests/vad-src-archive.tests.py
```

The imported `chunks_path` is a label only; it never opens or redirects an
input. Supported input is the current summary/trace schema2 and chunk/
checkpoint schema1 with the existing writer limits. Other versions or limits
refuse instead of guessing compatibility. Summary reads are capped at192 MiB,
records at8 MiB including newline, the stream at32 GiB, and boundary metadata
at256 distinct nodes. Streaming retains the bounded summary, capped current
record and per-boundary counters rather than lifetime callback arrays. These
are storage/work bounds, not a filesystem completion-time guarantee.

Checks include duplicate JSON keys, nonfinite numbers, strict field/type/range
schemas, owner and chunk sequence, registered node/kind identity, strictly
increasing callback sequence per boundary, chunk/pending limits, exact final
trace agreement, a final newline and no data after the terminal record.
Readback independently reconciles per-node counts, missing/unretained counts,
last sequences, writer retirement, drained totals, writer byte/count receipts
and both original history flags. Separate nodes need not appear in the same
chunk; their observations are not an atomic cross-boundary transaction.

An otherwise valid stream with an original scalar refusal remains incomplete
and emits a create-new diagnostic with exit1. Failed writer outcomes,
malformed/truncated streams and contradictory receipts refuse with exit1 and
no success report. Existing output is never replaced. Inputs and partial
archives are never changed. Output SHA-256 covers bytes consumed by this read;
it is neither producer authentication, post-read immutability nor power-loss
durability evidence. Imported JSON cannot construct native provenance.

`scalar_readback_complete` means serialized sequence-one through final scalar
coverage only. The report preserves unavailable polls, collection loss counters
and counts of callbacks with XRUNs, discontinuity, explicit mute or unknown/
saturated cursors. It does not validate cursor arithmetic or interpolate
phase, join source/driver/recorder epochs, infer a source origin, re-evaluate
waveform quality or calculate physical delay. Large split integer centers
remain integers; they are never combined into a large imprecise float.
`qualified=false`, `native_execution=false` and the explicit not-evaluated
fields remain unconditional, including a structurally complete archive.

## Verification

Test-first receipts retain the missing-reader RED, passing first implementation
and a later RED for omitted original collection counters. An expanded test
initially overwrites its own malformed writer-byte input while encoding the
fixture; the test is corrected, not production validation weakened. The first
rustfmt check catches one test-fixture line wrap, which is corrected. Every
nonzero receipt remains in the evidence directory.

The18 reader tests cover complete and gapped histories, original refusals,
zero callbacks, unknown polls, faults without quality promotion, changed owner,
reordered/duplicate callbacks and chunks, unknown/duplicate boundaries,
schema/type ambiguity, terminal bool/integer/float differences, truncated and
trailing records, byte caps including newline, exact stream-cap equality,
bounded read calls, split centers beyond2^53, create-new CLI output, explicit
input hashes and incomplete diagnostics. The CI fmt job runs this portable
suite beside the existing spacing/QPC/rebase tools; no remote CI is claimed.

The ignored `src_stream_independent_readback_fixture` is explicitly invoked
with `MOIREN_SRC_READBACK_FIXTURE` set to a new evidence child directory. It
uses the real public backend, EngineHost and playback -> Bus -> microphone
bindings, but a test-only provider and finite synthetic command clock; it
does not open native audio devices. An origin sidecar states that distinction.
Directory creation refuses reuse, preserving even failed fixtures.

The fixture emits132,000 checkpoints,66,000 per node, across60 chunks and
77,165,768 JSONL bytes. The independent CLI checks every callback and terminal
metadata and reproduces that stream hash:
`51f06edf8048e9f281de873cb4939a9f6bf821180ed22743700cf51f46e1a863`.
This deliberately unfed/undrained fixture retains66,000 playback and65,488
microphone callbacks with XRUN/discontinuity. Readback is scalar-complete,
but this is explicitly not a clean-audio fixture or an acquired clock history.

Fresh checks pass:658 probe tests with1 opt-in fixture ignored, plus the
separately invoked fixture;18 reader,16 spacing,17 capture-QPC and15 rebase
tests; strict probe Clippy, Ruff, rustfmt and whitespace checks. Focused
known-clock controls preserve the preceding eight oracle/model output lines.
The production example also passes Linux cross-compilation, compile-only;
this does not execute the ignored fixture or portable reader on Linux.
All four code files match their pre-suite source pin; the workflow receives
its own final pin. Exact-path staged/committed audits verify unchanged
production sources, preserved `gate.log`, logs and evidence hashes. An
independently rechecked manifest covers the evidence. This slice does not
rerun the whole workspace, native tests, Miri or remote CI.

## Still Open

Actual reset/fault-aware time-varying SRC mapping and independent known-clock
validation remain next work. This reader does not prove coordinate arithmetic,
native long-duration retention, source/driver/recorder correspondence, a
common reference, continuous waveform quality, native instrumentation cost,
matched VAD/Process measurements, sustained smaller budgets or calibrated
physical acceptance. Original nominal14/29 refusal,9686 packet-policy
violations,14 unknown observations and incomplete runtime quality remain
unchanged. No sub-10-ms result, impossibility proof, theoretical minimum or
default promotion follows. S1-S3 and the overarching goal remain active.

No native acquisition/replay, remote audio, driver/system-control writes,
speaker rendering, reboot, push, subagent or memory update occurs. Only the
focused implementation paths are committed locally; unrelated `gate.log`
is preserved.

The subsequent [callback-local phase interval slice](2026-11-01-src-phase.md)
uses an optional checkpoint observer behind full archive validation. It adds
reset/fault-aware scalar arithmetic/support queries, while this reader's
default scalar-only report and all native/quality limitations remain unchanged.
