# moiren-app

Crate guide. Read [../../AGENTS.md](../../AGENTS.md) first; this file adds local detail and never overrides the root rules. For any UI work, [FRONTEND.md](../../FRONTEND.md) is required reading.

## Role

The desktop application: application service (candidate graph, `EngineHost` driver, jobs, snapshots, persistence), gpui-kit UI, CLI entry (`main.rs`), i18n, and the offline demo. The documented flow is UI dispatches `AppRequest` → service validates/commits → engine host applies asynchronously → snapshots flow back. The UI never owns audio state or edits the graph directly.

## Layout

- `src/service/`: service runtime, control endpoint and native session driver, candidate/active graph, edits, parameters, preparation, exports, plugin service, managed driver/lifecycle, snapshots. Production code here does not import `crate::ui`; tests use `crate::ui::finish_service`, the only service→UI seam, to shut a runtime down.
- `src/project/`: the persisted document (`model.rs`, `graph.rs`, `exports.rs`, `storage.rs`, `startup.rs`, `window.rs`, `meter.rs`), versioned loading/saving, and autosave storage. Keep persistence versioning and unknown/new-node handling explicit.
- `src/ui/`: `shell/` (window, tray, autosave, capture harness), `pages/{graph,mixer,matrix,devices,settings}/`, `widgets/`, `state/`, `theme/`, `monitoring/`. Use `RenderOnce` components, `#[derive(IntoElement)]`, theme tokens, and Tabler icons only.
- `src/i18n.rs`, `src/i18n_guards.rs`, `locales/zh-CN.yml`: localization runtime and guard tests.
- `src/console.rs`, `src/logging.rs`, `src/system.rs`, `src/timeline.rs`: console/IPC, logging, OS helpers, and the test-only epoch helper.
- `tests/`: `service*.rs`, `ui_acceptance.rs`, `ui_lifecycle.rs`, `i18n_ledger.rs`, `automatic_startup.rs`, `offline.rs`; `examples/` has `ui_capture`, `vad_integration`, and `component_preview`.

## Contracts

- The candidate graph is the single routing truth; runtime facts come from engine observations. Show requested → accepted → applied truthfully, use Unknown/`—` instead of fake zeros, and never relabel candidate config as active.
- All user-facing strings go through `t!`; the key set in `locales/zh-CN.yml` is authoritative, and `tests/i18n_ledger.rs` plus `src/i18n_guards.rs` fail on drift. Keep `%{placeholder}` sets identical.
- UI must not allocate, reclaim, or block on audio threads; graph/buffer/signal binding and retired-plan reclamation stay on the control side. Subscribe only to visible nodes and release subscriptions when hidden or off-screen.
- Structural edits debounce and rebuild atomically; a failed rebuild keeps the old graph running. Parameter edits should use the realtime parameter channel where possible.
- Tests that construct the shell directly must clear `shell.autosave.path` and `shell.preferences.path` (the shared harness does), otherwise a dirty test graph can overwrite the developer's real `%APPDATA%\Moiren` autosave (audit AS-2/AS-3). Prefer `test_shell`/`shell_with_state`, and never open a real audio device in tests.
- Windows-only UI pieces (tray, window chrome, `gpui-test` headless acceptance) are gated; the Ubuntu CI job runs the portable app tests plus the device-free VAD integration example, while Windows adds the headless UI acceptance.

## Verification

```sh
cargo test --locked -p moiren-app
cargo test --locked -p moiren-app --features gpui-test          # headless UI acceptance (Windows)
cargo clippy --locked -p moiren-app --all-targets --features ui-profiler -- -D warnings
cargo run --locked -p moiren-app --features gpui-test --example ui_capture
cargo run --locked -p moiren-app --features component-gallery --example component_preview
```

`ui_capture` renders the real shell at 1440×900 and 1024×720 into `docs/designs/screenshots/application/`; attach UI screenshots to PRs. `--features vad-integration-probe` with `--example vad_integration` exercises the VAD service integration.

## Docs

- [FRONTEND.md](../../FRONTEND.md) (UI rules), [README.md](README.md) (behavior, windows/tray, offline app), [docs/TODO.md](../../docs/TODO.md) (current queue).
- `docs/designs/07-app-ui-design-system.md`, `11-unified-topology-views-evaluation.md`, `13-application-frontend-integration.md`, `17-two-tier-ui-components.md`, `18-node-cards-and-wiring.md`, `20-ui-restructure.md`, `21-ui-conventions.md`, `22-fader-feel-async-interaction.md`, `23-devices-page-role.md`.
