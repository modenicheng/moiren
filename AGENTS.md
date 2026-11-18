# Repository Guidelines

Moiren is a Rust 2024 workspace for Windows real-time audio. Read this file first; each crate also has a short `AGENTS.md` with its module map and crate-local rules. Crate guides refine this file and never override it.

## Project Structure & Module Organization

- `crates/moiren-core`: editable `LogicalGraph` topology and control protocol.
- `crates/moiren-engine`: DSP, graph compilation, buffers, and runtime execution.
- `crates/moiren-signal`: typed, bounded Latest/Stream signals.
- `crates/moiren-plugins`: VST2/VST3/CLAP plugin hosting (ABI declarations, dispatch wrappers, and host orchestration).
- `crates/moiren-windows-audio`: the professional audio backend (WASAPI Shared/Exclusive and ASIO), engine-backed capture/render, VAD support, clock adaptation, and probes.
- `crates/moiren-app`: application service, CLI, and gpui desktop UI.

Each crate uses `src/` for sources, `tests/` for integration tests, and optional `examples/` for demonstrations. The C++ VAD driver host lives in `driver/`, the rt-check harness and audio-hack-lab in `tools/`, and support scripts in `scripts/`. The desktop UI is gpui-kit (Rust, no markup language); UI code lives in `crates/moiren-app/src/ui/` with Tabler icon SVGs under `crates/moiren-app/assets/icons/`; architecture notes in `docs/`; shared logos in `assets/logos/`.

### Documentation map

Read the document that owns the area before changing it:

- [README.md](README.md): project overview, current boundaries, and the docs index.
- [FRONTEND.md](FRONTEND.md): UI and design collaboration rules; read it before changing anything under `crates/moiren-app/src/ui/`.
- [docs/TODO.md](docs/TODO.md): active design decisions and the current work queue.
- `docs/designs/NN-*.md`: numbered architecture and feature designs. Check a design's own status/date before trusting it.
- `docs/audit/YYYY-MM-DD/`: read-only audit reports. Open items are known work; verify their claims against the tree instead of quoting them as current state.
- `docs/validation/`, `docs/platform/`, `docs/experiments/`: acceptance records, platform notes, and experiment logs.
- `crates/*/README.md` and `crates/*/AGENTS.md`: crate contracts and local rules.

Architecture notes and user-facing docs are mostly Chinese; match the language of the document you edit. Old "completed" claims and audit verdicts are evidence, not verification: re-check the code and run the relevant gate.

## Build, Test, and Development Commands

Run from the root using the stable toolchain in `rust-toolchain.toml`:

- `cargo build --locked --workspace`: build all crates.
- `cargo check --locked --workspace --all-targets`: check all targets.
- `cargo test --locked --workspace`: run workspace tests.
- `cargo fmt --all -- --check`: verify formatting; omit `--check` to format.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: enforce clean lints.
- `cargo run --locked -p moiren-app`: launch the desktop UI.
- `cargo test --locked -p moiren-app --features gpui-test`: run the headless UI acceptance tests (Windows).
- `cargo run --locked -p moiren-app -- --gain 0.25`: run the offline audio demo.

`cargo test --locked --workspace` is the portable baseline, not the complete gate. CI also runs feature-gated suites and non-crate harnesses; the list of record is [.github/workflows/runtime-foundation.yml](.github/workflows/runtime-foundation.yml) plus the path-to-surface mapping in [.github/scripts/detect-changes.sh](.github/scripts/detect-changes.sh). Reproduce the relevant commands when touching those surfaces:

- `moiren-engine` encoded recorder sinks: `cargo test --locked -p moiren-engine --features mp3,flac --lib recorder` and `cargo test --locked -p moiren-engine --features mp3,flac --test host recording`.
- `moiren-windows-audio` VAD probes: `cargo test --locked -p moiren-windows-audio --features vad-probe --example vad_lifecycle --example vad_native --example vad_active_lifecycle`; probe examples need `--features probe-tools`.
- `moiren-app` on Windows: `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe` and `cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe --example vad_integration`; lint the profiler path with `cargo clippy --locked -p moiren-app --all-targets --features ui-profiler -- -D warnings`.
- `tools/rt-check`: `cargo test --locked --manifest-path tools/rt-check/Cargo.toml`.
- `driver/vad`: `bash driver/vad/test-portable.sh` for the portable helpers; from Windows with Visual C++ and WDK 10.0.28000.0 use `.\driver\vad\build.ps1` and `.\driver\vad\test.ps1`.

## Coding Style & Naming Conventions

Use four-space indentation and rustfmt defaults. Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Split modules by responsibility. Explain non-obvious unsafe, ownership, COM, and real-time constraints. Follow the existing gpui component/theme patterns in `src/ui/` (RenderOnce components, `#[derive(IntoElement)]`, `theme` tokens).

User-facing text in `moiren-app` goes through `t!` with keys in `crates/moiren-app/locales/zh-CN.yml`; the ledger and guard tests (`crates/moiren-app/tests/i18n_ledger.rs`, `crates/moiren-app/src/i18n_guards.rs`) reject drift. See [docs/designs/21-ui-conventions.md](docs/designs/21-ui-conventions.md).

## Testing Guidelines

Use Rust's `#[test]` harness and descriptive `snake_case` test names. Add regression tests beside affected behavior. Focus checks with `cargo test --locked -p moiren-engine`. Preserve allocation, concurrency, graph-validation, and DSP coverage. No numeric coverage threshold is configured. CI checks Ubuntu/Windows and nightly Miri with default/Tree Borrows modes. Pull requests run only the surfaces the diff touches (the mapping, including the crate dependency closure, lives in `.github/scripts/detect-changes.sh`) while pushes to main run the full suite. Keep hardware verification separate from portable tests.

Feature-gated and hardware suites are part of the gate only when their surface is touched; see the commands above and the crate guides. `moiren-plugins` and `moiren-app` are excluded from Miri by design: plugin tests spawn rustc and load real dylibs, and the app needs the desktop stack. Prefer the existing TLS-allocator and barrier-based concurrency patterns over sleeps when proving RT behavior.

## Architecture Constraints

Prepare graphs, buffers, and signal bindings off the audio thread. Real-time processing must avoid heap allocation/deallocation and blocking; reclaim retired plans on the control side. Keep logical topology separate from prepared execution plans and platform transports.

### Adding a node kind (checklist)

`NodeKind` is a closed enum; its exhaustive matches are the safety net that turns a missed touchpoint into a compile error. Adding a variant means updating all of the following (paths verified against the tree on 2026-11-11; re-verify when the tree moves):

1. `moiren-core`: the `NodeKind` enum and `initial_ports()` port layout in `crates/moiren-core/src/graph/model.rs`.
2. `moiren-engine`: the `Binding<S>` enum in `crates/moiren-engine/src/compiler/bindings.rs`, a `bind_*` constructor on `NodeBindings`, and the exhaustive `validate` match in the same file.
3. `moiren-engine`: the `NodeKind` lowering match in `crates/moiren-engine/src/compiler/plan.rs`.
4. `moiren-engine`: the parallel `NodeConfig` enum in `crates/moiren-engine/src/host/project.rs`.
5. `moiren-app`: the `GraphNodeKind` enum and its bidirectional `From` impls in `crates/moiren-app/src/project/graph.rs`.

The checklist is the compile-forced minimum. A usable node also needs non-compile-forced wiring: an engine processor/kernel and processor tests, plus app-side catalog, service preparation/builtin, export/persistence, and locale entries. Plugin nodes and the recorder are the reference for the full wiring; an enum arm alone is not a feature.

Keep the enum closed; do not replace it with a runtime registry — the exhaustive-match compile-time guarantee is worth more to an RT system than removing the boilerplate (2026-11-09 audit E-1).

## Commit & Pull Request Guidelines

Use the conventional prefixes in history: `feat:`, `fix:`, `refactor:`, `test:`, `chore:`, `docs:`, `perf:`, `ci:`, optionally scoped, e.g. `feat(engine): add compressor processor`. Keep commits focused; stage only relevant paths. PRs should describe behavior, reference applicable issues/design notes, report validation, and include screenshots for UI changes. When adding a crate, a `NodeKind`, or a feature gate, update this file and the affected crate guide in the same PR. Require passing CI before merging.
