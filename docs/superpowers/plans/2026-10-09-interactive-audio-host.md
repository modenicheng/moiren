# Interactive Audio Host Implementation Plan

> **For agentic workers:** Use subagent-driven-development to implement and review each responsibility-sized task. Continue until all six requirements have current evidence.

**Goal:** Deliver the complete non-UI M0.6 multi-source Audio Host milestone.

**Architecture:** Rust app-domain controller prepares graphs and plans, independent capture owners feed a Bus, and one continuous render master applies swaps. All compilation, device discovery and resource reclamation happen outside RT.

**Tech Stack:** Rust 2024, existing moiren-core/engine, WASAPI Shared, existing SPSC queues and clock bridge.

## Global Constraints

- Preserve the frontend work in the original checkout; all workers use `D:\coding\moiren\.worktrees\audio-host`.
- No allocation or resource destruction during engine render or plan swap. Return the engine to control after stopping.
- Keep the existing bounded monitor/render APIs compatible; add explicit continuous operation.
- Source stop signals are independent; one failed source never stops other sources or render.
- Reuse the output bridge identity and retained input backend identities across plans.
- Publish only prepared plans; acknowledge desired/active revisions separately and drain terminal parameter replies before retirement destruction.
- UI depends on moiren-app domain APIs, never engine or WASAPI internal ownership types.
- Modify only explicitly assigned files. Commit explicit paths. Use `CARGO_TARGET_DIR=D:\coding\moiren\.worktrees\audio-host\target` for native checks so frontend builds remain independent.

### Task 1: Continuous backend ownership

**Files:** `crates/moiren-windows-audio/src/{render,capture,process_loopback}.rs`, `src/render/wasapi{,/stream}.rs`, backend lifecycle tests.
**Interfaces:** Keep `RenderOptions`/`CaptureOptions` and current start/join calls. Add explicit continuous option constructors using a documented `Duration::MAX` sentinel, accepted by validation, while bounded calls retain 1..600 seconds. Add `RenderSession::join_with_renderer()` returning `(RenderReport, DemandRenderer)`. Existing `join()` and Drop must join and destroy returned render resources on their caller. Add a cloneable `RenderObserver` with atomic timeline/counters and attach it to `DemandRenderer` before start.

- [x] Test bounded validation and continuous validation for render, capture and process.
- [x] Return renderer from worker on normal stop and failure, with COM cleanup complete first; expose a control-side engine extraction method.
- [x] Test observable timeline and normal render behavior with the existing renderer tests.
- [x] Run `cargo test --locked -p moiren-windows-audio` and strict crate Clippy; commit explicit paths and review.

### Task 2: Portable AudioHost control and plan publication

**Files:** Create `crates/moiren-app/src/host.rs` and responsibility modules under `src/host/`, `tests/host.rs`; export in `src/lib.rs`.
**Interfaces:** `AudioHost::prepare(HostConfig) -> (AudioHost, HostRenderer)` with a portable engine/output handoff. Stable `SourceId` maps to source/gain/pan nodes. `add_source`, `remove_source`, `set_gain`, `set_pan`, `set_compressor`, `publish`, `poll`, `graph_snapshot`, `runtime_snapshot` operate on the control owner. Domain DTOs include request/revision identity, validation errors and terminal receipts; use no GUI dependency. Renderer can run deterministically offline and be handed to Windows integration.

- [x] Implement and test empty Bus output, two-source mixing, individual gain/pan and retained source/bridge identity after recompilation.
- [x] Compile all candidates outside RT, map reuse using stable NodeId/EdgeId, keep old ControlPorts until retired pending replies are drained.
- [x] Test repeated compressor insertion/removal, outstanding parameter requests, invalid edits and pending plan backpressure.
- [x] Provide graph commands for the existing DAG primitives and structured graph/runtime/plan snapshots.
- [x] Verify no RT allocation/deallocation during multi-source render and plan swaps using the existing allocation test approach.
- [x] Run `cargo test --locked -p moiren-app`; commit explicit files and review.

### Task 3: Windows host sessions and headless CLI

**Files:** `crates/moiren-app/src/host/windows.rs`, `src/host_cli.rs`, `src/main.rs`, `tests/host_cli.rs`, `README.md`.
**Interfaces:** App-domain capture selections identify physical endpoints or process PID plus creation time. `start/stop`, source enable/disable/replace, device/process lists and diagnostics work through AudioHost. CLI `host` accepts commands over stdin and emits JSON status/receipts; default lifetime is until `stop`/EOF, with an optional bounded test duration.

- [x] Start continuous capture sessions independently and continuous WASAPI render; prepare graph before handoff.
- [x] Poll finished capture workers, mark unavailable/failed/stopped, join only that source, leave other sources/render running.
- [x] Support adding and replacing sources during render without restarting the output session; clean up all partial startup failures.
- [x] Return renderer to control and drain pending/retired ownership during stop; surface output failure as host failure.
- [x] Add CLI parsing/command regression tests; document the UI control contract and reproduction commands; review.

### Task 4: Cold-start and stability / Miri investigation

**Files:** Existing clock bridge source/tests, `.github/workflows/runtime-foundation.yml`, privacy-safe evidence under `docs/validation/`, stress helper if needed.

- [x] Read failed workflow logs and identify the concrete Miri cause; fix the cause and retain meaningful aliasing coverage.
- [x] Reproduce cold-start reserve exhaustion and prevent partial underflow during startup priming; test intermittent packets and reconfiguration.
- [x] Run deterministic drift/long-run regression, stop/reconfiguration pressure and live owned-generator/microphone mixing.
- [x] Record duration, sample rates, shortfall/reprime/overflow, revisions, source outcomes and original/remaining limitations without storing PCM or private IDs.
- [x] Record performed long device runs and source exit/restart. Record device loss, sleep and hotplug as unverified P1 scenarios; no claim that those scenarios passed.

### Task 5: Controlled Takeover feasibility

**Files:** Experiment CLI/helper in `moiren-windows-audio`, reusable session inspection module only if justified, `docs/validation/` results.

- [x] Use an owned audio generator for session mute/volume changes; record prior values and restore/read back on every exit.
- [x] Measure original endpoint playback alongside process capture, then capture after mute/volume changes and with a concurrent capture observer.
- [x] Exercise target restart and explain OBS-specific evidence or remaining required external verification.
- [x] Commit reproduction commands, aggregate measurements and a supported feasibility conclusion; no silent changes to unrelated applications.

### Task 6: Complete integration audit

**Files:** Evidence, app/backend README, implementation ledger.

- [x] Run locked workspace/all-target checks and tests, strict Clippy, fmt, diff check and relevant examples.
- [x] Obtain independent review of the complete change and resolve material findings.
- [x] Map each original six-item requirement and Takeover investigation to authoritative tests/hardware evidence; leave unverified requirements open.
- [x] Deliver backend commits and UI-facing API documentation; preserve frontend checkout and report exact branch/worktree state.

## Final delivery audit

All six original M0.6 requirements and the controlled Takeover investigation passed independent code and evidence review at ce9aae0. Runtime code is identical to hardware-tested e1568ec; merge 599e108 integrates main 21cba63 while retaining native exact Compressor assertions and the finite Miri comparison.

Current evidence: six fresh 30-second cold scenarios plus 620.043 seconds of actual device operation; every observed Source generation has zero live/finished underrun, seven applied revisions per run, five normal Applied/all six terminal parameter receipts, zero output bridge shortfall/empty padding and clean Stop. Four two-hour drift traces are simulations. Full Windows/Linux checks and both Miri modes passed; final integration's native and Miri covering checks passed again. See [host validation](../../validation/2026-10-09-audio-host.md) and [Takeover validation](../../validation/2026-10-09-takeover.md).

Historical live 128-frame starvation remains recorded without an identified Windows cause. The reproducible native pre-Start reserve drain and delayed retirement marker are separately fixed. OBS, hotplug, sleep/resume and automatic recovery remain unverified or outside this six-item milestone. The nonblocking enable-guard test isolation suggestion remains recorded in the independent review.

Delivery keeps feat/interactive-audio-host and its worktree; the original main checkout and frontend changes are preserved.
