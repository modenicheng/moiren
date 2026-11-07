# Measured Process Delivery Admission Plan

This continues the authorized S3 requirement, not a replacement for S1-S3.
Use `executing-plans` and test-first implementation. Independent pure evidence
analysis/review may run as bounded parallel subtasks while native observation
integration stays with the parent.

## Design

Record two separate bounded, single-writer event histories: completed native
Process callback drains in fixed 48 kHz input frames, and actual complete
master consumption bursts in Engine frames. Read QPC at each boundary, never
per sample or Engine segment. Keep missing/regressed clocks and cumulative
event/overwrite counts visible. Each history retains at most 256 recent events;
control reads are one-attempt coherent atomic snapshots, without RT waits,
allocation, logging or mutable-data races.

Attach the histories to immutable Process identity, input node, professional
owner generation, Engine rate and the owner's complete prepared consumer bound.
They remain available through the existing generation-pinned final observation
handle. A native period is still unknown for Process Loopback.

Build typed measured evidence off RT from actual native observation data.
Reject unavailable/malformed/insufficient histories, timestamp faults, unsafe
arithmetic, mismatched identity/generation or unexplained frame evidence.
Use the overlapping retained window only; prefix overwrites outside that window
are explicit, not holes silently omitted inside it. Preserve unknown phase and
same-stamp ordering. Report producer-to-consumer phase without treating arrival
QPC as sample age, a hardware clock or acoustic latency.

Measured admission uses whole-drain burst headroom, maximum valid inter-drain
gap converted with ceiling arithmetic, complete observed consumer demand and
the actual new owner's prepared demand. Keep conservative all-phase reserve;
favourable observed phase never reduces it. A bounded-window feasibility replay
remains open: current stamps do not establish initial inventory, intra-drain
packet availability or when each frame was consumed within a whole burst.
Do not invent an initial fill and claim actual queue feasibility. Retain the
histories and an explicit regression demonstrating the per-wake model's density
limitation. Reconfiguration prepares/replaces an owner; no live
SPSC resizing or automatic production-default tuning.

## Tasks

- [x] Add failing regressions for fixed-capacity clocked event observation,
  atomic coherence, missing/regressed clocks, overwrite boundaries and no RT
  allocations. Implement producer and whole-wake consumer instrumentation.
- [x] Add typed measured evidence with process/generation association, gap/rate
  conversion, burst/phase handling and explicit refusal. Test admitted 1024,
  512 and 256 candidates only where synthetic evidence and demand fit; reject
  stale identity, bad clocks, insufficient data, unsafe target or larger demand.
- [x] Publish real measured observations in Native snapshots and preserve final
  generation handles; wire typed measured evidence into professional backend
  preparation, retaining compatibility/declared policies separately.
- [x] Verify affected audio/app/probe and RT surfaces, strict Clippy and fmt.
  Record failed attempts honestly; update ledger, atomically commit and push
  explicit paths, verify remote hashes. Preserve unrelated `gate.log`.

No native device, VM, host driver installation or deployment is authorized by
this software-only slice. Physical matched-route/timing/stability gates remain
open under the user's instruction; do not claim all S1-S3 complete.

Implementation committed and remote-verified as `4642657`. Evidence and review
fixes are recorded in `docs/validation/2026-10-29-process-measured-admission.md`.
The checked tasks describe this bounded per-wake slice only. Queue-feasibility
replay, actual app selection/application of a measured profile and all physical
qualification remain open; completing this plan does not complete S1-S3.
