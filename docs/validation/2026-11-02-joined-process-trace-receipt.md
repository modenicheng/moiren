# Opt-in joined Process trace diagnostic

## Scope

The closed-prefix coverage correction is in `8d080f5`. This increment adds an
independently inspectable diagnostic for the original S3 desktop acceptance:
both final measured histories, full-archive totals and the exact predecessor
identity must be inspectable rather than inferred from UI packet/read counts.
It does not add a native-proof importer, change candidate authorization or
establish a successful native Evaluate/Apply sequence.

The cleanup worker emits one compact JSON line after `drop(host)` has joined
the owner, retained final observation handles have been sampled and the trace
has been finalized. Emission precedes publication of `Completion::Reaped`.
Enable only this diagnostic with:

```text
RUST_LOG=info,moiren::audio::process_trace=debug
process_trace_cleanup_receipt=
```

The normal `info` filter disables serialization entirely. No logging work is
added to the audio thread, and no queue, buffer, ring, timer or default changes.
The serializer borrows the immutable retained evidence and writes through a
128 KiB bounded sink; oversize and serialization failure produce explicit
small error receipts rather than truncated success. A diagnostic refusal does
not skip cleanup publication or grant Apply authority.

## Evidence semantics

The payload records app generation, document, node, PID/creation time, native
generation, host/graph revisions, Closed/fault/cleanup state and collection
refusal separately from final measurement success/error. A successful
measurement is the exact retained `MeasuredProcessDelivery`; its producer
drains and consumer whole wakes are not reconstructed from graph segments.

On measurement refusal, only matching immutable post-join observations are
included. Missing, stale and multiple matches remain explicit; a later live
registry is never substituted. Archive packet/read counts, offered supply,
requested demand, closure, overwrite/clock/saturation/refusal and the outer
publication gap are summarized from the retained full archive. Incomplete
histories and arithmetic/clock errors remain unknown, not suffix totals.

The receipt always says `qualified=false`. Its diagnostic scope does not
authenticate native provenance. Software fixtures remain CallerSupplied; no
private origin label is invented and no replay is run just to label a log.

## Verification And Runner

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-joined-receipt-20261102-2235`.
Builds use `git archive 8d080f5` plus exactly three reviewed app overlays,
not the existing dirty probe/VAD sources. `parent-source-verification.json`
checks all 1368 baseline files and the one addition/two modifications against
the frozen source hashes. The build is not an unmodified-commit build.

- Actual RED: five receipt assertions fail before implementation; the original
  RED log/exit receipt is retained. Nine focused receipt tests then pass,
  including bounded serialization, default-off filtering, post-join ordering,
  exact retained measurement, refusal/missing/stale/multiple observations and
  no mutation of cleanup evidence.
- Fresh feature-enabled workspace: 2351 passed, 9 ignored, 0 failed.
  All-target check, strict Clippy, workspace formatting and release build exit0.
- An additional concurrent app gpui suite records 669 passed, 1 failed,
  3 ignored: `graph_meter_frame_repaints_node_bars_across_animation_only_frames`
  fails `simulate_next_frame(cx) > 0`. The exact unchanged test passes alone;
  its original failure is retained, not relabelled or hidden by weakened tests.
  The subsequent complete serial feature-enabled app suite passes 732 tests,
  4 ignored, 0 failed with identical source. The separate default app suite
  passes 604 tests, 3 ignored. Contention remains a possible explanation for
  the single animation failure, not a demonstrated cause or product fix.
- The evidence-only Python checker has 27 passing tests on the host and again
  in the remote isolated Python environment. Actual Rust-serialized software
  success/refusal fixtures exercise the CLI; they are not native acquisition.
- Seventeen PS5 fake-clock/static/parse runner checks pass. Independent
  read-only review finds no blocking defect in receipt integration/contract.
  A second review finds that the evidence wrapper swallowed checker exit1/2;
  a genuine failing harness assertion and corrected GREEN are retained. The
  exact final wrapper statement also propagates0/1/2 in isolated subprocess
  tests. This evidence-only repair does not change application source.

The runner independently freezes predecessor identity before Start, enables
only the receipt target, leaves the hot capture helper unchanged and keeps
the operational result separate. After all owned app/source processes exit,
a separately bounded offline checker can audit a single original stdout log.
It rejects malformed/duplicate/truncated/nonfinite/oversized JSON, wrong or
duplicate predecessor identity, incomplete histories, bad clocks, inconsistent
packet/drain/wake frame totals and incorrect recomputed gap reserve. A pass
means numeric/identity consistency only, not quality, native trust, token
issuance, sustained service, waveform, sample age or physical latency.

## Native Boundary

The new release is staged at
`D:\MoirenTest\20261102-joined-receipt-2235\bin\moiren-app.exe`:
34,150,912 bytes, SHA-256
`67025240CDDA7CFC6C277928FE20897806D4BB8D498D21B7E99F7003071AE974`.
Staging and offline checker tests are not an audio launch.

The November2 23:00:56 deployment verification checks all eight deployed
dependencies and has 520,120 KiB free physical memory, below the unchanged
8,388,608 KiB guard. No audio owner, native run directory or task is
started to bypass that refusal; protected OBS/QQMusic identities are preserved.
After the reviewed intent is authorized,23:02:26 verification briefly has
8,781,028 KiB free and no task/run/owner. One launch command then refuses at
its own first memory check before task registration. The23:02:50 read-only
audit has5,218,568 KiB free and confirms no task, run directory or audio owner.
No automatic retry, guard relaxation or resource-kill workaround follows;
the failed launch command and changing resource readings are all retained.
No host audio/UI process is launched or manipulated. First-packet investigation
remains deferred. Successful native closed-prefix Apply and its actual joined
receipt, waveform quality and calibrated physical qualification remain open.
Independent software development need not wait for physical equipment, and an
unchanged stability soak is not used as a substitute for a new acceptance test.
