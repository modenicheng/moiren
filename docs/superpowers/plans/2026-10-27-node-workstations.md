# Node Workstations Implementation Plan

> **For agentic workers:** Use subagent-driven-development for independent service and Inspector work; integrate and verify the native graph controls in this worktree.

**Goal:** Make graph nodes directly operable and turn the Inspector into a structured, secondary editor without replacing the authoritative graph.

**Architecture:** Keep camera, ports, wires and hit testing in the shared canvas geometry. Add node-specific native GPUI control surfaces over card bodies, with presentation-only control state and service-backed edits. Recording sessions belong to the service, never the window or audio thread.

**Tech Stack:** Rust 2024, gpui-kit 0.7.3, existing engine host and recording worker.

## Global Constraints

- Work only in the isolated `feat/node-ui-workstations` worktree based on `88d2774`.
- Preserve the original workspace and its untracked screenshots.
- Reuse local theme and ActionButton/Input/slider components; do not modify Figma masters.
- Keep actual graph topology and runtime confirmation in the existing service.
- Show unsupported formats and unavailable observations honestly; do not manufacture meters.
- Never perform file IO, allocation or blocking work on the audio thread.
- Run Cargo serially, with `-j 2` and tests using `--test-threads=2`.
- The user requested atomic commits, push and a PR on 2026-10-28. Do not merge without a separate request. Keep screenshot artifacts local and never stage them.

## Tasks

### Latest visual acceptance (2026-10-27)

- [x] First finish physical input only: device selection, digital trim, adaptive vertical per-channel meter; capture and inspect before advancing.
- [x] Replace fixed kind-specific card heights with intrinsic native-body measurement shared by canvas painting, ports and hit testing. Derive widths from control columns and information density.
- [x] Place each Clip latch at its channel's loud end. Meter track, scale and held marker share one height-dependent geometry; reduce tick density on short tracks.
- [x] Align knob labels, dials, numeric fields and units. Avoid sliders for DSP parameters; use plots, knobs, exact entry and mode buttons.
- [x] Iterate each remaining node separately with native screenshots. Remove empty flex space; grow Rack/Bus with their actual rows.

- [x] Baseline: run existing canvas tests, map current card geometry and native control patterns.
- [x] Node workstations: replace uniform fader cards with typed content, variable height, device selectors, real gain/processor controls and meters. Add deterministic schema/geometry tests before implementation. Preserve pan, zoom, dragging and connection behavior.
- [x] Inspector: split the monolithic renderer into responsibility-based sections, improve identity, advanced parameters, routing and diagnostics, and retain existing edit functionality. Add focused UI regression tests.
- [x] Recording: connect managed recording sessions to application requests and snapshots; expose configured path, supported WAV format, actual progress, errors and remaining capacity. Stop/finalize asynchronously and refuse accidental overwrite. Add lifecycle and estimate tests.
- [x] Integration: render recorder controls directly on its node; support numeric entry and safe event propagation. Update translation strings and headless UI tests.
- [x] Verification: run app tests including gpui acceptance, workspace check, strict Clippy and formatting. Capture the native graph/Inspector and inspect presentation. Document supported capabilities and remaining limitations.

## Ownership

- Main worker: canvas model/layout/painting, graph node body controls, shared parameter schema, shell state integration, translations, final tests and captures.
- Inspector worker: `ui/window/graph/node_inspector.rs` and its new child modules/tests only.
- Recording worker: service/model/managed recording integration and its tests; no graph UI edits.

## Acceptance

With the Inspector closed, a physical input can select its device, adjust digital level and show its own live meter. DSP cards expose their useful parameters rather than a generic balance track. A recorder shows its actual destination, supported format and lifecycle, with independent start/stop. Controls never drag a node or delete graph content while typing. Unknown observations are visibly unknown. The Inspector remains usable for advanced details and existing routing edits.

## Verification Results (2026-10-28)

All commands ran in the isolated worktree with `CARGO_TARGET_DIR=D:\coding\moiren\target`, `-j 2`, and test concurrency limited to two. The final sequential verification command exited successfully.

- `cargo test --locked -p moiren-app --features gpui-test --lib -j 2 -- --test-threads=2`: 465 passed, 0 failed, 3 ignored.
- `cargo test --locked --workspace -j 2 -- --test-threads=2`: 47 suites, 1,317 passed, 0 failed, 6 ignored, including the longer clock-control, fault and startup matrices.
- `cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings`: passed.
- `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -j 2 -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Native captures: 23 JSON manifests and 98 PNGs; each referenced image exists. Individual node cards and Inspectors were visually reviewed at both densities, with additional long-name, populated Bus/Rack, overview, recording and 32-channel cases. See `docs/designs/16-node-workstations.md` for adaptive-height measurements and real recording evidence.

The startup-import test previously unwrapped an intentionally nonblocking snapshot read while service publication could own its lock. It now uses the existing bounded test wait helper; first-driver-poll assertions and the production snapshot contract are unchanged.

Ignored tests require explicit hardware, machine-specific registry, native-window or idle-host benchmark conditions. Native screenshot audio uses the test backend, not physical hardware. WAV float32 is the only current recording format; realtime FFT, GR and LUFS observations remain explicitly unavailable. Destinations are session-local. Existing Tone and Mapper Inspector/card domain differences are recorded in the design document rather than silently presented as unified.

This verified milestone is being delivered as responsibility-scoped commits and a PR at the user's request. Capture source is versioned; PNGs and generated JSON manifests remain local and are excluded through Git's local exclude file. No Figma master or original-workspace source edits were performed.

## Follow-up Audit

The full redesign goal remains open beyond this verified milestone. The original workflow calls for pre-recording format selection, useful Rack-stage parameter editing and appropriate analyzer visualization. This milestone still has fixed float32 WAV output, default-only Rack stages, and an explicitly unavailable realtime spectrum display. Tone/Mapper parameter-domain consistency and complete Inspector precision editing also remain follow-up work. They must not be represented as complete merely because the baseline tests pass.
