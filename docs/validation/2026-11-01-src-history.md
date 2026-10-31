# Ordered actual-boundary SRC retention

## Scope and design

The [preceding checkpoint slice](2026-11-01-src-checkpoints.md), source
`194ffd0`, publishes one coherent callback. Slow control polls can expose a
gap but cannot recover its applied steps. This slice adds a fixed callback
history and a pinned, bounded control collector to the real VAD probe route.
It does not fit a source origin, reconstruct old acquisitions, or claim a
clean waveform or physical delay.

The pre-edit hypothesis, alternatives, scope and source hashes are frozen at
`D:\coding\moiren-evidence\20261101-src-history\hypothesis-freeze.json`.
Polling the latest value more frequently cannot prove an unobserved callback;
unbounded storage, backpressure and RT file writes are rejected. Instead the
existing unique-writer atomic payload publication becomes a 256-slot suffix.
Latest reads and history reads share the same payload and version, without a
second per-callback publication. Each callback still stores exactly 31 u64
payload words plus its version and fixed flags; modulo slot selection and a
finished-flag read are additional diagnostic work.

## History and ownership

`BoundaryProbe::src_history()` is control-only and returns one bounded owned
copy, or unknown without retry on contention, version exhaustion or reused
probe. A newly attached, unpublished writer has a known empty open history;
the latest-checkpoint API still returns unknown before its first callback.
The history reports total callbacks, overwritten callbacks and actual writer
closure. It preserves the original checkpoint schema, exact processing step,
reset barriers, explicit mute and returned faults.

The 256 x 31 atomic words occupy 63,488 payload bytes per probe, allocated
off RT directly into boxed storage. Probe objects remain small; there is no
large owner stack array, extra audio inventory, queue resizing, callback clock,
logging, locks, retry or callback heap work. This is additional fixed
diagnostic storage, not covered by the audio carrier/FIR byte accounting.
Preparation can still fail at the allocator like existing diagnostic objects;
no native CPU/cache/service-cost qualification follows from boundedness.

The attachment closes on boundary drop or off-RT probe replacement. A second
attachment permanently invalidates that probe's SRC provenance. Native
Running=false, registry clear, worker join and Engine binding retirement are
different facts: joining an owner returns its Engine to control, so the
boundary can remain open until that Engine actually retires. A real
EngineHost drop test confirms the host's cleanup path retires its bindings
before collector finalization. Closure never proves successful driver/native
cleanup.

## Pinned control archive

`NativeObservationReadHandle::src_trace_collector(limit)` pins the exact owner
session and separate node/kind probes. Registry replacement cannot switch its
generation. The caller chooses 1..131,072 total checkpoints shared across all
boundaries, not that many per node. Each poll validates an ordered suffix,
compares retained overlap, appends unseen callbacks, and retains reset/fault
events unchanged. It does not resample, interpolate or insert fake callbacks.

An unavailable poll is counted; it does not itself prove missing data when a
later history can recover the complete interval. A missing first/interior/tail
sequence is counted permanently. Archive exhaustion preserves the first
retained checkpoints, counts subsequent unretained callbacks, and refuses a
complete-history verdict instead of restarting an empty candidate. The final
poll after independent teardown must observe closed writers and nonempty,
sequence-one prefixes through every final callback. Repeated finish/poll calls
do not rewrite a finalized artifact. All copies, allocations and JSON work
are control-side; a history read allocates at most 256 checkpoints per probe.

`history_complete` means scalar callback coverage only. It can be true while
the retained checkpoints report xruns, startup silence, resets or dropped
content. Those faults are not excused; `qualified` remains false. An open,
unavailable, malformed, truncated or missing prefix is explicitly refused.
Imported JSON has no constructor for pinned native provenance.

## Actual probe integration

The VAD E2E probe pins the owner after Engine activation, before gate
admission; its existing startup/admission/collection polls drain actual
histories, including runs without a diagnostic journal. Ordinary VAD soak
polls do the same. It takes another pre-cleanup sample and finalizes after
EngineHost drop, then writes create-new `src-trace.json`. The path is reserved
before native/file setup, exposed in artifact paths, and summarized in the
native and collection report even when scalar coverage is refused. Existing
collection, runtime quality and cleanup verdicts are not relaxed or replaced.

Serialization has a separate 192 MiB cap including JSON escaping/newline,
checked before opening the artifact. IO failures retain partial files and an
explicit export failure; an existing artifact is never overwritten. The
131,072 checkpoint limit is finite: e.g. two 32-frame boundaries at 48 kHz
produce about 3,000 callbacks per second before segmentation, so this cap is
not a promise to cover 600 seconds or a multi-hour soak. Callback segmentation,
control stalls, setup/cleanup and writer count also matter. A 256-slot history
does not guarantee a slow provider QUERY will finish before overwrite. Gaps
or limit refusal must remain visible; no shorter run is relabelled as the
requested full duration. Longer streaming/chunked retention remains open.

## Verification and preserved failures

History, collector and exporter tests first fail for their missing APIs.
An implementation build then catches two existing tests moving drift state
out of newly Drop-bearing boundary types; they now borrow the same state,
without changing assertions. Another build rejects unused fixture fields;
the tests now actually assert node provenance and closure. A later exact-Bus
test initially uses the wrong graph disconnect signature and is corrected to
the existing EdgeId API. All failed commands/logs remain archived; none is
claimed as a passing behavioral regression or a native failure.

Focused tests verify:

- 300 callbacks yield exactly ordered sequences 45..300 and overwrite44;
  latest and history agree. Concurrent histories are coherent or unknown.
- 600 actual sink callbacks across three 200-callback polls remain complete
  through registry clear and final faults. Repeated polls never duplicate.
- Missing prefixes, interior/tail gaps, shared caps, open writers, second
  attachment, changed overlap and malformed suffixes refuse honestly.
- The exact playback -> Bus -> microphone production owner returns its Engine
  with open bindings; Engine retirement closes them. Real EngineHost drop
  then finalizes complete callback histories without opening native devices.
- Existing actual graph allocation/deallocation guards run with both probes
  attached across precisions/rates/channels, starvation, overflow and resets.
- Create-new export, byte caps, normalized path collisions and manifest paths
  preserve evidence before native setup. Linux checks are compilation only.

The final receipts contain these exact totals; focused reruns are not counted
again. These are distinct test targets, not a count of unique behaviors.

| Receipt | Command | Passed | Ignored |
| --- | --- | ---: | ---: |
| `audio-final` | `cargo test --locked -p moiren-windows-audio --all-features` | 993 | 3 |
| `examples-final` | `cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --example shared_render_preflight --example vad_lifecycle --example vad_native --example vad_active_lifecycle` | 751 | 0 |
| `app-final` | `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe` | 672 | 4 |
| `app-vad-final` | `cargo test --locked -p moiren-app --features vad-integration-probe --example vad_integration` | 116 | 0 |
| `shadow-rt-final` | `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 226 | 0 |
| Total | Distinct package/example test targets above | 2,758 | 7 |

Strict Clippy passes for audio (all targets/features) and app (all targets,
`gpui-test,vad-integration-probe`). Audio and the shadow RT harness pass their
all-target Linux cross-compilation checks; these are compilation only. The
eight known-center/model/oracle lines are byte-identical to the preceding
known-clock receipt. All 14 changed code files match the source hashes pinned
before the final suites. Production read/write DSP bodies, resample/servo and
the model implementation remain byte-identical to the base revision.

Fresh formatting/whitespace checks, source hashes, reviewed scopes, retained
failure receipts and postcommit status are recorded in the evidence audits
and verified SHA-256 manifest. No test result substitutes for native
acquisition, Miri, CI, independent timing or calibrated physical equipment.

## Remaining objective

This closes finite ordered scalar retention in the actual probe path when
its declared caps and polling permit complete coverage. No historical PCM
gains new checkpoints. Historical nominal14/29 refusal, 9686 packet-policy
violations, 14 unknown boundary observations and incomplete quality remain.

Actual source/driver/recorder epoch and packet correspondence, an independent
reference, whole-waveform continuity, native diagnostic cost, matched VAD vs
Process routes, sustained lower-budget delivery and calibrated physical
acceptance remain open. No sub-10-ms pass, impossibility proof, theoretical
minimum or new default follows. S1-S3 and the overarching goal remain active.
This slice performs no native acquisition/replay, remote audio, driver/system
control write, speaker render, push, subagent or memory update. Unrelated
`gate.log` is preserved.

The subsequent [streaming retention slice](2026-11-01-src-stream.md) adds
draining and create-new ordered chunks through a bounded control-only writer.
The lifetime archive limit above describes this revision; pending/stream caps
and polling gaps still refuse explicitly, and scalar coverage remains separate
from persistence, waveform quality and physical qualification.
