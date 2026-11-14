# Process host-ingress elapsed limit

This implements the already-authorized S1-S3 work. Source sample age remains
unsupported for positionless Process capture. The new policy measures elapsed
host QPC time from the packet's publication-start observation, not source age,
capture latency, or physical presentation delay.

## Design and alternatives

1. Use a separate optional `max_ingress_elapsed_100ns` on the Process bridge
   and ClockBridgeConfig. Retain the actual publication-start observation per
   accepted frame. Enforce freshness in the real consumer. This is selected:
   it bounds retained old inventory even below the high-water mark.
2. Shrink the queue or high-water mark alone. Rejected: elapsed time can grow
   while frame inventory remains small.
3. Reinterpret arrival observations as sample timestamps. Rejected: this would
   fabricate Process source-age provenance.

The compatibility default remains None. Some(0) refuses during preparation.
Missing, zero, regressed, or future clock observations do not prove freshness.
The existing sample-age request remains explicitly refused for Process.

## Global constraints

- No native audio, VM, driver installation/deployment, or production default
  change. Preserve unrelated gate.log. Local atomic commits; batch pushes.
- No allocation, deallocation, blocking, per-sample clock read, or unbounded
  producer chasing on RT. Prepare storage and policy off RT.
- Reuse the packet's existing host publication-start observation. Preserve
  native packet stamps and the positionless source-age refusal.
- With the policy enabled, acquire one finite consumer prefix per read. A
  separate fixed host observation after acquisition checks that prefix; new
  concurrent publications remain for the next read.
- Check actual interpolation support before rendering. Stale/unknown support
  clears held frames and the remaining acquired prefix, reports loss/reset,
  and waits for newly primed input with the existing recovery fade. It must
  never preserve a stale two-frame suffix or chase concurrent new writes.
- Clearing a captured prefix may also discard fresh frames within that prefix.
  This conservative recovery is counted explicitly. It is not a waveform
  continuity guarantee.
- Keep whole-native-drain and whole-wake admission; segmentation cannot reduce
  the admitted demand. Preserve queue conservation and non-overlapping loss
  categories with a distinct ingress-elapsed discard field/counters.
- Expose independent requested intent through Process API, app settings,
  actual desktop input controls and monitor CLI. Requested/active remain
  distinct and structural changes still replace an owner.
- Measured/replay/token paths must preserve the policy exactly or refuse it
  explicitly. Current finite replay does not reproduce native freshness
  clocks, so a candidate with the new policy is refused. Baseline traces with
  recorded freshness discards must not silently omit those losses.
- Unknown source age stays unknown. No host-residence limit or passing
  software check qualifies latency or sustained real output below 10 ms.

## Tasks

1. [x] Add optional API/configuration, frame publication-start provenance,
   finite-prefix freshness enforcement, diagnostics and queue conservation.
   Add regression tests first and retain genuine failure evidence.
2. [x] Add native preparation validation and explicit measured/replay guards;
   carry the new discard category through retained/exported evidence.
3. [x] Add app settings, persistence/rebase, desktop controls/visible summaries,
   and monitor CLI forwarding/refusals with focused regressions.
4. [x] Verify actual probe/RT consumers compile, relevant audio/app/RT tests,
   strict lint/format checks, and independent source review. Commit exact
   source/doc paths locally and record remaining native/physical gates.

App field: `TransportLatencySettings.process_max_ingress_elapsed_100ns`.
Monitor flag: `--max-ingress-elapsed-100ns N`.
Process/ClockBridge field: `max_ingress_elapsed_100ns`.
Queue sample/counter loss field: `ingress_elapsed_discarded_frames`.
App measured replacement must refuse an incompatible active/desired limit;
it must not silently clear it.

## Completion evidence

This software slice is complete in local source commits `02bd1f0` (core/API,
loss accounting, admission/refusal consumers and compile dependencies) and
`03e77a0` (desktop, CLI, real click regressions and localization). The exported
first commit compiles independently. No push is performed.

Final results: audio829 passed/3 ignored; probe338 passed; RT182 passed;
full app gpui-test589 passed/4 ignored. Relevant strict Clippy, root/RT formatting
and diff checks pass. Linux audio all-targets compiles with3 cfg warnings.
Core clock/freshness, queue/replay/export, guard, app and localization reviews
have no remaining material finding. Two initial core review findings have
retained runtime RED/GREEN regressions.

Exact limits, failed attempts, commands/logs and remaining native/physical
gates are recorded in
[`../../validation/2026-10-29-process-ingress-elapsed.md`](../../validation/2026-10-29-process-ingress-elapsed.md).
Completing this software slice does not complete the full S1-S3 objective.
