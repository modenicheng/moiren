# Actual-boundary SRC callback checkpoints

## Problem and implemented surface

The [known-clock control](2026-11-01-src-clock-trace.md) validates one changing
synthetic servo/FIR history, not the historical acquisition's actual history.
The old BoundaryProbe publishes latest relaxed occupancy and rounded ppm
independently. Those values cannot say which Engine call applied which exact
SRC step, how much FIR input/output moved, or whether the observed call crossed
a reset. Sparse control polls cannot manufacture the missing facts.

This slice adds those facts at the actual VirtualPlaybackSource and
VirtualMicrophoneSink calls. It retains **one coherent latest callback**, not
a complete history or a reconstructed source-to-recorder mapping. The
additive `NativeBoundaryProbe.src_checkpoint` field exposes it through the
existing native observation/report path, including retained owner snapshots.
The legacy water-level fields and their meaning are unchanged and are not
atomically associated with this separately coherent checkpoint.

## Coordinates and report contract

`StereoResampler::cursor()` returns scalar `SrcCursor` facts:

- `reset_generation` advances on every actual FIR reset. This is not a native
  device epoch or necessarily one-to-one with the boundary reset fence.
- Successful FIR `input_frames` and `output_frames` since reset. Playback input
  includes starvation silence-fill; output includes frames later muted or
  dropped. These are not lossless source/driver frontiers.
- The next output's input center split into `next_input_frame` and fractional
  phase, plus the exact current input-per-output `step`. The integer center
  uses checked integer discarded-history arithmetic, never a large f64 origin.
- Sticky saturation, preserved across resets. Counters cannot wrap into a new
  clean timeline. Split-coordinate overflow returns a saturated unknown center,
  not zero or a usable mapping.

The DSP output, kernels, servo arithmetic, steps and ring-discard logic are
unchanged. Diagnostic counters use fixed checked scalar work on successful
push/output; no per-sample clock reads, logging, locks or allocation are added.

`BoundarySrcCheckpoint` schema1 brackets each nonempty actual boundary call:

- Callback sequence, actual Engine frame count/rate, and the supplied
  `ProcessContext` timeline epoch/start. Those Engine labels are not wall-clock
  or source-application timestamps.
- Observed boundary reset fence before/after, and requested fence sampled
  after the call. Another reset may occur after that sample; it is not a claim
  that the transport stayed current indefinitely.
- Optional FIR cursors before/after, and the exact `processing_step` captured
  after that call's servo update and before its PCM processing. Before/after
  `step` alone is insufficient when that same call updates correction or resets.
- Successful native-rate carrier transfers, the explicit startup/recovery edge
  mute in Engine frames, and the exact returned Engine-rate transfer/xrun/
  discontinuity report. The mute count does not cover every kind of silence;
  starvation, prime/fence waits, invalid layout and epoch races still require
  the report/frontiers. Nothing here excuses a nonzero xrun or dropped content.

Wrapping the existing processing body keeps every return observable: prime
wait, epoch fence wait, starvation, drop-new overflow and an epoch changing
after PCM processing. Zero-frame calls are omitted. A call that does not
process SRC has `processing_step=null`; an unadapted equal-rate boundary has
both cursors null. A reset-generation change is a barrier, not a continuous
coordinate span. The cursor before a call can belong to the old fence while
the after cursor belongs to the reset state; do not subtract across it.

## Publication and boundedness

The probe stores31 atomic u64 payload slots, a version and two attachment
flags. A unique boundary writer publishes once per nonempty callback with
fixed scalar work and no retries. The reader makes one attempt; unpublished,
in-progress, exhausted version or a reused probe returns `None`, serialized as
explicit null. Atomic payloads avoid a seqlock data race. Reusing a probe for
a second attachment permanently invalidates its SRC provenance; it does not
claim two callbacks are one stream.

The slots are inline in the already off-RT allocated probe, not another queue,
callback allocation or unbounded history. Each resampler adds four u64
counters and a sticky bool. The existing audio byte budget continues to cover
audio carrier/staging/FIR heap storage; this change adds fixed diagnostic
object storage, not larger audio inventory or a new target/capacity policy.
No lower latency or native CPU-cost qualification follows from boundedness.

Callback sequence gaps expose missed polls but do not recover their steps.
Owner generation and node/kind remain the enclosing provenance; replacing an
owner must not join its coordinates with an old owner. Retained handles can
read final published facts but do not prove native cleanup or a joined worker.
Independent boundary checkpoints are not an atomic transaction across the
graph. This surface intentionally does not claim that a slow collector now
has complete per-callback mapping evidence.

## Verification

Evidence root `D:\coding\moiren-evidence\20261101-src-checkpoints` freezes the
hypothesis, base `05d304f`, intended scopes and input hashes before edits.
The cursor and actual-boundary tests first fail for the missing methods.
The serialization regression separately fails because the native report does
not contain `src_checkpoint`, then passes with the real adapter wired.
All command failures and their logs remain archived.

Focused coverage includes:

- Positive/negative correction, exact pushes/output counts, fractional center,
  reset generations, integer origins above2^53, counter/coordinate saturation.
- Actual playback and microphone calls, initial prime mute/wait, FIR
  frontiers, exact applied step, returned report and consecutive callbacks.
- Starvation, full-carrier drop-new/reset, recovery, pending epoch fences,
  and a deterministic in-flight epoch change using the existing test hook.
- One-attempt contention/exhaustion/reused-probe refusal and10,000 concurrent
  publications with every returned sample internally coherent.
- Real graph RT allocation/deallocation guards with both probes attached,
  both precisions, mono/stereo, four rates, drift enabled/disabled, starvation,
  backpressure and fenced resets. Snapshot reads inside the guard also allocate
  nothing. No callback clock/log/heap operation is introduced.

One expanded test initially requests a zero-frame BufferArena view; that
arena rejects it with InvalidFrames before the boundary call. The invalid
test-only invocation is removed, not reinterpreted as a production defect or
a tested zero-frame path. Production zero-frame omission is explicit in the
wrapper but that unreachable-through-this-arena branch has no direct test.

An additional exact-limit regression catches an overly conservative intermediate
overflow in the new cursor: adding the retained16-frame FIR prefix to a large
discard origin before subtracting it rejects an otherwise representable final
center. The test fails, then passes after subtracting that local prefix before
the checked origin addition. A genuine next-center overflow remains saturated.
Only the new diagnostic arithmetic changes; no sample calculation changes.

The first full app run retains a graph meter animation-test failure at
`simulate_next_frame > 0`. The initial isolated command uses an incomplete
exact test name and executes zero tests; that is not counted as a pass. The
fully qualified one-test rerun passes, followed by a passing complete app
rerun, without UI source changes or timeout/threshold adjustment. The initial
failure is preserved rather than attributed to an unproven environmental
cause. The queued app probe example name is corrected to Cargo's actual
`vad_integration` before execution; no invalid-target run is claimed.

The known two-stage source fixture now checks every emitted production cursor
against its independent integrated center, output count and exact applied
step. Its eight known-center/model/oracle log lines are byte-identical to the
previous pre-instrumentation receipt. Model identities, scores, intervals and
PCM oracle errors are unchanged; the model implementation and servo source
are also byte-identical. This is finite evidence of unchanged sample math,
not a native performance or universal waveform-quality proof.

The final package and example receipts contain these exact totals (focused
reruns are not added again):

| Receipt | Command | Passed | Ignored |
| --- | --- | ---: | ---: |
| `audio-package-final` | `cargo test --locked -p moiren-windows-audio --all-features` | 982 | 3 |
| `examples-final` | `cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --example shared_render_preflight --example vad_lifecycle --example vad_native --example vad_active_lifecycle` | 749 | 0 |
| `app-tests-final` | `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe` | 672 | 4 |
| `app-vad-example-final` | `cargo test --locked -p moiren-app --features vad-integration-probe --example vad_integration` | 116 | 0 |
| `shadow-rt-final` | `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 220 | 0 |
| Total | Distinct package/example test targets above | 2,739 | 7 |

Strict Clippy passes for audio (all targets/features) and app (all targets,
`gpui-test,vad-integration-probe`). Audio and the shadow RT harness also pass
their all-target Linux cross-compilation checks. The strengthened atomic
publisher test is rerun separately after the package suites; it compares both
full cursors, including integer centers above2^53 and distinct fractions.
Fresh formatting, whitespace, source-hash, DSP-body and evidence-integrity
checks are retained in the final precommit audit. Linux checks are compilation
only. Tests do not exercise real native audio, a remote target, Miri, CI or
calibrated physical equipment.

## Next required evidence

This implements actual-boundary phase/consumption provenance that was missing
from the previous latest-ppm sample. Reconstructing an actual time-varying map
still requires complete retained ordered checkpoints for the measured spans,
owner/reset/fault barriers, and an independently established source/recorder
reference. Latest snapshots and nominal timestamps alone remain insufficient.
No old artifact gains new data retroactively; historical14/29 nominal
identity refusal,9686 packet-policy violations,14 unknown boundary samples
and incomplete runtime quality remain unchanged.

Native verification of final worker faults, continuous acquired waveform,
matched VAD/Process conditions, sustained lower-budget delivery and calibrated
physical acceptance remain open. S1-S3 and the overarching goal remain active.
No native acquisition/replay, remote/audio process, driver/control write,
speaker rendering, push, subagent or memory update occurs. `gate.log` remains
untouched.

The subsequent [ordered retention slice](2026-11-01-src-history.md) replaces
the inline latest payload with an off-RT prepared 256-slot history sharing
the same latest publication. It adds bounded pinned collection, actual writer
retirement and create-new probe export. The latest-only limits above describe
this receipt's revision; finite ordered coverage is now separately verifiable,
while source/recorder reference, waveform and physical gates remain open.
