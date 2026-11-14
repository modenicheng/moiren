# VAD Playback decode-call envelope, 2026-10-31

## Scope and source identity

Author commit `ca26bacb792a86d4f8cbc10660a9d3d9d6b2fae0` is integrated as
`afaef5e51aa5fe0a23964899212d30c15184d4d8`. Root verifies all six changed files
byte-identical to the author source. The separate engine carrier-abandonment
fix `2d0a16c67fd154994ded0d73bc3a743a0f8d68a2` is excluded from this experiment.
Playback/virtual speaker remains Moiren Input/Source; MIC remains Output/Sink.

This additive diagnostic changes no READ count/order, decode return/error
policy, native admission, buffer/default, gap480/jitter96, recovery, cleanup or
quality rule. It adds two host QPC reads per accepted decode call and fixed
atomic updates; that overhead is not qualified as negligible or hard realtime.
No PCM copy, logging, allocation, wait or new native read is added by the hook.

## Ownership and measurement meaning

`vad/provider.rs` binds the first actual `PcmOwner` after successful owner
creation. The immutable identity retains its original five-field native stamp,
reset epoch, admission ticket/revision and native PCM shape. It is not relabeled
from a later QUERY or a later admission revision. The sole PCM worker mutates
the recorder in `vad/provider/telemetry/publication.rs`.

`vad/provider/pcm_worker.rs` brackets the existing accepted
`bridge.decode_packet(pcm)` call, not packet completion or an atomic carrier
publication. Stale-order placeholders that return `Ok(true)` without decoding
are excluded. Schema5 exposes the optional summary as
`VadPcmTelemetrySnapshot.playback_publication`:

- `successful_decode_calls` counts accepted calls returning `Ok(true)`.
  `decode_ok_frames` counts the frames represented by those calls. Neither
  proves that the consumer received those frames: the existing exact writer
  has an abandonment race, deliberately not repaired in this source slice.
- `max_call_interval_100ns` is the maximum eligible inter-call envelope,
  `ceil((current_after_qpc - previous_before_qpc) * 10000000 / frequency_hz)`.
  Subtraction uses raw ticks from the same host clock before u128 conversion.
  This is not driver completion cadence, source sample age, consumer delivery
  spacing or end-to-end latency.
- Full/false and decode-error calls retain their separate counters without
  erasing the previous successful-call frontier. Clock loss/regression or
  conversion overflow breaks comparability; saturation and version exhaustion
  remain explicit faults/unknowns rather than wrapping cleanly.
- Pre/post epoch and admission checks annotate validity without changing the
  decode/audio decision. A call followed by observed revocation retains its
  success count but is ambiguous, not freshly attributed delivery.

A reset observed by the worker invalidates the original segment. Any subsequent
owner bind, even with an identical native stamp, increments `owner_reopens`,
retains the original identity and leaves it invalidated. There is no automatic
rearm. Later `Ok(true)` calls still increase cumulative success/frame counts,
but increase `ambiguous_packets` rather than eligible intervals or their
first/last QPC/sequence frontier. Thus a retained small eligible maximum can
describe an early segment while the remainder of the run is unknown. It must
not be presented as a whole-window maximum. These rules explain the receipt
after a reset; they do not identify why that reset happened.

Normal worker exit and the actual outer panic-catching boundary invalidate
Playback telemetry. `WorkerPanicked` and original cleanup behavior remain
unchanged. Invalidation reflects worker observation, not an instantaneous
transaction with external revoke/reset or control-side QUERY.

## Coherence and permanent limits

Every recorder mutation uses a single-writer version and fixed scalar atomics.
A reader loads version, fields and version once, returning `None` on contention
or a version change; it does not spin. This coherent sub-summary is not an
atomic snapshot of the parent legacy counters, native QUERY, admission, stop
or join. Legacy serialized telemetry lacking the field deserializes it as
`None`, not synthetic valid zeros.

`carrier_publication_effect_known` and `complete_delivery_coverage` are always
`None`/JSON null. Neither a live snapshot nor drained worker storage proves
joined execution, complete prefix/suffix coverage, PCM continuity or delivery.
No gap reduction, physical latency claim, default promotion or current-device
sub-10-ms impossibility conclusion follows from this diagnostic.

## Software verification and retained failures

Author evidence is under
`D:\coding\moiren-evidence\20261031-vad-publication-telemetry`:

- `publication-pump-red.log` retains the genuine missing-summary RED. The
  original test/field wording predates the final decode-call naming.
- `publication-review-p2-red.log` retains22 passing and2 failing tests when only
  outer panic invalidation and the final reader-version check are removed.
  Both changes are restored before final GREEN verification.
- Final focused tests pass24. Both default and `--features vad-probe` runs pass
  audio672 with2 ignored and probe622. Strict Clippy passes in both Windows
  configurations; Linux-target check/Clippy, formatting and diff checks pass.
  These author Cargo runs use jobs2, `RUSTFLAGS=-Dwarnings` and the author's
  worktree-local target, confirmed by the handoff and retained log headers.
  This does not assert root used the same environment flags. Linux checks
  compile only: the Windows provider/pump tests are not claimed to execute on
  Linux.
- Regressions exercise actual FakeIo pumping, stale/full/error results,
  post-call revocation/reset, reader exit, zero allocations/deallocations,
  real writer contention, writer advancement between field reads and the final
  version check, clock failures, owner reopen, legacy parsing and panic exit.

`author-handoff-ca26bac.json` retains exact commands, source/log hashes and
limits. `independent-source-review-ca26bac.json` records22/22 review checks with
no blocking finding; these are inspected source/artifacts, not independent
test executions.

Root integration evidence is under
`D:\coding\moiren-evidence\20261031-vad-call-envelope-native`. The first
feature run passes audio672/2 ignored but fails two existing2-second handshake
tests: `collection_cleanup_unwind_broadcast_precedes_a_blocked_sink` for
Physical, and `microphone_retirement_broadcasts_before_blocked_host_join` for
`close_failure=2`. The original620-pass/2-fail probe output and panic backtraces
remain in `root-feature-test*`. Concurrent build activity reported by root is
context, not a proven cause. No code or timeout is changed: each isolated test
then passes1/1, and the full same-source feature rerun with
`RUST_TEST_THREADS=2` passes audio672/2 ignored and probe622. The default full
run also passes672/2 ignored and622; both strict Clippy configurations and fmt
pass. The frozen fixture/pin review separately records34/34 checks; root's
contract/runner/archive fixtures record21/27/40, not native acceptance.

## Fresh binary and terminal acquisition

Root builds the actual integrated release with
`cargo build --locked --offline -j 2 --release -p moiren-windows-audio --features vad-probe --example e2e_latency_probe`.
`build-completion-pin.json` records7238144 bytes and SHA256
`4282DD7F488666CCEAC4EE421F6C17A0BDD14331B64FC469F29115E02013E507`.
It is a fresh build, not a reused prior probe.

Root completes `dawn-144-e32-t881-1` with driver610 and the unchanged repeated-v1
source/DAWN144/Engine32/target881 configuration. Capacity4800, declared packet/
gap480, jitter96 and recovery cap4 remain unchanged. The request is305 seconds,
but the terminal probe exits1 and `observed_duration_seconds` is0.0. This is
not a305-second successful run:61 sampled observations extend from approximately
0 to300.3363808 seconds, and the final cleanup error is retained.

The terminal report's live provider snapshot retains101660 completed packets
and101660 successful decode calls/14639040 represented frames. Of these calls,
99772 are ambiguous. Only1887 eligible intervals remain, with first sequence1
and last eligible sequence1888. Their first-before/last-after QPC values are
1104642522472/1104699130803 at10000000Hz, an approximately5.6608-second prefix.
The eligible maximum36274 in100ns units is3.6274ms for that prefix only, not
the remaining live interval. `invalidated=true`, `owner_reopens=1` and
`segment_breaks=1`; effect and coverage remain null. The separate maximum
completion interval240141 is24.0141ms. It is neither physical latency nor a
complete carrier-publication/delivery bound, and it does not repair the missing
call-envelope suffix.

Live telemetry retains epoch reset1/recovery1; the boundary retains32 startup
prime muted frames and32 recovery-prime muted frames. QUERY retains
`dropped_packets=4`, while collected-packet telemetry retains
`driver_dropped_packets=0` and aggregate `dropped_packets=0`. These distinct
observations are preserved, not reconciled into zero loss. QUERY's gate epoch
changes from1 to2 while stream generation remains1. This is an observed gate
change, not evidence of a replacement stream or a root-cause attribution.
Post-cleanup telemetry has epoch resets2/recovery2, separately from the live
counts; that increment must not be silently assigned to the active interval.

Cleanup fails with `VAD PCM shutdown terminal failures` for slots0,1,2,3,
each retaining HRESULT `0x8007051A`. The engine's final summary is Dormant/Closed
with no active revision but a non-null cleanup error. The final backend
explicitly reports `VAD final host did not join cleanly`; Removed/Closed
provider state and owner disappearance are not a clean host-join verdict.
The cleanup provider preserves dropped4 and silence0, gate epoch7,
stream generation1, `transport_prepared=false` and `removing=true`.
No cancellation/revision-error cause or safe-to-ignore classification is
established by this receipt.

The source separately reports native Stop success and a joined worker with no
source failure. Its repeated-v1 schedule acknowledges14400000 source frames
and `schedule_submitted_complete=true`. Those are successful ReleaseBuffer
submissions, not proof of playback, recovered waveform or delivery. No PCM is
recorded in this diagnostic and no independent calibration is present.

The controller confirms the probe's exit1 with no timeout, forced termination,
remaining owned process or controller failure. Boot and driver hash remain
unchanged, the driver remains Running, and no default-role or render-volume
change is reported. The dynamic child is Removed; the root device remains
present. These cleanup/inventory facts do not override the retained host error.

Root's `dawn-144-e32-t881-1-root-verification.json` records archive/provenance
verification with17 payload members,75806 ZIP bytes and SHA256
`94B6F5B94CA7219DAEAEC0DF3EAB2DB65F7136DBC60C1EE006566B4A69BF41F3`.
The extracted originals are under `dawn-144-e32-t881-1-verified`; the run's
`capture/report.json`, `capture/observations.jsonl` and `result.json` support
the facts above. `call-envelope-compact-report.json` is a field extraction,
not a second acquisition or an independent archive-integrity verdict.

The result remains unqualified with a real cleanup failure and incomplete
eligible telemetry coverage. No zero-fault/continuous-waveform pass, fixed
physical delay, gap/jitter reduction, default promotion or proof that this
device cannot achieve sub10ms is claimed. Root's next source-only review of
the retained `0x8007051A` handling is separate from this factual receipt.
