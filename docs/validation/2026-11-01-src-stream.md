# Streaming actual-boundary SRC evidence

## Purpose and frozen scope

The [ordered history slice](2026-11-01-src-history.md), source `54e5f3b`,
retains at most 131,072 callbacks for a whole run. Two 32-frame boundaries at
48 kHz can exhaust that in about 43.7 seconds. Increasing a lifetime RAM cap
does not solve the requested longer acquisition. This slice drains bounded
control chunks to a separate writer without changing the RT publisher,
carrier capacity, operating targets, DSP, servo, defaults or quality gates.

The pre-edit hypothesis, alternatives and hashes are frozen at
`D:\coding\moiren-evidence\20261101-src-stream\hypothesis-freeze.json`.
Unbounded channels, RT file IO and synchronous disk writes in the collection
poll are rejected. Existing 256-callback histories still have finite polling
requirements; no writer can recover a callback that was overwritten before
the control collector observed it.

## Collector contract

`NativeSrcTraceCollector::drain_chunk()` moves pending scalar checkpoints to
an owned chunk, with pinned owner generation, monotonic chunk sequence and
separate node/kind boundaries. The caller owns persistence: a drained chunk
is not an IO acknowledgment. Trace schema2 adds drained chunk/checkpoint
counts; checkpoint and chunk schema1 remain explicit. Imported JSON still
has no constructor for native provenance.

The configured checkpoint limit now bounds pending checkpoints. Callers that
never drain retain the preceding lifetime-limit behavior. Missing callbacks,
invalid suffixes, reused probes and pending-cap loss remain sticky across
chunk boundaries. At most 256 drained checkpoints per boundary are retained
separately to validate overlap after a drain. Consecutive small drains retain
the necessary prior overlap instead of forgetting earlier facts. This is
bounded control memory, not extra RT publication or audio inventory.

Final scalar coverage requires independently retired writers and the
sequence-one through final callback count across drained and pending facts.
It can be complete while retained callbacks contain starvation, explicit
mute, discontinuities, resets or xruns. `qualified` remains false.

## Probe stream and failure handling

The actual VAD path replaces its lifetime archive with `NativeSrcStream`,
using a 4,096-checkpoint pending cap and a 2,048-checkpoint drain threshold.
A control-only synchronous channel holds at most four owned chunks. Polling
only collects and tries to submit; it performs no file IO, serialization,
thread join or channel-capacity wait. A full queue returns the same owned
chunk to one retained pending slot. If slow IO later exhausts the collector
or overwrites boundary history, the original gap/cap refusal remains.

A separate named worker serializes create-new `src-trace.chunks.jsonl`.
Each record is a tagged chunk or terminal summary. Chunk sequence and owner
must match; JSON escaping and the newline count toward an 8 MiB record cap
before any record bytes are written. A separate 32 GiB total stream cap is
not an audio-buffer change and is not a promise to cover every two-hour
configuration. Callback segmentation, boundary count and IO rate matter.
The number of checkpoints retained in RAM no longer grows with run duration.

After actual EngineHost retirement, teardown submits the retained Full chunk
before the final tail, closes the channel and joins the writer. This is the
only path allowed to wait for disk IO; a stalled filesystem can still delay
teardown. Drop also joins instead of leaving a detached writer. This slice
does not claim a bounded filesystem completion time or power-loss durability.

An IO error, partial write, zero write, serialization/byte-cap refusal, owner
or sequence mismatch, missing terminal record, flush failure or caught sink
panic prevents stream completion. Already written bytes are preserved.
Failed/queued chunks and an unsent tail remain in the bounded final summary
where possible; allocation abort or an unexpected worker failure cannot
manufacture their provenance. No successful prefix replaces failed evidence.

Create-new `src-trace.json` now contains schema2 trace metadata, writer counts,
limits, failed chunks and the stream path. Its top-level `history_complete`
requires both scalar coverage and matching successfully written chunk and
checkpoint counts, including terminal flush. A successful summary export can
therefore contain `stream.complete=false` and `history_complete=false`.
Summary-write failures are reported separately; repeated finish calls retain
their original error rather than turning the retry into success. Flush means
the OS writer accepted bytes, not independent readback or fsync durability.

Both paths are reserved before native/file setup, including normalized
report/manifest/input aliases, and are copied into artifact/manifest paths.
Existing startup/collection/soak/precleanup polls and post-host cleanup now
operate on the stream. Existing runtime quality, collection and cleanup
verdicts are not replaced. No disk failure authorizes a waveform-quality pass.

## Verification

Test-first commands fail for the missing drain/chunk/writer APIs. The initial
example fixture also tries the library-private publication helper; this is
corrected to use the real public backend/owner/EngineHost with a test-only
provider and finite command-driven clock, rather than exposing a fake native
provenance constructor. The corrected RED then fails only for the absent
stream type. A strict lint catches a test-only finish helper in a non-test
build; it is scoped to tests, without suppressing the lint. All failed logs
and receipts are retained.

Focused checks cover:

- 132,000 actual sink callbacks drained through 660 chunks with a 256 pending
  cap; sequences, timeline positions, owner identity and final closure agree.
- Missing callback gaps and final faults survive drains; changed overlap
  after consecutive small drains cannot replace prior evidence.
- The real playback -> Bus -> microphone owner emits 132,000 combined
  callbacks into create-new chunks, beyond the former lifetime cap. A
  line-by-line readback checks each node's sequence through 66,000, terminal
  record, matching counts and scalar completeness after actual host drop.
- Deterministically blocked IO fills exactly four queued chunks and returns
  the next unchanged chunk without waiting. A released writer completes the
  retained tail in order.
- Stream/record caps, partial IO, terminal flush, owner/sequence change,
  missing terminal and sink panic refuse honestly and retain failed chunks.
- An actual stream failure persists failed scalar chunks in a summary while
  distinguishing complete collection from incomplete export. An existing
  summary is preserved; repeated finish remains an error.

Final package/example receipts contain these exact totals; focused reruns
are not added again. These count distinct test targets, not unique behaviors.

| Receipt | Command | Passed | Ignored |
| --- | --- | ---: | ---: |
| `audio-final` | `cargo test --locked -p moiren-windows-audio --all-features` | 996 | 3 |
| `examples-final` | `cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --example shared_render_preflight --example vad_lifecycle --example vad_native --example vad_active_lifecycle` | 760 | 0 |
| `app-final` | `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe` | 672 | 4 |
| `app-vad-final` | `cargo test --locked -p moiren-app --features vad-integration-probe --example vad_integration` | 116 | 0 |
| `shadow-rt-final` | `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 229 | 0 |
| Total | Distinct package/example test targets above | 2,773 | 7 |

Strict Clippy passes for audio (all targets/features) and app (all targets,
`gpui-test,vad-integration-probe`). Audio and the shadow RT harness also pass
all-target Linux cross-compilation; these are compilation only. The eight
known-center/model/oracle output lines match the preceding receipt exactly.
All nine changed code files match pre-suite source pins. The entire RT
boundary/publisher files, resample/servo and model sources remain unchanged
from the base revision; callback cost from the preceding instrumentation is
not newly qualified by this control-only change.

Formatting/whitespace, exact staged/committed paths, retained failures, source
hashes and postcommit status are checked in the final audits. An independently
rechecked SHA-256 manifest covers the archived evidence. No test substitutes
for native acquisition, remote target, Miri, CI or calibrated physical equipment.

## Remaining S1-S3 requirements

This removes the lifetime-RAM cap on ordered retention within the declared
finite stream and polling limits, not the need for acquired evidence. Actual
source/driver/recorder epochs and packet correspondence, an independent common
reference, continuous waveform quality, native diagnostic CPU/cache/service
cost, matched VAD/Process conditions, sustained lower-budget delivery and
calibrated physical acceptance remain open. Historical nominal14/29 refusal,
9686 packet-policy violations, 14 unknown observations and incomplete runtime
quality remain unchanged. There is no new source-origin fit, physical delay,
sub-10-ms pass, impossibility proof, theoretical minimum or promoted default.

S1-S3 and the overarching goal remain active. This slice performs no native
audio acquisition/replay, remote audio, driver/system-control writes, speaker
rendering, reboot, push, subagent or memory update. Unrelated `gate.log` is
preserved.
