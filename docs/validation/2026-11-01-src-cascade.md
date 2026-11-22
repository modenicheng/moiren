# Stamped execution and bounded two-FIR dependency envelopes

## Frozen scope

Base `aaf88ad` retains [initial prepared IO/Bus/IO linkage](2026-11-01-src-route.md),
but preparation alone cannot identify the route or parameters that execute a
callback. The pre-edit hypothesis, alternatives, source hashes and restrictions
are frozen in `D:\coding\moiren-evidence\20261101-src-cascade\hypothesis-freeze.json`.
This slice adds actual execution identity and an offline bounded dependency
composer, not a physical-delay estimator or S1-S3 completion claim.

No push, subagent, native acquisition/replay, remote audio, speaker rendering,
driver/system-control write, reboot or memory update occurs. FIR/servo kernels,
queue policies, operating budgets, capacities, defaults and quality gates stay
unchanged. Unrelated `gate.log` retains its original221 bytes and SHA-256.

## Actual execution identity

`ProcessContext.execution` is optional. `Engine::render` supplies the actual
plan revision plus the Engine-lifetime number of **applied** parameter events,
after events at the segment's first frame. Merely accepted, future, rejected,
reply-backpressured and budget-deferred requests do not increment it. Ramps
remain noninitial even after settling. Plan replacement does not reset the
counter, including when parameter state/ramp progress is inherited. Exhaustion
becomes sticky unknown, never a wrapped zero. Manual context callers use `None`.

`PrepareRequest.plan_revision` retains the candidate compilation identity,
distinct from graph intent. Prepared route schema2 records that revision;
checkpoint schema2 records the execution stamp; trace schema4 carries both.
Summary2, chunk1 and history1 envelopes remain unchanged. Independent readback
still accepts trace2/3 and checkpoint1 diagnostically, but the cascade mapper
never upgrades their missing execution/prepared-plan linkage into authority.
Imported JSON cannot authenticate native origin, even with consistent stamps.

The bounded checkpoint payload grows31 to34 atomic u64 fields:6144 additional
prepared bytes per256-slot probe history and three additional scalar stores
per published callback. No RT allocations, locks or retries are added. This
is instrumentation overhead, not a measured service-cost or latency result.

Engine regressions check actual event segmentation, plan switching, deferred
acknowledgements, stale requests, block-end scheduling and sticky exhaustion.
Heap accounting around applied/deferred renders records zero allocation,
reallocation and deallocation. Actual EngineHost controls change callback
generations while preparation remains uninvalidated; a same-owner graph edit
changes actual callback plan identity while preserving the initial witness.

## Three-pass dependency contract

Run with explicit local paths:

```text
python scripts/vad-src-cascade.py --summary RUN/src-trace.json --chunks RUN/src-trace.chunks.jsonl --queries RUN/queries.json --output RUN/cascade.json
python scripts/tests/vad-src-cascade.tests.py
```

Queries use schema1, owner generation and1..256 distinct microphone callback
`sequence`/`output_offset` pairs. The exact initial witness supplies source and
sink node identities. Each stage's rational phase arithmetic reuses the fixed
`2^-32` input-frame-per-output rounding allowance; integer origins never become
floats. A32-tap FIR needs `floor(center)-15 .. floor(center)+16`. Numerical
floor ambiguity can widen that to33 input frames, never an empirically fitted
tolerance. The mapper retains at most256*33 dependency requests per stage,
not lifetime coordinate/PCM arrays.

1. Read the complete archive and validate requested microphone local support,
   actual initial-plan identity and control generation0.
2. Read it again, mapping **every** microphone pushed-input support sample to
   the actual callback's Engine timeline and execution identity. Use successful
   pushed-input counts, not nominal rate subtraction or native packet clocks.
3. Read it again, locating each Engine frame in playback's actual output
   callback and propagating its complete upstream FIR support.

All three passes must finish strict terminal, byte/count, owner and complete
history checks and consume identical stream hashes before results are returned.
The input path label inside the archive is never followed. CLI output uses
create-new only; truncation or changed bytes between passes releases no results.
Faults, mute, short transfer, unknown cursor, gaps, reset/epoch boundaries,
saturation and tainted FIR history refuse affected dependencies. Unknown or
noninitial execution is sticky for that boundary's remaining owner lifetime;
later imported zero/reused-plan labels cannot restore initial authority.

Callback sequences are not paired across independently observed boundaries.
Different segmentation and all-microphone-before-all-playback chunk order are
tested. Dependencies must share a usable timeline and reset/segment domain;
duplicate, missing or conflicting coordinate coverage refuses. A later failed
or cancelled Update preparation does not erase earlier actual initial-plan
dependencies: the sticky preparation flag alone is not execution authority.

Successful output is a **dependency envelope in reset-scoped playback pushed
input**, plus individual Engine/playback dependencies. It is not a unique
`cascade_center`, transfer function, marker identity, native packet frontier,
source/driver/recorder clock, waveform verdict or physical latency. All results
keep `qualified=false`, `native_execution=false`, unknown source origin and
unevaluated waveform/physical delay. A synthetic control maps microphone taps
113..144 to Engine113..144 and playback support98..160; adding two center
coordinates would not establish this dependency certificate.

## Coupled changing-PCM controls

The portable test compiles exactly Source -> Bus -> Sink with default-unity
sends, binds actual `VirtualPlaybackSource`/`VirtualMicrophoneSink`, and renders
512 callbacks per boundary. It covers44100/48000/88200/96000 Hz, mono/stereo and
f32/f64:16 configurations. Deterministic changing carrier PCM, deliberately
changed source occupancy and actual applied corrections exercise both stages.
Every Bus input sample equals a separately fed playback FIR result after the
documented startup mute and processing-precision conversion. Every published
microphone f32 frame equals a separately fed microphone FIR result driven by
that **actual Bus PCM**, not an unrelated microphone signal. Final cursors also
match both controls. Collection uses test-only locks/Vecs, not RT production.

The controls are separate production FIR instances sharing exact applied ppm;
they are **not independent FIR-kernel implementations**. Python independently
checks rational intervals against their per-output split cursors. Optional
create-new export via `MOIREN_SRC_CASCADE_FIXTURE` retains16 complete scalar
archives,16384 checkpoints and144 first/middle/last callback-output queries.
All144 mapped envelopes contain the actual cursor support. These fixtures are
explicit test-only archive assembly from a compiled graph, not native collector
or acquired-waveform provenance. Whole-run PCM is checked in Rust, not stored
or replayed as a native recording.

The separate real-backend/EngineHost132000-checkpoint fixture now exports trace4.
After removing only the new execution/schema fields, all60 chunk records match
the older trace3 fixture structurally at every checkpoint. Trace2/3/4 readback
remains complete and the six historical local/fault query results are unchanged.
This provider remains synthetic and unfed; it is not a clean waveform result.

## Verification and retained failures

Fresh command receipts and original failure outputs live under the frozen root.
The engine suite passes352 tests; audio passes1005 with5 intentionally ignored;
probe examples pass762 with1 ignored; standalone RT shadow passes229. Python
archive/phase/cascade regressions pass25/16/16. Counts are test-target totals,
not unique allocation-only behaviors. Strict engine/audio all-target/all-feature
Clippy, Ruff, rustfmt and whitespace checks pass. App all-target/all-feature
check passes. Engine/audio and RT shadow Linux checks pass **compile-only**.
No full workspace/app test, Miri, remote CI, native run or physical acceptance
is claimed.

Retained nonzero receipts include missing execution/trace/composer/FIR-fixture
API REDs, the recovered-authority regression RED, initial rustfmt/Ruff findings,
an invalid same-revision swap in the exhaustion test, an unavailable block
reborrow API in the test wrapper, and a Windows-only metadata import in the
portable fixture. The latter is repaired with explicitly test-only serialized
route metadata, not by widening production cfgs. A direct-control test initially
targets an absent optional strip; inspection shows only configured strips are
valid host targets. It now uses the actual existing send-gain target. Failed
focused/full receipts remain, followed by passing focused/full reruns; no
production control semantics, timing thresholds or quality gates change.

## Remaining boundaries

Source/driver/recorder packet correspondence, native sample origins, independent
common-reference acquisition, long-duration native retention/service cost,
matched VAD/Process conditions and calibrated physical qualification remain
separate unresolved work. Historical nominal14/29 refusal,9686 packet-policy
violations,14 unknown observations and incomplete runtime quality remain intact.
There is no sub-10-ms pass, impossibility proof, justified theoretical minimum,
default promotion or source-path winner. S1-S3 and the overarching goal remain
incomplete.
