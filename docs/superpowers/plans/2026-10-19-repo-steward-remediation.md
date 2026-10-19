# Repo steward audit remediation plan

> **Source:** read-only repo-steward audit on 2026-10-19 (four parallel crate
> audits + hygiene scan + spot-check verification). This document is the
> master checklist; statuses below reflect the remediation executed the same
> day (commit range `aa19314..17f04e5`).

**Goal:** Consolidate duplicated invariants into single sources, fix the three
drifts the audit found, converge test scaffolding, and land the structural
cleanups — all without changing observable behavior except where a drift fix
is explicitly flagged.

**Status legend:** `[ ]` pending · `[x]` done · `[~]` partial (see notes) ·
`[!]` blocked on a decision or hardware validation.

## Global constraints

- No allocation/blocking changes on RT paths; all extractions live in the
  control plane or in pure functions.
- Behavior-preserving refactors only, except items explicitly marked
  **drift fix** (numerical-equivalence or error-handling alignment, each with
  a test).
- Per AGENTS.md: `cargo fmt --all`, `cargo clippy --locked --workspace
  --all-targets -- -D warnings`, and the affected crate tests after each
  group; batches were committed and pushed to main per user instruction.
- `driver/` stays out of the repository for now (decided 2026-10-19). Do not
  add ignore rules that would hide it; `.vs/` and `.v2c/` ignore entries are
  still correct.

## Recorded decisions

- **D1 (2026-10-19, user):** `driver/` is not entering the repo for now.
- **D2 (open):** `docs/superpowers/plans/2026-10-14-engine-stress/` and
  `2026-10-14-frequent-swap/` contain ~489 KB of tracked `*.stderr.log` /
  `*.stdout.jsonl` run outputs. Keep as plan evidence or strip to summaries —
  awaiting user decision. Not touched by this plan.
- **D3 (open, hardware):** Switching the default audio path (default
  `AudioSettings` → professional backend) and retiring the legacy stack is a
  behavior change requiring on-device verification. Excluded from this plan's
  execution; see item W-K. Prerequisite W3/W4 kernel unification is done.
- **D4 (resolved during W3):** PERIODICITY_LOCKED: professional keeps its
  structural exact-or-fail contract (retry would silently open a
  non-requested period); the retry policy is parametrized in the shared
  kernel (`EnginePeriodicityPolicy`) and pinned by a const assert + test.
  NOPERSIST: not unified — `LegacySharedInit` named params carry each stack's
  flag set.
- **D5 (open, small):** Whether the legacy driver should also apply `SetSend`
  in real time (A8); requires edge-addressable bindings in legacy
  `CompiledBindings`.
- **D6 (accepted residual):** `managed_stress` example keeps its
  `Arc<WindowsBackend>` forwarding impl — the engine-side Box forwarding
  covers `Box<dyn EngineBackend>` only; an `Arc` blanket impl is not worth API
  surface for one example.

## Group 1 — Zero-risk extractions (each independently landable)

### moiren-engine

- **[x] E1 — Dynamics support helpers + smooth-coefficient drift fix.**
  Helpers live in `src/processor/builtin/dynamics.rs` (`db_to_gain`,
  `sanitize_sample_rate`, `smooth` on the `exp_m1` form, `float_specs`,
  `validate_float_domains`, `validate_linked_io`); gate/limiter unified onto
  the `exp_m1` smoothing with equivalence + slow-ramp tests. The interrupted
  first attempt's buggy test expectations (gate off-by-one, limiter start
  gain/tolerance) were corrected before landing. Delay's handwritten
  `parameters()` intentionally not merged (different shape).
- **[x] E2 — Pan/balance coefficient single source.** `Pan::coefficient` is
  `pub(crate)`; `compiler/send.rs` reuses it.
- **[x] E3 — Supervisor idle predicate.** `Supervisor::idle()` replaces the
  three hand-written triples.

### moiren-windows-audio

- **[x] W1 — FollowerServo extraction.** `professional/bridge.rs` owns
  `FollowerServo { target, correction }` (`observe`, `start_target`);
  constants `FOLLOWER_GAIN_PPM` / `FOLLOWER_SMOOTHING` /
  `CORRECTION_PPM_LIMIT`; both start-target formulas keep their exact integer
  arithmetic paths.
- **[x] W2 — Endpoint projection merge.** `catalog.rs::endpoint_rows` is the
  single projection; capture/render keep thin public types via `From`.

### moiren-app

- **[x] A1 — Session bookkeeping methods.** `session_started` /
  `session_stopped` in `service/driver.rs`.
- **[x] A2 — Error string constant.** `PROFESSIONAL_SETTINGS_UNSUPPORTED`.
- **[x] A3 — graph.rs helper reuse.** `inspector_toggle(id, cx)` parametrized;
  `StudioShell::begin_pointer_session` replaces the three preamble copies.
- **[x] A4 — Mixer faders/sends twin merge.** `sync_sliders<K>` +
  `SliderEdit` alias.
- **[x] A5 — capture.rs snapshot helper.** Local `capture(cx, path)` closure;
  the optional phase split of `run_ui_capture` was skipped.
- **[x] A6 — devices.rs row reuse.** `input_rows`/`output_rows` are
  `pub(crate)`.

### Repository hygiene

- **[x] H1 — `.gitignore`:** `.vs/` and `.v2c/` ignored; `driver/` left
  visible per D1.

## Group 2 — Rule single-sourcing

### moiren-signal

- **[x] S1 — Sender-slot bitmap helpers.** `retired_slot_snapshot`,
  `next_sender_slot`, `reclaim_retired_and_clear_adopted`,
  `entry_storage_mut`, `SubscriptionLease::staged`.
- **[x] S2 — `retire_sender` single implementation.** `storage.rs`
  `detach_sender`/`retire_sender` with the race-invariant comment; registry
  and dispatcher call them. (`fanout.active` move in the dispatcher Latest
  path is unobservable — private non-atomic field, no ordering relation.)
- **[x] S3 — Stream delivery-eligibility predicate.** `StreamSender::eligible`
  + `record_drop`; per-site liveness/`counted` checks kept because the two
  implementations are genuinely not equivalent.
- **[x] S4 — `try_write` shares production validation.** Via shared
  `check_write`/`acquire_slot` (literal delegation impossible: `&mut self`
  cannot produce the owned guard); check order and atomic-read count
  unchanged.
- **[x] S5 — `WriteError → PublishOutcome` flatten.** `error::
  flatten_write_error` used by the three call sites.
- **[x] S6 — `ring_index_bound`.** `budget::ring_index_bound(depth)` replaces
  the three hand checks.
- **[x] S7 — identity allocator.** `checked_identity` uses `try_update`
  (verified to return the pre-update value — exact replacement).

### moiren-core

- **[x] C1 — Port-twin consolidation.** Private `create_node_with_widths`,
  `add_port`/`remove_port` by `PortRole`; public thin wrappers keep the API.
- **[x] C2 — `check_edge` shared.** Single-edge order follows `connect`;
  `validate`'s unreachable double-fault state now reports
  `InvalidSendParameters` (not constructible via public API).
- **[x] C3 — protocol frame helpers.** `REPLY_BODY_BYTES = 40`, opcode
  constants, const-generic `read_frame`/`write_frame`; wire format
  byte-identical.

### moiren-engine

- **[x] E4 — Per-processor parameter knowledge single source.** Each builtin
  has a `resolve*` entry applying intent by named constants (no positional
  index backfill); `specs_accept_initials`/`accepts_parameter` in
  control/schema.rs serve project validation and rack checks. ParameterId
  numbering unchanged (`OUTPUT_GAIN=8` pinned by test).
- **[x] E5 — Delay bounds single source + precheck drift fix.**
  `Delay::validate_settings` (60 s + frames/bytes incl. `MAX_BUFFER_BYTES`)
  shared by `Delay::new` and the rack precheck; 563 MiB rejection test added.
- **[x] E6 — Channel bound constants.** Single `builtin::MAX_CHANNELS`.
- **[x] E7 — `EngineConfig::validated`.** One owner; compiler reuses.
- **[x] E8 — `IoSchema::from_io`.** Single construction point.
- **[x] E9 — Observer window continuity.** `WindowContinuity` +
  `TimelineHeader` (+ `validate_single_input`) in processor/mod.rs; the
  queue-based `LevelMeter` keeps its boundary comment.
- **[x] E10 — Swap compatibility predicate.** `swap_rejection` shared by the
  three sites; control+RT double-check preserved; `apply_pending_plan` split
  into four `#[inline]` phases (no allocation/lock/atomics added).
  `prepare_reuse` additionally checks revision via the shared source —
  reachable behavior unchanged (60 runtime tests pass).
- **[x] E11 — `host/signals.rs` get-or-register.** `topic_or_register`,
  `subscribe_active`, `reader_handle`, `unregister_topics`; public API
  unchanged.
- **[x] E12 — Dead/alias API.** `PlanSnapshot::single_sink` deleted;
  `OUTPUTGAIN` alias dropped, `OUTPUT_GAIN` kept and pinned.

### moiren-windows-audio

- **[x] W3 — WASAPI negotiation kernel.** `src/negotiation.rs`: GUID consts,
  `parse_descriptor`, Client3 capability query + fallback matrix,
  `friendly_name`, `LegacySharedInit` flags, `EnginePeriodicityPolicy`
  (decisions per D4). Descriptor/Client3/legacy-init plumbing shared by both
  stacks; exclusive branch untouched.
- **[x] W4 — `open_shared_client(direction)`.** Full shared open sequence for
  capture/render; professional reuses the clean sub-steps only.
  `StageError` + `From` impls unify the ×4 `api()` shape (messages
  unchanged).
- **[x] W5 — Lease family parametrization.** `PacketLease<'a, C:
  LeaseClient>` with `LeaseFallback` enum (CapturePacket / RenderSilent /
  RenderEmpty); no default unified.
- **[x] W6 — Dead/narrow API.** `stream_observations()` and
  `resample::capacity()` deleted; `push`/`SampleCodec::new` test-gated;
  `codec`/`resample` are `pub(crate)`; rt-check locally includes those files
  via `#[path]` (harness suite 68/68).

### moiren-app

- **[x] A7 — InputSelection ⇄ SourceMode mapping.**
  `UiState::apply_source` + `current_selection()` replace the five matches.
- **[x] A8 — GraphEdit → parameter mapping.** Pure
  `graph::edit_parameter_values` (managed version canonical, incl. SetSend);
  legacy filters out SetSend explicitly — whether legacy should support it is
  open decision D5.
- **[x] A9 — Node-state validation single source.**
  `graph::validate_node_state` + `InputSelection::validate`; dummy-SessionSpec
  trick removed; messages unified to the per-rule zh style; project layer
  keeps node-level field paths.
- **[x] A10 — Stale-observation guard.** `ServiceContext::fresh` for
  snapshot() filters; managed writers use a local `fresh` closure; UI keeps
  one display-level fallback (`fresh_observation`).
- **[x] A11 — `managed::Backend<S>` newtype removed.** Enabled by
  `impl EngineBackend<S> for Box<dyn EngineBackend<S>>` in engine
  (commit c97730d); `lifecycle.rs` passes the boxed backend directly. See D6
  for the managed_stress example residual.
- **[x] A12 — Catalog refresher.** `service/jobs.rs::CatalogRefresher`;
  automatic-startup retry divergence documented per side.

## Group 3 — Test scaffolding convergence

- **[x] T1 — engine `tests/common/`.** `compile_config`/`engine_config`,
  `connect`/`connect_with`, `test_engine` + `TestIo`, `CountingSource`,
  `pump`, `assert_samples_close`/`assert_samples_exact`; src-side
  `test_support.rs`; `wait_until` in host/support.rs. Deliberately kept:
  `src/runtime/swap.rs` in-crate `engine()` (cfg(test) import boundary), the
  reuse-registry second engine (scenario), spectrum/observers budget
  variants (behavior), `wait_recording` (diagnostics-rich domain helper).
- **[x] T2 — signal test utils.** `tests/common/mod.rs` pump helper (bounded
  1000 + panic semantics); `spawn_parked_drop` replaces the four Resume
  blocks; `tests/allocation.rs` includes `src/test_alloc.rs` as the single
  counting-allocator owner.
- **[x] T3 — windows-audio test commons.** `tests/common/alloc_guard.rs`
  (six users migrated; EventAllocator stays independent — global-allocator
  uniqueness + cross-thread attribution), `tests/common/clock.rs` +
  `clock_sim.rs`, `examples/common/mod.rs` arg parsing.
- **[x] T4 — app UI harness.** `src/ui/window/test_harness.rs`
  (`spawn_service`, `initial_projection`, `finish_service`, `wait_until` /
  `wait_for`, `test_shell`, `shell_with_state`) mirrored by
  `tests/common/mod.rs` for integration binaries; ~26 exit/join pairs, 8
  spawn/snapshot preambles and 5 drifted deadline loops converged. Kept
  unmigrated (documented): managed `Rig` tick-driven waits,
  diagnostics-dumping domain helpers, non-adjacent exit/join pairs with
  interleaved assertions, custom-driver spawns.
- **[x] T5 — examples arg parsing.** `examples/common/mod.rs`.

## Group 4 — Structural

- **[x] E13 — process() dual-arm unification.** `buffer::view::FrameIo`
  (`SeparateFrames` / `InPlaceFrames`, all `#[inline]`) lets all seven
  processors write the kernel once as `process_linked<S, Io: FrameIo<S>>`.
  Per-processor arm-diff audit done; compressor's
  complete-detection-before-overwrite ordering preserved; eq/gain hoisted
  loop-invariant coefficient/ramp reads (bit-identical results). No dyn, no
  allocation.
- **[x] E14 — Long-function phase splits.**
  engine: `prepare` → five named stages + `CompileInputs`; `plan.rs::build`
  rack block → `lower_rack`.
  windows-audio: `open_inner` → `select_format` / `negotiate_exclusive` /
  `negotiate_shared_exact`; `run_stream` → `record_callback` +
  `WakeTrace`/`CallbackOutcome`.
  app: managed `poll` → ten named phases (ACK-barrier comments preserved);
  node_inspector → `BuiltinFieldSpec` tables + `step_button`.
- **[x] A13 — `ui/state.rs` presentation extraction.** `ui/presentation.rs`
  holds `phase_text`, `latency_apply_text`, `native_timing_rows`,
  `elapsed_text`, `control_feedback`, `fresh_observation` taking snapshots;
  StudioShell field grouping (optional) skipped — fields already cluster by
  lifecycle.
- **[x] A14 — NodeId strong typing in UI.** `selected_node` /
  `node_positions` / drag/selection state are `NodeId`; 14 production
  Debug-string key sites removed; explicit conversions remain only at the
  project-document boundary and the canvas view's own string-id identity
  (hit-test API — view-layer concern).
- **[x] W7 — rt-check out of `src/`.** Now `tools/rt-check/` (own
  `[workspace]`, so no root exclude needed); CI path updated; the ~2.6 GB
  stale `target/` under src physically deleted; 68/68 green at the new
  location.
- **[x] W8 — Example-only library modules.** Feature-gated behind
  `probe-tools` (not default): `probe`, `exclusive_probe`, `physical`,
  `swdevice` + their examples' `required-features`. Migration was rejected:
  the modules depend on ~700 lines of internal COM machinery (`owner`,
  `catalog`, `capture_format`) that would have to become public. The 12
  module tests still run under `--features probe-tools`; CI now runs a
  dedicated probe-tools clippy step.
- **[x] W9 — asio_probe test relocation.** Pure logic (options,
  ProgressMonitor, run_stream over FakeStream) + 14 tests moved to
  `src/professional/asio/probe.rs`; the example keeps `main` + reporting.
- **[!] W-K — Default path → professional + legacy retirement.** Blocked
  per D3: behavior change on default settings, needs hardware validation
  and explicit user go-ahead. Prerequisite W3 is done, so the eventual
  switch is a routing change rather than a semantics change.

## Cross-crate notes (recorded, mostly no action)

- `HostId` name collision (`moiren-signal/src/host/binding.rs` vs
  `moiren-engine/src/host/snapshot.rs`): rename deferred until next touched.
- Engine `BoundIo` forwarding wrapper, `compile_update` convenience forward,
  `EngineObserver` subscription forwards: kept — documented encapsulation
  reasons.
- windows-audio `src/stats.rs` peak/RMS vs engine meters: intentional
  (probe diagnostics vs exact windows).
- 256-channel limits across crates: ASIO driver-domain vs engine bound —
  coincidental; engine side now uses the single `builtin::MAX_CHANNELS`.
- moiren-core `LogicalGraph::validate_connection` and moiren-signal
  `copy_latest`/`copy_next`: still test/example-only consumers; doc-comment
  or remove when their intended consumers land.
- moiren-app `ui/units` consolidation of the four dB-formatting helpers
  (controls/node_inspector/mixer): not done — domains differ; revisit if a
  fifth copy appears.

## Validation

- Baseline (pre-change, 2026-10-19): `cargo test --locked --workspace` green.
- Per batch: affected-crate `cargo test` + `cargo clippy --all-targets
  -D warnings` + `cargo fmt --all -- --check`, plus `cargo test -p moiren-app
  --features gpui-test` for app/UI batches, rt-check suite for
  windows-audio batches, and `cargo test --features probe-tools` for W6/W8.
  Test counts preserved at every step (e.g. windows-audio 338 → 337+5 ignored
  after probe tests moved under the feature; app 174 unit / 210 gpui-test
  unchanged; engine 314).
- Closeout: full-workspace `cargo test`, `clippy -D warnings`, `fmt --check`
  on the final tree (commit `17f04e5`).
