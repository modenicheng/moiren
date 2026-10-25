# Auto-Apply Graph Plan

**Status:** Design accepted on 2026-10-25/26. Supersedes the "Keep explicit Apply
semantics" decision in `2026-10-25-mixer-strip-redesign.md` §9. Implementation
runs on the `feat/auto-apply-graph` worktree branch, based on a snapshot commit
of the in-flight mixer strip redesign.

**Goal:** Every graph edit takes effect by itself. The Apply graph button,
pending badges, and release-to-apply hacks disappear from all views. Meters
never stop because of an edit. Failures speak like the Rust compiler: where it
is wrong, why, and how to fix it — never a description of internal runtime
state.

## Core principle

Edits split into two classes, both automatic. The user never "submits".

| Class | Covers | Effect |
| --- | --- | --- |
| Parameter (no prepared-structure change) | strip gain/balance/mode/mute/solo/dim, send gain/pan/mute, gain/pan/mono/swap, compressor | Live parameter write through the existing SPSC control channel. No recompile, no pending state, next audio block. |
| Structure (prepared-structure change) | add/remove node, connect/disconnect, bus inputs, send tap changes, exports, builtin config, source/output bindings | Automatic background rebuild. Success: atomic plan swap. Failure: keep running the old graph with zero interruption; retry on the next edit. |

`AppRequest::ApplyGraph` survives as a service-level request (virtual-bound
transport start, settings restart transaction). Only the manual UI semantics
die.

## Current defects (checked 2026-10-26)

- Strip controls go through `GraphEdit::SetMixer`, which has no entry in
  `edit_parameter_values` (`src/service/graph.rs:195`) — there is no live path,
  so a fader drag can only ever take effect after recompilation.
- Every successful edit bumps `candidate_revision`; `active_revision` catches
  up only after an explicit Apply is confirmed by the engine.
- Meter reads are gated on `active_revision == candidate_revision`
  (`src/ui/window/mixer.rs:389,410-414,416-421`, `src/ui/monitoring.rs:330`,
  `src/ui/window.rs:401-409`, tray and graph canvas). One fader tick kills
  every meter until Apply confirms.
- `ChannelStrip` ignores `ProcessParameters` entirely
  (`crates/moiren-engine/src/processor/builtin/strip.rs`) — no slots exist to
  make live even if the service wanted to.
- Errors are flat strings (`record_error`) with no node/edge identity; lower
  layers (`GraphError`) discard location entirely.

## Phase 1 — Engine: ChannelStrip parameter slots

Follow the existing `Send`/`Pan` builtin pattern; no new RT machinery.

- `strip.rs`: `GAIN` (Float 0..16), `BALANCE` (Float −1..1), `MODE` (Enum,
  variants = channels+3: ST/MONO/L/R/SWAP/CH-n), `EFFECTIVE_MUTE` (Bool);
  `ChannelStrip::parameters(id, config, channels)` seeds initials from
  `StripConfig`; `validate_parameters` mirrors `Pan`; `process()` reads
  parameters per segment with per-frame `FloatRamp` sampling (the
  `runtime/mix.rs` consumption shape). Config becomes compile-time initial
  only.
- `compiler/plan.rs` `Builder::strip`: POST instances get all four specs; PRE
  instances get BALANCE/MODE/EFFECTIVE_MUTE only (PRE bypasses gain — the test
  `strip_pre_fader_bypasses_only_strip_gain_and_respects_mute` is the
  contract).
- `CompiledBindings`: `strips: BTreeMap<NodeId, Vec<StripParameterKeys>>`;
  `host/parameters.rs resolve()` fans one node target out to every instance
  key (PRE lacks GAIN, so it is skipped); the Router submits one request per
  resolved key under a single receipt (Applied when all applied; first
  Rejected rejects).
- `host/project.rs`: `validate_parameter` strip arm with domain checks;
  `parameter_values()` emits per-strip effective values so `send_values`
  diffing pushes them live.
- `host/supervisor.rs` `same_structure`: stop comparing strips (value changes
  no longer force recompiles — same treatment as edge `SendParams`);
  `host/preparation.rs` folds recorded strip intent so recompiles keep live
  values.
- Tests: compiler strip submits (MODE ramp==0, PRE fan-out); host updates —
  strip value changes route without recompile.

## Phase 2 — Service: live SetMixer and edit classification

- `edit_affects_prepared_structure(edit)` in `service/graph.rs`: structure =
  AddNode, RemoveNode, Connect, Disconnect, AddBusInput, RemoveBusInput,
  SetExports, SetBuiltin, BindSource, BindOutput, ClearBinding, and SetSend
  when the tap changed.
- `edit_parameter_values` gains SetMixer → BALANCE/MODE for the edited node.
- New `submit_effective_strip_parameters(c)` in the managed driver: recompute
  every strip's effective value from the candidate snapshot (reuse the
  `mixer_configs` solo-audibility closure and dim/mute folding), diff against
  the driver cache, submit GAIN/EFFECTIVE_MUTE deltas. Solo fan-out, dim, and
  mute are covered; policy stays in the service, DSP stays dumb.
- `candidate_revision` keeps bumping for every edit (change detection and ACK
  machinery unchanged); parameter-class edits never create pending state.
- Legacy driver: SetSend tap changes classify as structure; SetMixer stays
  candidate-only until Phase 4 auto-apply covers it.

## Phase 3 — UI: meters decoupled from revisions

- `NodeMeters::update`: drop the revision clause (`active = phase Running`);
  keep generation re-fencing and the 500 ms stale rule — during a plan swap
  meters keep the previous data and only mark stale after the timeout, never
  blank instantly.
- Remove every `active_revision == candidate_revision` gate:
  `meters_frame_active`, `refresh_observation_nodes`, mixer `meter()` (keep
  the legacy probe's desired-binding check), tray strips, graph canvas peak
  rings, inspector observation copy.

## Phase 4 — Service: automatic apply for structure edits

- Managed driver: `auto_apply_deadline: Option<Instant>` (200 ms trailing
  debounce, rearmed by each structure edit); while armed `is_drained()` is
  false so the 10 ms park cadence holds; on expiry arm the existing apply
  (`apply_audio_settings = current`, `apply = true`; `automatic_startup`
  untouched). Existing coalescing reused: 16-deep edit ledger, newest
  candidate per edit, Capacity retries next round.
- Failure policy: `EditOutcome::Rejected` → `preparation_failed` keeps the
  owner running — that is already the desired "keep running the old graph"
  behavior. No retry loop; the next edit retries naturally.
- Legacy driver: structure edits auto-trigger its existing ApplyGraph path
  (a legacy apply restarts the session, same as the manual button did —
  accepted limitation; the managed driver is the primary path).

## Phase 5 — UI: remove the Apply face; rustc-style faults

- Delete the apply button (`controls.rs apply()` and its four render sites),
  mixer Pending badge, tray badge, the whole `release_applies` mechanism and
  the tray's release/apply hacks.
- Structured faults: `GraphSnapshot.faults: Vec<GraphFault>` with
  `{subject: Node/Route/Graph, phase, cause, hint}`; identity is captured at
  `edit_candidate` arms and `configured_engine_project` (`ProjectError`
  already carries `NodeId` in 7/10 variants) — lower layers discard it, so it
  must be captured where context still exists.
- The global error strip becomes a clickable fault list rendered like rustc:
  `错误：原因` / `位置：<user-visible name>` (channel label, matrix route
  naming, device names) / `提示：<how to fix>`. Clicking selects the channel
  or route and opens the Inspector. The group clears on the next successful
  activation. Parameter `Rejected` becomes a fault naming the target;
  `Superseded`/`Closed` stay silent.
- Copy sweep: no internal-state narration anywhere ("graph", "engine",
  "session", "plan" are forbidden words in user-facing copy); validation
  strings upgraded to cause+hint pairs.

## Phase 6 — Docs

- FRONTEND.md: apply-free interaction model, meter semantics, fault list spec.
- This document tracks final status.

## Acceptance

- Drag a fader: audio changes live, meters keep dancing, no Apply exists.
- Delete a node / rewire: rebuild completes within ~300 ms of the last edit
  and swaps atomically; meters do not blank during the swap.
- Force a compile failure: audio continues, meters continue, the draft stays,
  only the fault list speaks; the next edit retries and clears.
- Mute/Solo/Dim/mode/balance/send gain/pan/mute all live, zero recompiles.
- No UI element hides data based on revision equality.

## Verification

Run serially per the repository convention:

```powershell
cargo test --locked -p moiren-engine -j 2 -- --test-threads=2
cargo test --locked -p moiren-app -j 2 -- --test-threads=2
cargo test --locked -p moiren-app --features gpui-test -j 2 -- --test-threads=2
cargo check --locked --workspace --all-targets -j 2
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings
```
