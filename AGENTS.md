# Repository Guidelines

## Project Structure & Module Organization

Moiren is a Rust 2024 workspace for Windows real-time audio:

- `crates/moiren-core`: editable `LogicalGraph` topology and control protocol.
- `crates/moiren-engine`: DSP, graph compilation, buffers, and runtime execution.
- `crates/moiren-signal`: typed, bounded Latest/Stream signals.
- `crates/moiren-windows-audio`: WASAPI capture/render, clock adaptation, and probes.
- `crates/moiren-app`: application service, CLI, and gpui desktop UI.

Each crate uses `src/` for sources, `tests/` for integration tests, and optional `examples/` for demonstrations. The desktop UI is gpui-kit (Rust, no markup language); UI code lives in `crates/moiren-app/src/ui/` with Tabler icon SVGs under `crates/moiren-app/assets/icons/`; architecture notes in `docs/`; shared logos in `assets/logos/`.

## Build, Test, and Development Commands

Run from the root using the stable toolchain in `rust-toolchain.toml`:

- `cargo build --locked --workspace`: build all crates.
- `cargo check --locked --workspace --all-targets`: check all targets.
- `cargo test --locked --workspace`: run workspace tests.
- `cargo fmt --all -- --check`: verify formatting; omit `--check` to format.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: enforce clean lints.
- `cargo run --locked -p moiren-app`: launch the desktop UI.
- `cargo test --locked -p moiren-app --features gpui-test`: run the headless UI acceptance tests.
- `cargo run --locked -p moiren-app -- --gain 0.25`: run the offline audio demo.

## Coding Style & Naming Conventions

Use four-space indentation and rustfmt defaults. Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Split modules by responsibility. Explain non-obvious unsafe, ownership, COM, and real-time constraints. Follow the existing gpui component/theme patterns in `src/ui/` (RenderOnce components, `#[derive(IntoElement)]`, `theme` tokens).

## Testing Guidelines

Use Rust's `#[test]` harness and descriptive `snake_case` test names. Add regression tests beside affected behavior. Focus checks with `cargo test --locked -p moiren-engine`. Preserve allocation, concurrency, graph-validation, and DSP coverage. No numeric coverage threshold is configured. CI checks Ubuntu/Windows and nightly Miri with default/Tree Borrows modes. Pull requests run only the surfaces the diff touches — the path-to-surface mapping, including the crate dependency closure, lives in `.github/scripts/detect-changes.sh` — while pushes to main always run the full suite. Keep hardware verification separate from portable tests.

## Architecture Constraints

Prepare graphs, buffers, and signal bindings off the audio thread. Real-time processing must avoid heap allocation/deallocation and blocking; reclaim retired plans on the control side. Keep logical topology separate from prepared execution plans and platform transports.

## Commit & Pull Request Guidelines

Follow history: `feat:`, `fix:`, `refactor:`, `test:`, or `chore:`, optionally scoped, e.g. `feat(engine): add compressor processor`. Keep commits focused; stage only relevant paths. PRs should describe behavior, reference applicable issues/design notes, report validation, and include screenshots for UI changes. Require passing CI before merging.
