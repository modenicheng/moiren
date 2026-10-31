# Process native observations and measured per-wake admission

Implementation commit: `464265793c0cadb4b533d7bf8608cbebf9a3b4a0`, pushed to
`origin/feat/minimal-io-latency` and verified by matching `ls-remote` SHA.

This is software evidence for the open S3 requirement in
`docs/vad-low-latency-goals-2026-10-28.md`. The user's 2026-10-29 direction
defers native VM timing/stability acceptance until suitable hardware. This
slice opens no audio device, installs no driver and performs no deployment.

## Native observations

Each capture bridge owns a single-writer, fixed 256-event atomic history of
completed native drains. Master render owns a separate history of complete
Engine consumption bursts per nonzero native render demand. A graph segment
does not become a separate demand event; partial native writes do not prefetch
or count the same consumption again.

Recording performs scalar atomic publication without allocation, deallocation,
blocking, logging or per-sample clock reads. Preparation allocates the histories.
Control reads make one attempt, returning unknown during concurrent writes.
Snapshots retain event sequences, prefix-overwrite totals, missing/regressed
clocks and counter exhaustion explicitly. The history is not a PCM trace.

`NativeObservationSnapshot.process_delivery` associates actual histories with
the Process PID/creation time, source node, owner generation, fixed 48 kHz input
rate, Engine rate and complete prepared consumption bound. It includes the
exact admitted bridge options and any reused profile's source generation.
Existing generation-pinned handles preserve final updates after live registry
clear/replacement. Registry retirement alone still does not prove worker join.
The current virtual-only owner supplies no measured consumer phase; its absent
history is refused rather than replaced with synthetic device timing.

## Typed evidence and preparation

`NativeProcessDeliveryObservation::measured(expected_process,
expected_generation)` constructs `MeasuredProcessDelivery` off RT. Evidence
has private fields and cannot be deserialized directly as a trusted profile.
Its constructor validates supported rates, stable identity, nonzero owner
generation, two retained events on each side in an overlapping window,
contiguous sequences, clock validity, cumulative delivery accounting and
complete callback bursts. Retained prefix overwrites are explicit; unexplained
internal holes and malformed counts are refused.

`ProfessionalBackend::with_measured_process_bridge` binds explicitly selected
evidence to one source node and its expected originating generation. Initial
validation does not replace preparation-time admission. The real preparation
path accounts for the selected queue's storage, checks the node's stable
source again, negotiates the master, and admits using its complete demand.
Insufficient measured targets return an error without compatibility fallback
or Process activation. Existing explicit/compatible policies retain their
separate entry point.

The budget uses the greater of packet-arrival and retained inter-drain gaps,
ceiling conversion to native input frames, whole-drain capacity headroom, and
the maximum of historical prepared/observed demand and the new owner's
prepared demand. Historical demand is rebound upward when Engine rate changes.
Synthetic evidence admits 1024/512/256 candidates only when the entire tuple
fits; these values are not production presets or measured latency claims.

## Phase and qualification limits

Phase is diagnostic time from a strictly earlier completed producer drain to
a consumption-start event. Timestamp ties retain unknown ordering and never
reduce the conservative all-phase reserve. These stamps are not Process sample
age, a hardware clock, or physical end-to-end latency.

The original slice plan proposed a finite-window queue replay. That proof is
not implemented here: entry inventory and intra-drain packet publication are
unknown, and consumption is stamped at the start of a complete Engine burst.
Inventing an initial fill or treating drain-completion stamps as the exact
availability of every packet would misrepresent the actual queue. A regression
explicitly preserves a pathological consumer-density history that passes the
per-wake floor while failing to establish sustainable queue balance. This
remaining qualification detail is open; per-wake admission does not certify
finite-window occupancy feasibility or continuous output.

The native report is available through the existing app/CLI snapshots, but
automatic profile selection/tuning and an application command to apply a
selected measured profile are not implemented by this backend slice. Existing
app/CLI declared tuples remain functional. Actual matched-route measurement,
negotiated lower targets, phase/packet qualification, stability and sub-10-ms
results remain open or hardware-deferred as recorded in the S1-S3 ledger.

## Verification

Hardware-free checks on the final implementation:

- Audio library: 441 passed, 2 ignored; latency integration: 12 passed;
  Process integration: 5 passed; transport budget integration: 5 passed.
  The library includes 21 new pure measured-profile regressions, real backend
  preparation with fake native negotiation/storage limits, final-generation
  snapshots and capture termination paths.
- Probe example with `vad-probe`: 259 passed. Existing correctness/strict
  collection and packet analysis contracts remain intact.
- Standalone professional RT harness: 100 passed, including actual whole-wake
  event totals, partial native writes and zero allocations/deallocations in
  both precisions. The standalone tool's Linux all-target cross-check passes;
  this is compilation evidence, not a Linux execution/CI result.
- Strict all-target audio Clippy with `vad-probe`, strict all-target app Clippy
  with `gpui-test`, workspace formatting and diff whitespace checks pass.
- Final app library with `gpui-test`: 426 passed, 3 ignored with bounded
  `--test-threads=4`; this does not reclassify the earlier unrelated parallel
  ACK-barrier timeout documented in the monitor CLI slice.

Review found a final-drain gap: Stop or a packet-query error after successful
packet release returned before recording the accumulated burst. Both new
regressions failed with an empty history, then passed with one retained
16-frame drain, matching delivery/telemetry and the original Stop/error outcome.
The capture loop now publishes the accumulated burst before propagating its
terminal result. Target-exit uses the same path; no native period is inferred.

Final review also identified internally consistent shortened suffixes that a
real fixed ring cannot produce: total=5, overwritten=2 and only events 3..5.
That shape could omit a larger early consumer demand while passing sequence
and sum checks. The new regression first reproduced acceptance, then the
validator was tightened to require `retained == min(total, 256)` and exactly
the corresponding overwrite count. This preserves genuine full-ring prefix
overwrites without trusting malformed shorter histories.
The complete audio suite and strict audio Clippy were rerun after this fix,
as was the Linux standalone all-target cross-check. Final read-only review
confirmed the fix with no remaining Important/Critical findings in this scope.

The first standalone harness build failed because the new tests reached a
crate-private producer method through the library rather than the shadow
module. The producer regression now belongs beside ingress telemetry; the
native projection test uses its public observer and late consumer publication.
Review also found a Linux-only gated Process import in the standalone harness;
it is now portable, and the final cross-check passes. An early integration
test compile used the wrong allocator-helper name; this was corrected before
the successful full runs. Failures are retained rather than relabeled PASS.

Logs: `target/s1-s3-20261028/s3-measured-*.log`,
`s3-backend-measured-red.log`, `s3-observations-check.log`, and the focused
test-first logs retained by the implementation workers. No production defaults
change; no native continuity, latency or hardware qualification is claimed.
