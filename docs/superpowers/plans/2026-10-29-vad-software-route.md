# VAD playback to microphone collection implementation plan

**Goal:** Complete the explicitly requested S1 software route: VAD playback ->
Bus -> VAD microphone, retaining bounded raw capture and honest route evidence.

**Architecture:** Reuse one WindowsVadProvider session, two distinct owned
exports and the existing ProfessionalBackend/EngineHost virtual-realtime owner.
Use the existing changing source and MeasurementSession recorder. The physical
route remains the default; virtual selection is explicit and incompatible
physical intent is rejected, not silently ignored.

**Basis:** S1 in `docs/vad-low-latency-goals-2026-10-28.md` already requires this
exact graph. This implements that approved scope, not a new transport design.
The design/audit options were: reuse production virtual routing (selected), add
a dummy physical master (rejected: changes route), or build a separate direct
PCM loop (rejected: bypasses the real graph). No new product setting/default.

## Boundaries

- No native audio, VM, driver installation/deployment or physical measurement.
- Preserve unrelated gate.log; scoped local commits, no per-commit push.
- Prepare graph/resources off RT; no additional RT allocation/blocking/clocks.
- Two stable IDs under the same actual control root; exact direction and unique
  MMDevice matching. Names/default endpoints are never routing authority.
- Exact stereo Source -> Bus -> Sink graph; no Gain or hidden physical output.
- One confirmed Engine revision ticket admits both independent gate stamps.
- Confirm initial recorder Paused -> Running before source activation/preroll
  within the original Start deadline. Preserve every prepared identity/shape/
  stream/gate stamp; unexplained initial or later changes invalidate collection.
- Stop all peers before joins; release microphone native client before its PDO
  retirement; failure of one endpoint never suppresses cleanup of its sibling.
- Internal software evidence only, always unqualified. Duplicate stereo output,
  source submission and nominal frame arithmetic are not an independent delay
  reference. Unknown sample age and physical latency stay unknown.

## File responsibilities and sequence

1. [x] Extract the existing inline native control module to `live_native.rs`
   without behavior change, keeping parser/report edits separate from owner work.
2. [x] Add portable `vad_route.rs`: route identity/validation and the exact graph
   builder. Test physical compatibility, virtual follower/pacing, distinct IDs,
   graph shape, and refusal of unsupported output targets with RED -> GREEN.
3. [x] Extend `live.rs` parse/help/validation/reporting with explicit microphone
   identity/name. Require owned VAD collection, Internal boundary and no physical
   output/render-only request; persist route intent even in dry-run/failure.
4. [x] Extend `vad.rs` exact resolver to capture direction/root/stable identity,
   returning copied identity evidence with no escaping COM apartment objects.
5. [x] Extend `live_native.rs` aggregate lifecycle: create both, prepare clients,
   open real virtual graph, admit both gates, retain both running/final receipts,
   confirm initial recorder Running without rebasing, and retire both independently.
6. [x] Extend `vad_collection.rs`: schema2 resolved owned input with optional
   exact-ID assertion; route-aware health/final evidence; startup confirmation before
   preroll; recorder-native release before owned endpoint removal. Preserve
   physical schema1/default and fail-fast/correctness distinctions.
7. [x] Execute focused RED/GREEN, full example and relevant audio regressions,
   strict normal/test-profile Clippy, fmt/diff, Linux compile checks; independent
   source review and injected second-endpoint/startup/cleanup regressions.
8. [x] Record exact achieved scope and remaining S1-S3/native gates, then make
   coherent local commits. Do not claim native acceptance or sub-10-ms PASS.

## Initial evidence

Previous goal turn was concrete progress: capture join/report retention and
whole-wake probe repairs committed at a2a275a/2b09537, with docs080712f. Fresh
worktree inspection shows only unrelated gate.log before this slice. Read-only
provider/lifecycle audits are in ignored target/s1-s3-20261028. The extracted
module passes example test-profile compilation before behavior edits.

## Final implementation checkpoint

The source now includes bounded typed-absence-only initial MMDevice publication
waiting, exact prepared-stamp startup confirmation without rebasing, and a
current microphone Running bit so same-stamp Pause cannot pass ongoing health.
The complete frozen example suite passes 423 tests; audio regressions pass
831 tests / 3 ignored. Strict all-target/test-profile Clippy, workspace fmt/diff
and Linux compile checks pass (existing Linux cfg warnings retained).
The [validation receipt](../../validation/2026-10-29-vad-software-route.md)
records exact logs and remaining gates. Final independent re-review finds no
remaining concrete lifecycle/identity/collection blocker. Source `0d7dd81` and
this documentation checkpoint are local commits with no push. The plan's
bounded software deliverable is complete, not native testing or S1-S3 overall.
