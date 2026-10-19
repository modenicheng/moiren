# Repo steward audit remediation plan

> **Source:** read-only repo-steward audit on 2026-10-19 (four parallel crate
> audits + hygiene scan + spot-check verification). This document is the
> master checklist; update the `[ ]` status of each item as work lands so
> nothing is lost across sessions.

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
- Per AGENTS.md: run `cargo fmt --all`, `cargo clippy --locked --workspace
  --all-targets -- -D warnings`, and the affected crate tests after each
  group; no commits unless requested.
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
  execution; see item W-K.
- **D4 (during W-K):** PERIODICITY_LOCKED retry divergence: align professional
  with legacy's single retry **only if** professional's strict-period contract
  permits fallback; otherwise parametrize and document. NOPERSIST flag
  difference: parametrize, do not silently unify.

## Group 1 — Zero-risk extractions (each independently landable)

### moiren-engine

- **[ ] E1 — Dynamics support helpers + smooth-coefficient drift fix.**
  Extract into `src/processor/builtin/mod.rs` (or `builtin/dynamics.rs`):
  `db_to_gain` (verbatim ×3: `compressor.rs:134`, `gate.rs:94`,
  `limiter.rs:70`), `sanitize_sample_rate` (×3: `gate.rs:107-113`,
  `limiter.rs:74-80`, `eq.rs:70-74`), `float_specs` (parameters() constructor
  shape ×5), `validate_float_domains` (DOMAINS loop ×4:
  `compressor.rs:197-216`, `gate.rs:170-187`, `limiter.rs:106-123`,
  `eq.rs:133-150`), shared `validate_io` (gate/limiter identical).
  **Drift fix:** unify `smooth` on the `exp_m1` form used by
  `compressor.rs:152-158` — `gate.rs:98-105` and `limiter.rs:82-89` currently
  use the cancellation-prone `1.0 - exp()` form the compressor comment
  explicitly avoids. Add an equivalence test documenting
  `-exp_m1(-x) == 1 - exp(-x)` and a slow-ramp step-preservation test before
  switching.
- **[ ] E2 — Pan/balance coefficient single source.** `pan.rs:51-57`
  `coefficient()` duplicated inline in `compiler/send.rs:113-119`; make it
  `pub(crate)` and reuse (send keeps its `channels != 2` early-out).
- **[ ] E3 — Supervisor idle predicate.** The
  `preparing.is_none() && pending.is_none() && cleanup_inflight == 0` triple
  appears at `host/supervisor.rs:182`, `231-233`, `241-245`; extract
  `fn idle(&self)`.

### moiren-windows-audio

- **[x] W1 — FollowerServo extraction.** `professional/bridge.rs:78-85` and
  `professional/owner.rs:503-509` are the same 8-line drift servo (magic
  numbers 500.0 / ±2000 / 0.01 also at `resample.rs:78`). Extract
  `FollowerServo { target, correction }` with `observe(queued) -> ppm` and
  `start_target(capacity, period)`; the two start-target formulas
  (`bridge.rs:65`, `owner.rs:460-464`) parametrize. Named constants. Pure
  move, no behavior change.
- **[x] W2 — Endpoint projection merge.** `capture/wasapi/endpoint.rs:22-57`
  and `render/wasapi/endpoint.rs:26-65` are the same CatalogSnapshot → rows
  mapping (4× `format!("{}: {}", stage, hresult)`); share one projection with
  flow/support params.

### moiren-app

- **[ ] A1 — Session bookkeeping methods.** `service/driver.rs:385-392`
  (Start) vs `523-533` (ApplyGraph) repeat a 7-line ownership-transfer block;
  Stop (`398-403`) and `begin_exit` (`790-798`) repeat subsets. Extract
  `session_started`/`session_stopped`.
- **[ ] A2 — Error string constant.** `"legacy audio driver cannot apply
  professional audio settings; use the managed engine driver"` ×3 at
  `driver.rs:370`, `377`, `520`.
- **[ ] A3 — graph.rs helper reuse.** `window/graph.rs:408-419` re-inlines
  `controls.rs:145-156` `inspector_toggle` (parametrize id); the
  `on_select_node`/`on_select_wire`/`on_port_down` shared preamble
  (`graph.rs:94-125`, `339-369`, `247-272`) becomes
  `begin_pointer_session`.
- **[ ] A4 — Mixer faders/sends twin merge.**
  `window/mixer/model.rs:54-94` vs `95-132`: parameterize by key type +
  edit-constructor closure.
- **[ ] A5 — capture.rs snapshot helper.** The 7-line screenshot block ×7
  (`ui/window/capture.rs:95-105` … `358-367`) becomes a local
  `snapshot(name, window, cx)`; optionally split `run_ui_capture` phases.
- **[ ] A6 — devices.rs row reuse.** Expose `state.rs:14-26`
  `input_rows/output_rows` as `pub(crate)` for `window/devices.rs:57-63`.

### Repository hygiene

- **[x] H1 — `.gitignore`:** add `.vs/` and `.v2c/`; leave `driver/` visible
  per D1.

## Group 2 — Rule single-sourcing

### moiren-signal

- **[x] S1 — Sender-slot bitmap helpers.** The retired-slot snapshot,
  free-slot search, lease construction, and reclaim loop are duplicated
  between `subscribe_latest` (`registry.rs:190-276`) and `subscribe_stream`
  (`298-390`); `latest_storage_mut`/`stream_storage_mut` (`170-188`/`278-296`)
  are type-only twins. Extract shared storage helpers.
- **[x] S2 — `retire_sender` single implementation.** The 4-line clear
  sequence (senders/occupied/fanout/retired_receivers with Acquire-read /
  Relaxed-clear ordering) is hand-written ×4: `registry.rs:253-267`,
  `368-378`, `dispatcher.rs:85-94`, `179-192`. One `retire_sender(&mut self,
  index)` (Stream variant keeps its `finish_dispatch` hook). This is the
  highest-risk-of-silent-race duplication in the workspace.
- **[x] S3 (kept per-site liveness/`counted` checks — the two implementations are not fully equivalent; only the shared eligibility predicate and drop counting were converged) — Stream delivery-eligibility predicate.** Eligibility + drop
  counting implemented twice with different shapes:
  `transport/stream.rs:266-288` (`deliver_to`, contains `lease.counted`) vs
  `dispatcher.rs:222-247` (`step`, uses `lease.upgrade().is_some()`).
  Extract `StreamSender::eligible(meta, demand)` + `record_drop`.
- **[x] S4 (via shared `check_write`/`acquire_slot` — literal delegation impossible: `&mut self` cannot produce the owned guard) — `try_write` thin delegation.** Test-only `try_write`
  (`transport/latest.rs:121-143`, `stream.rs:167-181`) re-implements the
  checks that production `try_write_owned` performs; delegate so tests
  exercise production validation.
- **[x] S5 — `WriteError → PublishOutcome` flatten.** Same 4-arm match ×3
  (`transport/stream.rs:129-138`, `188-197`, `host/scope.rs:97-106`); plus
  `scope.rs:97-109` `publish_slice` body duplicates `stream.rs:188-201`.
- **[x] S6 — `ring_index_bound`.** `(depth + 1) * 2` hand-checked ×3
  (`registry.rs:59-63`, `306-313`, `528-535`) and laid out at
  `budget.rs:154-161`; expose one `pub(crate)` helper in budget.rs.
- **[x] S7 (verified `try_update` returns the pre-update value, exact replacement) — identity allocator.** `lifecycle.rs:119-130`
  `checked_identity` hand-rolls the CAS loop; the crate already has the
  `try_update` form (`host/mod.rs:26-35`) duplicated cross-crate at
  `moiren-engine/src/host/snapshot.rs:21-31`. Use `try_update` in signal.
  Cross-crate `HostId` name collision documented, rename deferred.

### moiren-core

- **[x] C1 — Port-twin consolidation.** `create_node`/`create_rack`
  (`graph.rs:77-117`/`136-192`), `add_input_port`/`add_output_port`
  (`196-232`/`235-268`), `remove_input_port`/`remove_output_port`
  (`271-297`/`300-328`): parameterize by role, keep thin public wrappers so
  the API is unchanged.
- **[x] C2 (single-edge order follows `connect`; `validate` now reports `InvalidSendParameters` for the unreachable double-fault state — not constructible via public API) — `check_edge` shared.** `connection_nodes` (`370-397`) and
  `validate` (`462-519`) maintain the same edge invariants; extract the
  single-edge check, call from both.
- **[x] C3 — protocol frame helpers.** `read_parameter` (`protocol.rs:126-135`)
  vs `read_reply` (`183-218`) duplicate prefix/length/op decoding;
  `REPLY_BODY_BYTES = 40` replaces literals at `171,179,186,189`; opcodes get
  named constants; `read_frame`/`write_frame` helpers.

### moiren-engine

- **[ ] E4 — Per-processor parameter knowledge single source.** Adding a
  parameter currently touches `builtin/x.rs` DOMAINS, `host/preparation.rs:
  159-336` backfill (compressor arm backfills by positional
  `ParameterId(index)` — reorder silently misreads), `host/project.rs
  validate`, `rack.rs`. Give each processor one
  `parameters_from(settings, overrides)` entry point; preparation.rs only
  dispatches. Also fixes the duplicated `parameters().iter().all(|spec|
  spec.accepts(..))` idiom (`rack.rs:295`, `host/project.rs` arms).
- **[ ] E5 — Delay bounds single source + precheck drift fix.**
  `Delay::MAX_DELAY_MS` private at `delay.rs:44` while `rack.rs:279-283` and
  `host/project.rs:231` hard-code 60_000.0; `rack/prepare.rs:59-78`
  re-derives capacity and already drifted (misses `MAX_BUFFER_BYTES`).
  Add `Delay::validate_settings` used by `new` and the rack precheck.
- **[ ] E6 — Channel bound constants.** `recorder.rs:27` private
  `MAX_CHANNELS = 256` duplicates `builtin/mod.rs:31`;
  `host/project.rs` uses `<= 256` literals ×3; route through one constant.
- **[ ] E7 — `EngineConfig::validated`.** Identical validation at
  `compiler.rs:152-158` and `runtime.rs:146-153`; one owner.
- **[ ] E8 — `IoSchema::from_io`.** Same collect+sort+construct at
  `runtime/swap.rs:464-475` and `runtime/swap/reservation.rs:18-33`.
- **[ ] E9 — Observer window continuity.** meter/spectrum duplicate the
  demand gate + generation/epoch/next_frame/sample_rate reset preamble
  (`meter/signal.rs:109-124` vs `spectrum.rs:219-234`) and `validate_inputs`;
  extract `WindowContinuity` (+ shared `TimelineHeader` fields). The older
  queue-based `LevelMeter` (`meter.rs:102-118`) stays; add a doc comment
  marking its boundary.
- **[ ] E10 — Swap compatibility predicate.** Three parallel predicates
  (`runtime/swap.rs:617-622`, `787-806`, `928-949`). Keep the defensive
  control+RT double-check; extract the (config, epoch, revision) comparison
  so the three sites share one source. Split `apply_pending_plan`'s four
  phases into `#[inline]` private fns (no allocation/locking change).
- **[ ] E11 — `host/signals.rs` get-or-register.** Three
  shape-cache→register→prepare copies (`335-384`, `404-438`, `580-600`),
  twin readers (`513-533` vs `535-555`), twin subscribes, and the three
  unregister loops in `create_host` (`294-313`); extract a generic
  `topic_or_register` + `reader_handle`.
- **[ ] E12 — Dead/alias API.** Delete `PlanSnapshot::single_sink`
  (`runtime/swap/reservation.rs:57-65`, zero workspace references); resolve
  `Compressor::OUTPUT_GAIN` alias (keep one name, update the alias test).

### moiren-windows-audio

- **[ ] W3 — WASAPI negotiation kernel.** Two full negotiation stacks:
  `professional/wasapi.rs` (368-449 + fallback 737-742) vs
  `shared_stream/wasapi.rs` (187-268 + failure() 29-59); PCM/FLOAT GUID
  consts duplicated (`professional/wasapi.rs:29-30` vs
  `shared_stream/format.rs:14-15`); descriptor parsing duplicated
  (`professional/wasapi.rs:813-877` vs `shared_stream/format.rs:27-72`);
  `friendly_name` duplicated (`professional/wasapi.rs:32-41` vs
  `catalog.rs:117-130`). Extract a shared kernel (descriptor parse, Client3
  capability query + fallback matrix, legacy-init flags as named policy
  params per D4). PERIODICITY_LOCKED retry per D4.
- **[ ] W4 — `open_shared_client(direction)` builder.** The same COM
  sequence ×3 (`capture/wasapi/physical.rs:26-99`,
  `render/wasapi/stream.rs:110-197`, `professional/wasapi.rs:152-516`
  exclusive branch aside); parameterize direction + error type via a shared
  stage error (also unifies the ×4 `api()` shape: `render/wasapi.rs:40-45`,
  `capture/wasapi.rs:32-40`, `professional/wasapi.rs:51-71`, `catalog.rs:32-40`).
- **[x] W5 — Lease family parametrization.** Three RAII release-buffer leases
  with different fallback flags (`professional/wasapi.rs:669-683`,
  `render/wasapi/packet.rs:6-25`, `owner.rs:60-77`); one type, flags as
  parameter.
- **[x] W6 (rt-check adjusted: local `#[path]` include of codec/resample + standalone thiserror dep; harness suite 68/68 green) — Dead/narrow API.** Delete `professional/backend.rs:44-46
  stream_observations()` (self-described alias, zero callers) and
  `resample.rs:66-68 capacity()`; `push()` is test-only → gate or remove;
  `codec`/`resample`/`native` modules narrow to `pub(crate)` except
  `native::NativeLatency` (app consumer) — verify rt-check's `#[path]`
  shadow-compile still builds.

### moiren-app

- **[ ] A7 — InputSelection ⇄ SourceMode mapping.** 5 copies
  (`ui/state.rs:547-561`, `600-625`, `649-666`, `722-749`, `753-770`);
  extract `apply_source(&mut self, InputSelection)` +
  `current_selection(&self) -> Option<InputSelection>`.
- **[ ] A8 — GraphEdit → parameter mapping.** `service/driver.rs:407-439`
  lacks `SetSend`, `service/managed/parameters.rs:39-85` has it; one pure
  function in `service::graph` (managed version canonical), both drivers use
  it.
- **[ ] A9 — Node-state validation single source.** Same domains three times
  with three message styles: `service/graph/edit.rs:119-206`,
  `service/graph/preparation.rs:237-271`, `project/graph.rs:270-311`; the
  dummy-`SessionSpec` trick is verbatim ×2 (`edit.rs:159-167`,
  `preparation.rs:252-266`). One `validate_node_state` +
  `InputSelection::validate`; project layer keeps field-path context.
- **[ ] A10 — Stale-observation guard.** The `Running && generation && owner
  _generation` predicate ×7 (`managed/driver.rs:174-177`, `193-196`,
  `service/mod.rs:255-280` ×4 filters, `ui/state.rs:293-302`, `385-389`);
  one `fresh()` helper in service, UI keeps one display-level fallback.
- **[ ] A11 — Remove `managed::Backend<S>` newtype** (`managed.rs:21-33`,
  pure forwarding, no second implementation, no added invariant).
- **[ ] A12 — Catalog refresher.** `service/driver.rs:287-294` vs
  `managed/driver.rs:139-150` (+ parallel completion consumers) share the
  catalog-refresh job shape; extract `CatalogRefresher`; the
  automatic-startup retry divergence stays in each poll and is documented.

## Group 3 — Test scaffolding convergence

- **[ ] T1 — engine `tests/common/`.** `CompileConfig` ×4
  (`tests/compiler/main.rs:20-33`, `runtime/signals.rs:1103-1112`,
  `runtime/update.rs:31-44`, `runtime/swap.rs:56-72`), connect helpers ×3+,
  hand-built engines ×3 (`runtime/swap.rs` test mod, `runtime/signals.rs
  source_engine`, `runtime/swap.rs prepare`), counting sources ×2 vs unused
  shared `ConstantSource`, `PumpBudget` literal ×5, deadline-wait ×5.
  Converge: `compile_config`, `connect`, `counting_source`, `test_engine`,
  `pump`, `wait_*`. Keep the two `assert_samples` tolerance policies but
  name them (`assert_samples_exact` / `assert_samples_close`).
- **[ ] T2 — signal test utils.** `pump_until_settled` (5 variants with
  drifted bounds: 1000-panic vs 8-silent) in `tests/common/mod.rs` + a
  `#[cfg(test)]` unit-side helper; the `Resume`/deallocation-gate 18-line
  block ×4 (`tests/registry.rs:205-226`, `428-445`, `488-505`, `753-771`)
  becomes one helper; counting-allocator probes: keep the independent
  cross-check mirrors in `tests/registry.rs` (intentional), converge
  `src/test_alloc.rs` vs `tests/allocation.rs` counting semantics if a
  shared dev-only module is cheap; otherwise document the split.
- **[ ] T3 — windows-audio test commons.** Counting-allocator scaffold ×7
  (`tests/clock_bridge.rs:46-80`, `output_followers.rs:17-44`, `render.rs`,
  `render_diagnostics.rs`, `latency.rs`, `professional/rt_tests.rs:46-81`,
  `render/wasapi/stream.rs:502-556`) → one `alloc_guard` module; clock_*
  packet/push/consume helpers (`clock_age.rs:6-41`, `clock_timing.rs:9-45`,
  `clock_rate.rs:9-42`, `clock_bridge.rs:80-188`) and the three `simulate()`
  kernels (`clock_control.rs:54-152`, `clock_startup.rs:115-213`,
  `clock_faults.rs:110-231`) → `tests/common/` with parameterization.
- **[ ] T4 — app UI harness.** `ServiceRuntime::spawn` ×51,
  `initial_snapshot` ×41, `add_window_view(StudioShell::new)` ×31,
  request_exit/join ×61, per-test 6-line preambles, ~26 deadline loops with
  2s/3s/5s timeouts. Add `#[cfg(feature = "gpui-test")] test_shell(cx)` and
  `wait_until(deadline, poll)` next to the existing service `Rig`; migrate
  call sites mechanically.
- **[ ] T5 — examples arg parsing** (`w00_probe.rs`, `w00_physical.rs`,
  `w00_swdevice.rs` ~40-line twin parsers): shared tiny helper or accept as
  example-local noise — implementer's call, keep it small.

## Group 4 — Structural

- **[ ] E13 — process() dual-arm unification.** Seven processors write the
  algorithm twice inside Separate/InPlace arms (`gate.rs:203-255`,
  `limiter.rs:141-197`, `compressor.rs:230-264`, `delay.rs:172-222`,
  `eq.rs:166-208`, `gain.rs:51-73`, `pan.rs:120-164`); introduce a
  frame-level symmetric access API on the existing buffer abstraction so
  each kernel is written once. Preserve the ordering constraints called out
  in arm comments (e.g. `compressor.rs:252`). No RT semantics change
  (no allocation); existing DSP tests are the gate.
- **[ ] E14 — Long-function phase splits (no file moves).**
  `host/preparation.rs::prepare` (347 lines, five phases),
  `compiler/plan.rs::build` (186 lines; rack-degradation block extracted),
  `professional/wasapi.rs::open_inner` (365 lines → select_format /
  negotiate_exclusive / negotiate_shared_exact),
  `render/wasapi/stream.rs::run_stream` (163 lines; one `record_callback`
  for the stats/diagnostics/telemetry bookkeeping triple),
  `service/managed/driver.rs::poll` (327 lines → named poll phases),
  `ui/window/graph/node_inspector.rs` table-driven builtin controls
  (`510-558`/`564-616`/`622-666` triple + ± button closures `967-997`).
- **[ ] A13 — `ui/state.rs` presentation extraction.** 1488-line module
  mixes six responsibilities; move the pure text projections
  (`native_timing_rows` ~200 lines, `phase_text`, `latency_apply_text`,
  `elapsed_text`, `control_feedback`) into `ui::presentation` taking
  `&AppSnapshot`; state keeps projection + intent. Group `StudioShell`
  fields along existing sub-module boundaries.
- **[ ] A14 — NodeId strong typing in UI.** `Option<String>` /
  `HashMap<String, _>` keys with 17 `format!("{:?}", id)` sites
  (`window.rs:142,162`, `node_inspector.rs`, `mixer.rs`, `graph.rs`,
  `project.rs`, `graph_canvas.rs`); switch to `NodeId` (Copy/Eq/Hash),
  keep one conversion at the project-document boundary.
- **[ ] W7 — rt-check out of `src/`.** `src/professional/rt-check/` shadow-
  compiles four modules via `#[path]` and drops a ~2.2 GB `target/` inside
  src. Move the harness directory out of `src/` (e.g. `tools/rt-check/`),
  keeping the `#[path]` mechanism and CI wiring
  (`.github/workflows/runtime-foundation.yml:33`) intact; verify
  `cargo test` from the new location and that the workspace still excludes
  it correctly.
- **[ ] W8 — Example-only library modules.** `src/probe.rs` (327 ln),
  `src/exclusive_probe.rs` (726 ln), `src/physical/*` (~900 ln),
  `src/swdevice.rs` (269 ln) compile into the library but only examples
  consume them; move under `examples/` module dirs or feature-gate
  (`probe-tools`). Keep the tools working.
- **[ ] W9 — asio_probe test relocation.** `examples/asio_probe.rs` (1053
  ln) carries ~490 lines of embedded tests; move pure-logic tests
  (parse_options, ProgressMonitor) into the library test tree.
- **[!] W-K — Default path → professional + legacy retirement.** Blocked
  per D3: behavior change on default settings, needs hardware validation
  and explicit user go-ahead. Prerequisite: W3 kernel unification so the
  switch is a routing change, not a semantics change.

## Cross-crate notes (recorded, mostly no action)

- `HostId` name collision (`moiren-signal/src/host/binding.rs:10` vs
  `moiren-engine/src/host/snapshot.rs:15`): same name, different types;
  consider renaming the engine side when next touched.
- Engine `BoundIo` seven-method forwarding wrapper, `compile_update`
  convenience forward, `EngineObserver` subscription forwards: kept —
  documented encapsulation reasons.
- windows-audio `src/stats.rs` peak/RMS vs engine meters: different
  semantics (probe diagnostics vs exact windows), intentional.
- 256-channel limits in three crates: ASIO driver-domain vs engine bound —
  coincidental, add cross-reference comments only (E6 covers engine side).
- moiren-core `LogicalGraph::validate_connection` and moiren-signal
  `copy_latest`/`copy_next`: currently test/example-only consumers; add
  "reserved for" doc comments or remove when their intended consumers land.

## Validation

- Baseline: `cargo test --locked --workspace` (recorded pre-change state),
  `cargo clippy --locked --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`.
- After each group: affected-crate tests + clippy + fmt; full workspace suite
  at closeout. UI acceptance: `cargo test --locked -p moiren-app --features
  gpui-test` where app/UI code changed.
