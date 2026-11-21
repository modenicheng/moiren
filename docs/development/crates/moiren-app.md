# moiren-app

Crate guide. Repository top rules: [AGENTS.md](../../../AGENTS.md); development index: [../README.md](../README.md). Paths below are relative to the crate unless linked. For UI work, [FRONTEND.md](../../../FRONTEND.md) and [app-ui.md](../app-ui.md) are required reading.

## Role

The desktop application: application service (candidate graph, `EngineHost` driver, jobs, snapshots, persistence), gpui-kit UI, CLI entry (`main.rs`), i18n, and the offline demo. The documented flow is UI dispatches `AppRequest` → service validates/commits → engine host applies asynchronously → snapshots flow back. The UI never owns audio state or edits the graph directly.

## Layout

- `src/service/`: service runtime, control endpoint and native session driver, candidate/active graph, edits, parameters, preparation, exports, plugin service, managed driver/lifecycle, snapshots. Production code here does not import `crate::ui`; tests use `crate::ui::finish_service`, the only service→UI seam, to shut a runtime down.
- `src/project/`: the persisted document (`model.rs`, `graph.rs`, `exports.rs`, `storage.rs`, `startup.rs`, `window.rs`, `meter.rs`), versioned loading/saving, and autosave storage. Keep persistence versioning and unknown/new-node handling explicit.
- `src/ui/`: `shell/` (window, tray, autosave, capture harness), `pages/{graph,mixer,matrix,devices,settings}/`, `widgets/`, `state/`, `theme/`, `monitoring/`. Pages are plain render functions (for example `page`, `mount`, `inspector`) that take `&StudioShell` plus the gpui window/context and return `AnyElement`, composing `div()` trees and `widgets/` helpers. Reusable interactive elements live in `widgets/` as `RenderOnce` + `#[derive(IntoElement)]` components. Colors, spacing, radii, and type go through `src/ui/theme/` tokens; icons are Tabler only.
- `src/i18n.rs`, `src/i18n_guards.rs`, `locales/zh-CN.yml`: localization runtime and guard tests.
- `src/console.rs`, `src/logging.rs`, `src/system.rs`, `src/timeline.rs`: console/IPC, logging, OS helpers, and the test-only epoch helper.
- `tests/`: `service*.rs`, `ui_acceptance.rs`, `ui_lifecycle.rs`, `i18n_ledger.rs`, `automatic_startup.rs`, `offline.rs`; `examples/` has `ui_capture`, `vad_integration`, and `component_preview`.

## Contracts

- The candidate graph is the single routing truth; runtime facts come from engine observations. Show requested → accepted → applied truthfully, use Unknown/`—` instead of fake zeros, and never relabel candidate config as active.
- All user-facing strings go through `t!`; the key set in `locales/zh-CN.yml` is authoritative, and `tests/i18n_ledger.rs` plus `src/i18n_guards.rs` fail on drift. Keep `%{placeholder}` sets identical; audio abbreviations stay English. Service-side fault causes are catalog keys too: closed error enums map variant-by-variant in `src/service/fault_causes.rs` (payloads stay in the service log), and the same guards list those keys.
- UI must not allocate, reclaim, or block on audio threads; graph/buffer/signal binding and retired-plan reclamation stay on the control side. Subscribe only to visible nodes and release subscriptions when hidden or off-screen.
- Structural edits debounce and rebuild atomically; a failed rebuild keeps the old graph running. Parameter edits should use the realtime parameter channel where possible.
- Tests that construct the shell directly must clear `shell.autosave.path` and `shell.preferences.path` (the shared harness does), otherwise a dirty test graph can overwrite the developer's real `%APPDATA%\Moiren` autosave (audit AS-2/AS-3). Prefer `test_shell`/`shell_with_state`, and never open a real audio device in tests.
- UI composition: page-level rendering stays free functions in `pages/` following the existing `page`/`mount`/`inspector` shape (read `shell.state`, compose `div()` trees, call `widgets/` helpers); only `widgets/` elements implement `RenderOnce`, so do not introduce page-level components. Add a `RenderOnce`/`#[derive(IntoElement)]` widget only when the element has its own props and is reused across pages, and keep its styling on theme tokens.
- Windows-only UI pieces (tray, window chrome, `gpui-test` headless acceptance) are gated; the Ubuntu CI job runs the portable app tests plus the device-free VAD integration example, while Windows adds the headless UI acceptance.

## Verification

```sh
cargo test --locked -p moiren-app
cargo test --locked -p moiren-app --features gpui-test          # headless UI acceptance (Windows)
cargo clippy --locked -p moiren-app --all-targets --features ui-profiler -- -D warnings
cargo run --locked -p moiren-app --features gpui-test --example ui_capture
cargo run --locked -p moiren-app --features component-gallery --example component_preview
```

`ui_capture` renders the real shell at 1440×900 and 1024×720 into `docs/designs/screenshots/application/`; attach UI screenshots to PRs. `--features vad-integration-probe` with `--example vad_integration` exercises the VAD service integration. See [verification.md](../verification.md) for the full gate.

## Docs

- [FRONTEND.md](../../../FRONTEND.md) (UI rules), [crate README](../../../crates/moiren-app/README.md) (behavior, windows/tray, offline app), [TODO](../../TODO.md) (current queue).
- Design records: `docs/designs/07-app-ui-design-system.md`, `11-unified-topology-views-evaluation.md`, `13-application-frontend-integration.md`, `17-two-tier-ui-components.md`, `18-node-cards-and-wiring.md`, `20-ui-restructure.md`, `21-ui-conventions.md`, `22-fader-feel-async-interaction.md`, `23-devices-page-role.md`.
- Engineering rules: [app-ui.md](../app-ui.md), [architecture.md](../architecture.md).
