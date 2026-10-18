# Graph Canvas Rendering Implementation Plan

**Goal:** Preserve existing graph operations and node styling while sharing camera, layout, wire geometry, painting, hit testing, and overview.

**Architecture:** Pure geometry modules feed an immutable prepaint frame. GPUI stroke tessellation uses the exact screen-space centerline used for hit testing.

**Constraints:** Preserve unrelated changes. No staging, commits, branches, new dependencies, audio changes, or alternate topology.

- [x] Reproduce wire mesh overlap and camera limit failures with focused tests.
- [x] Extract camera, node layout, wire geometry, and frame modules with regression tests.
- [x] Replace painting and input with shared frame geometry; integrate drag, Fit, route selection, and overview.
- [x] Run headless acceptance tests, native captures, formatting, and app Clippy.
- [x] Audit scoped changes and every approved requirement before reporting completion.

## Verification

- Twenty geometry/persistence regression tests and three headless gesture/selection tests pass.
- Full app tests pass with `--features gpui-test` using `--target-dir target/graph-rendering-verify`.
- App Clippy passes for all targets with `--features gpui-test -- -D warnings`.
- Native capture succeeds at Fit, 25%, 89%, and 200%; captures are in `.tmp/graph-rendering-capture`.
- Scoped rustfmt and `git diff --check` pass. Workspace rustfmt still reports unrelated existing formatting in `service/mod.rs` and `moiren-engine/src/host.rs`; those changes remain untouched.
- Fit below the normal manual zoom range supports smooth relative zoom and validated project save/load.
- No commits, staging, branches, dependencies, or audio implementation changes are introduced by this work.
