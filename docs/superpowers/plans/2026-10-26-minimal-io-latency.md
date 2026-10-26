# Minimal I/O + Bus Latency Implementation Plan

> **For agentic workers:** Use `executing-plans` for the overall sequence and
> `subagent-driven-development` for bounded independent tasks. Steps use
> checkboxes and the validation report records evidence, including failures.

**Goal:** Implement and qualify stages 0 through 7: stable Input -> Bus ->
Output with measured p95 <= 10 ms where the actual Windows/device path
supports it, with honest end-to-end evidence for VAD and Process Loopback.

**Architecture:** Extend existing platform latency budgets, bridge preparation
and diagnostics. Retain a single graph owner, bounded transports, epoch
fencing and off-RT preparation/reclamation. Resolve virtual delivery timing
separately from negotiated physical periods.

**Tech Stack:** Rust 2024 stable, WASAPI/IAudioClient3, ACX/KMDF, bounded
SPSC/FIR/servo, gpui-kit, PowerShell, target-machine PCM probes.

## Global Constraints

- Source base: ec66e85117fbf668c2def334bdb33028afd454e9; date 2026-10-26.
- Only modify `D:\coding\moiren\.worktrees\minimal-io-latency`.
- Reference graph Input -> Bus -> Output, 48 kHz, no other processors.
- Steady-state p95 <= 10 ms; report p50/p95/p99/max; explore p50 <= 8 ms.
- Require 30-minute continuous PCM with controlled load; two-hour runs and
  repetition on the target machine qualify a product default.
- No unexpected silent packet, drop, underrun, discontinuity or epoch reset
  in a passing continuous window.
- Capacity, normal target, measured queue age and physical E2E stay distinct.
- No RT allocation/deallocation, blocking, COM preparation or JSON output.
- Physical timing, empirical virtual delivery and unknown timing stay distinct.
- Never relax microphone freshness, waveform checks or existing epoch fencing.
- Preserve primary worktree user changes and never stage unrelated paths.
- Plan approval and implementation authorization were supplied by the user.

## Stage 0 / Task 0: Archive scope and establish source baseline

**Files:** this plan; matching design under `docs/superpowers/specs/`;
`docs/validation/2026-10-26-minimal-io-latency.md`.

- [x] Create independent `feat/minimal-io-latency` worktree at the source base.
- [x] Archive design and this complete sequence before production changes.
- [x] Record baseline build/tests and present-device inventory.
- [x] Commit only design, plan and initial evidence.

Commands:

```powershell
cargo test --locked -p moiren-engine -p moiren-windows-audio
cargo check --locked --workspace --all-targets
git add docs/superpowers/specs/2026-10-26-minimal-io-latency-design.md docs/superpowers/plans/2026-10-26-minimal-io-latency.md docs/validation/2026-10-26-minimal-io-latency.md
git commit -m "docs: archive minimal IO latency implementation and qualification plan"
```

## Stage 1 / Task 1: Reproducible latency marker and measurement foundation

**Files:** create `crates/moiren-windows-audio/src/latency/marker.rs`,
`crates/moiren-windows-audio/tests/latency_marker.rs`; export from
`src/latency.rs`. Later probe integration uses the same detector.

**Interfaces:** a deterministic marker generator, a correlation result with
delay frames/confidence and an explicit failure for silence/ambiguity; all
analysis off RT and caller-limited buffers. Sample-to-time conversion accepts
an explicit rate. No measurement labels itself physical E2E.

- [x] Write failing tests: known 0/64/128/480/4096-frame delays, attenuated and
  inverted polarity, bounded noise, silence, truncated marker, periodic
  ambiguity and invalid rates/limits. Show red then green.
- [x] Implement deterministic broadband generation and normalized detection;
  prevent accepting missing/ambiguous signal as zero latency.
- [x] Verify bounded sizing and no RT-facing allocation; document that marker
  and recording allocation occur before streaming.
- [x] Run the focused integration test and strict crate Clippy.
- [x] Review and commit the marker/budget slice as `3b0d3f7`.

```powershell
cargo test --locked -p moiren-windows-audio --test latency_marker
cargo clippy --locked -p moiren-windows-audio --all-targets -- -D warnings
```

## Stage 2 / Task 2: Explicit virtual-input budget

**Files:** `src/latency/budget.rs`, new
`src/latency/transport.rs`, `tests/latency_budget.rs`, new
`tests/transport_budget.rs`.

**Interfaces:** explicit delivery bound/provenance, capacity, prime, servo,
low/high water, lookahead and age; reuse `PreparedInputQueue`. Policies use
native input frames and checked rate conversion. Empirical timing is never
stored in `NativeStreamTiming.period_frames` as negotiated hardware timing.

- [x] Add failing tests for target/capacity separation, packet/demand phase,
  invalid target, unknown timing, overflow and 44.1/48/96 kHz conversion.
- [x] Resolve conservative input requirements from demand + delivery gap +
  jitter + lookahead. Return a typed refusal if a requested target cannot fit.
- [x] Keep current Compatible behavior; express lower expert targets through
  validated preparation, without globally replacing defaults.
- [x] Run `latency_budget`, `transport_budget`, `clock_control`, `clock_age`
  and `clock_bridge` tests; review and commit a focused change.

```powershell
cargo test --locked -p moiren-windows-audio --test latency_budget --test transport_budget --test clock_control --test clock_age --test clock_bridge
```

## Stage 3 / Task 3: Process Loopback policy propagation and baseline probe

**Files:** `src/process_loopback.rs`, `src/process_loopback/stream.rs`,
`src/capture/wasapi/{stream,preparation}.rs`,
`src/engine_backend/{preparation,profile}.rs`,
`src/professional/backend.rs`, `src/professional/backend/process*.rs` if split,
`crates/moiren-app/src/monitor/windows.rs`, example/probe callers and tests.

**Interfaces:** `ProcessLoopbackOptions` carries the requested bridge policy.
Prepared capture resolves the budget before the activation gate and exposes
the exact effective input queue. Physical render policy remains independent.

- [x] Add regression tests proving changed process targets reach real bridge
  preparation (not only displayed metadata) and defaults retain compatibility.
- [x] Propagate policy to every production and probe call site; retain target
  process identity/liveness and event/Windows conversion flags.
- [x] Add explicit empirical delivery assumptions and telemetry; no
  IAudioClient3 hardware-period claim for the process virtual client.
- [ ] Probe actual packet sizes/gaps and supported initialization on the
  target. Evaluate API changes against official Microsoft documentation before
  experimenting; do not blindly reuse physical-endpoint flags.
- [ ] Test candidate targets 2048/1024/512/256 against measured delivery,
  all packet phases, drift, burst and output demand. Reject unsafe candidates.
- [x] Run process/capture/render/bridge/probe tests; commit real propagation.

```powershell
cargo test --locked -p moiren-windows-audio --test process_loopback --test capture --test render --test clock_bridge
cargo test --locked -p moiren-app --lib monitor
```

## Stage 4 / Task 4: Explicit VAD carrier and native-packet budgets

**Files:** `src/vad/provider.rs`, `src/vad/provider/{transport,pcm_worker}.rs`,
`src/virtual_owner/{provider,boundary,owner}.rs`,
`src/professional/backend/virtual_io.rs`,
`src/engine_backend/preparation.rs`, boundary/packet-phase/worker tests;
driver `src/audio/` only if small-packet capability work requires it.

**Interfaces:** pass prepared target and capacity independently through
`VirtualBoundaryRequest` and prepared boundaries. Native packet shape may
arrive after idle endpoint preparation; validate on admission and shape
changes without RT allocation or assuming MAX_PACKET_FRAMES is actual size.

- [x] Reproduce current capacity-derived playback/microphone residency in a
  deterministic test; replace it with explicit targets and retain bounded FIR.
- [x] Sweep 64/128/240/256/480 native packet frames against 64/128/256 graph
  blocks, phase, drift, late completions and demand deficits.
- [x] Make recovery elapsed-time aware: reducing block size must not turn
  ordinary jitter into repeated destructive reset/re-prime.
- [x] Preserve gate/epoch/quota/cancellation and stale microphone rejection.
- [ ] Validate client negotiation and driver-advertised minimum periods;
  implement any required ACX period changes with host and target regressions.
- [x] Run event and timer matrices, driver analyzed build/INF, portable VAD,
  owner, professional and RT allocation tests. Commit tested slices.

```powershell
cargo test --locked -p moiren-windows-audio --lib
cargo test --locked -p moiren-windows-audio --test virtual_boundary
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
pwsh -NoProfile -File driver/vad/test.ps1
pwsh -NoProfile -File driver/vad/build.ps1 -Analyze
```

## Stage 5 / Task 5: Minimal graph and shared physical-output path

**Files:** new `tests/minimal_latency_graph.rs` in Engine; measurement probe
graph preparation; render/professional owner code only where measured demand
shows additional buffering.

- [x] Add a real compiled Input -> Bus -> Output impulse/marker regression
  for 64/128/256 and variable device demand. Verify same-call sample transfer.
- [x] Verify exactly one execution owner and no added graph-block batching.
- [x] Check copy/scan callback cost against actual output deadline; optimize
  only measured bottlenecks while preserving allocation/deallocation guards.
- [ ] Use actual output period and independent queue target for Shared;
  compare available Exclusive/ASIO physical outputs without rewriting them.
- [x] Run compiler/runtime/boundary/RT checks and commit regressions/fixes.

```powershell
cargo test --locked -p moiren-engine
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
```

## Stage 6 / Task 6: Configuration, telemetry and safe rename

**Files:** app service latency/audio-settings, project/preferences serialization,
UI settings/presentation, `service/exports/{reconcile,runtime}.rs`, associated
tests and localized strings. Preserve established gpui-kit patterns.

- [x] Expose requested/effective profile and validated expert target separately
  from capacity and actual period; reconfigure via owner replacement.
- [ ] Show queue fill/age, render padding, recovery/drop/silence counters and
  refusal/fallback reason. Unknown E2E remains unknown.
- [x] Add tests for persisted settings, stale observations, invalid budgets
  and pending reconfiguration; add gpui headless acceptance.
- [x] Verify rename retains stable ID/binding/project intent, drains/closes
  before retirement and recreates with actual new metadata.
- [x] Add held-client removal, create failure and repeated rename regressions;
  retain desired name on failure and report actual/pending state.
- [x] Run app tests and gpui acceptance; commit with screenshots if UI changes.

```powershell
cargo test --locked -p moiren-app
cargo test --locked -p moiren-app --features gpui-test
```

## Stage 7 / Task 7: E2E harness, controlled load and final qualification

**Files:** `crates/moiren-windows-audio/examples/e2e_latency_probe.rs`,
supporting probe module, `scripts/latency-qualification.ps1`,
`scripts/tests/latency-qualification.tests.ps1`,
`docs/validation/2026-10-26-minimal-io-latency.md`,
selected evidence in `docs/designs/evidence/`.

**Interfaces:** a versioned run report records provenance, measurement method,
calibration, durations, latency distribution, quality counters, configuration
and result. Missing external calibration is an explicit unqualified result.

- [x] Add failing report/acceptance tests for missing markers, short duration,
  uncalibrated/internal-only evidence, sample gaps, hidden recovery, mismatched
  endpoints/format, stale hashes and percentile failures.
- [x] Implement the probe with explicit source/output/measurement boundary;
  capture marker-bearing PCM with bounded preallocated buffers and analyze
  after stream stop/join. Label internal measurements honestly.
- [ ] Run same-condition VAD/Process Loopback idle and controlled CPU/load
  baselines, then ordinary app startup, owner restart and device-change cases.
- [ ] Run 30-minute PCM for each viable low-latency configuration, no waveform
  or counter failures, p95 <= 10 ms measured at calibrated physical output.
- [ ] Run two-hour repetitions for proposed defaults; retain failed windows,
  diagnostics and original waveform thresholds.
- [x] Run workspace build/check/test/fmt/strict Clippy, gpui-test, RT guard,
  driver analyzed build and portable CI/Miri where available.
- [x] Review the whole branch and resolve significant findings.
- [ ] Audit every stage against current files and recorded actual evidence;
  mark the goal complete only when all required gates are proven.

```powershell
cargo build --locked --workspace
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
git diff --check
```

## Execution ledger

- 2026-10-26: isolated worktree created; full plan archived before production
  implementation. Baseline and device inventory completed.
- 2026-10-26: marker analysis, transport budgets, Process Loopback policy
  propagation, VAD explicit target admission, minimal compiled graph transfer,
  app transport settings, safe rename state machine and offline qualification
  evaluator completed in software. Focused and full Rust/app/driver host checks
  passed; the branch still has uncommitted implementation changes.
- 2026-10-26: hardware gates remain open. The current machine has no present
  Moiren VAD endpoint, the target VM is unavailable, and no calibrated physical
  measurement chain has run. No VAD-vs-Process latency winner or `<10 ms`
  qualification claim is allowed.
