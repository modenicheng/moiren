# Managed Routing and Low Latency Implementation Plan

> For agentic workers: use subagent-driven-development for the independent
> native-backend and managed-service work, then review and verify integration.

**Goal:** Run the desktop on EngineHost with multiple IO and validated latency choices.

**Architecture:** Preserve LogicalGraph as routing authority. EngineHost owns
compilation, swaps, resources and retirement. Native follower outputs adapt to
the master clock using bounded queues. Service/UI project confirmed state and
native timing rather than duplicating lifecycle logic.

**Tech Stack:** Rust 2024, existing WASAPI owners, EngineHost, moiren-signal, gpui-kit.

## Global Constraints

- No allocation, destruction, waiting or COM work on the audio thread.
- Preserve existing uncommitted Exclusive probe files and README edits.
- Do not auto-commit, push or change branches.
- Native IO remains Shared stereo, output/processing 48 kHz; reject unsupported formats.
- Physical latency, CPU and XRUN remain unknown unless measured by their real publishers.
- Tests must exercise behavior, cancellation and stale identities, not implementation shape.

## Task 1: Managed application service

Files: add service/managed.rs and responsibility-sized submodules; change
service/mod.rs, service/graph/preparation.rs and focused service tests.

Interfaces: consume EngineHost/EngineBackend, EngineProject and current
GraphPreparation. Produce ManagedDriver configurable with a backend factory,
and Windows production startup using the engine-backed WindowsBackend.
Expose latency selection as AppRequest::SetLatencyProfile(AudioLatencyProfile),
with service::AudioLatencyProfile::{Compatible,Balanced,Low}, and publish the
selected value on AppSnapshot.latency_profile. The shared portable profile type
is implemented by the root coordinator in service/latency.rs.

- [x] Add controlled backend regressions for compatible topology swap retaining
  owner/epoch and rejected compilation retaining active output.
- [x] Translate every Source/Sink binding, Gain/Pan/Compressor/Bus and node/send
  desired parameters to a validated EngineProject.
- [x] Implement asynchronous host ownership, bounded receipts, cancellation,
  shutdown, candidate/active projections and stable parameter identities.
- [x] Keep existing public legacy drivers functional while production startup
  adopts ManagedDriver; bridge existing meter subscriptions to host signals.
- [x] Remove the production single Source/Sink restriction; validate all bound IO.
- [x] Test multiple IO translation, stale project identity, parameter ACKs,
  resource failures, pending edits, Stop/Exit and host cleanup off UI.
- [x] Run focused app service tests and report exact commands/results.

## Task 2: Native multiple outputs and latency observations

Files: engine_backend modules, render and runtime sink-guard modules as needed;
new focused multi-output tests and portable output bridge modules.

Interfaces: consume EngineProject IO nodes and WindowsBackendConfig; keep
WindowsBackend::new(config) and EngineBackend contracts compatible. Produce
safe multiple native output ownership and WindowsBackend observations usable by
the managed service. Communicate any new API to the coordinator/service agent.

- [x] Test master/follower selection, per-output routing, invalid/duplicate IO,
  clock drift, discontinuities, bounded memory and zero RT allocation.
- [x] Prepare one native master and followers before the shared activation gate.
  Feed follower sinks through preallocated adapted output sources; never run
  the main engine on more than one thread.
- [x] Protect all active sink bindings during compatible swaps; transport identity
  changes replace all affected native owners and advance epoch.
- [x] Join every owner on failure/cancellation, preserve diagnostic reports and
  require every output/input to acknowledge Start before Running.
- [x] Publish coherent immutable prepared timing plus bounded live observations;
  retain existing Compatible behavior and actual-period admission validation.
- [x] Run engine/render/native-adapter portable tests, allocation regressions,
  all-target check and strict crate Clippy; report hardware separately.

## Task 3: Low latency settings and persistence

Files: service/latency.rs, project/model.rs, UI settings/state/window and tests;
integration with the managed driver and WindowsBackendConfig.

- [x] Add serializable Compatible/Balanced/Low profile type with descriptive
  labels, explicit period and queue policies, safe Compatible default.
- [x] Add backward-compatible preference persistence and regression tests.
- [x] Wire three settings actions to service requests and saved preferences;
  clearly indicate pending apply and show selected vs actual native timing.
- [x] Ensure impossible queue/period budgets fail visibly and observations reset
  on owner replacement/stop. Add UI headless and service/profile regressions.
- [x] Run targeted tests then gpui acceptance; update outdated restart/single-IO copy.

## Task 4: Integration, review and validation

- [x] Review task diffs for cancellation, ownership, RT safety, graph/project
  identity and latency claims; fix all important findings.
- [x] Run cargo check --locked --workspace --all-targets, cargo test --locked
  --workspace, cargo test --locked -p moiren-app --features gpui-test,
  cargo fmt --all -- --check and strict workspace Clippy.
- [x] Exercise available muted native endpoints with finite opt-in tests; record
  unsupported hardware and distinguish API results from physical latency.
- [x] Update current README/design documentation and record evidence, limitations
  and the preserved Exclusive experiment state.
