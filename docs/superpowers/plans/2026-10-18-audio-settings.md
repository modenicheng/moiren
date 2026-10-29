# Professional audio settings implementation plan

> **交付状态（2026-10-29 核对）：已全量交付。** 设置页、持久化与档位接线随 b79224b、1b1a817、ce933e9 落地（`crates/moiren-app/src/ui/window/settings/`、`src/service/latency.rs`）。本文 checkboxes 未逐项勾选，工单不再具有待办性。

> **For agentic workers:** Use dispatching-parallel-agents for the disjoint native,
> service and UI slices. Parent owns portable intent, persistence and integration.

**Goal:** Provide functional professional audio configuration with defaults,
per-device overrides, WASAPI Shared/Exclusive and ASIO in a settings overlay.

**Architecture:** Portable intent is resolved into DeviceOptions for each graph
endpoint. The service starts a typed f32/f64 host and delegates native ownership
to the selected driver. The UI edits a draft and displays confirmed activity.

**Tech Stack:** Rust 2024, gpui-kit, WASAPI, IASIO, serde, bounded rtrb queues.

## Global constraints

- Preserve unrelated dirty probe and graph documentation.
- Rates: 44100, 48000, 88200, 96000 Hz.
- Buffers: 64, 128, 256, 512, 1024, 2048, 4096 frames.
- Prepare and reclaim outside RT; no blocking or allocation during processing.
- Defaults and overrides are intent; driver results are separate observations.

## Task 1: Portable intent and persistence

Files: `moiren-engine/src/audio_config.rs`, node/config, app/project/model/tests.

- [ ] Add tests for exact bounds, inheritance, resetting and invalid identity.
- [ ] Implement settings enums, defaults, validation and device resolution.
- [ ] Persist preferences and project audio with backward compatible defaults.
- [ ] Run engine and project persistence tests.

## Task 2: Native audio transport

Files: `moiren-windows-audio/src/professional.rs`, professional/native, wasapi,
asio, conversion and transport modules.

- [ ] Add conversion/buffer tests before implementation.
- [ ] Open native WASAPI Shared/Exclusive streams with exact intent validation.
- [ ] Enumerate/open IASIO drivers without requiring SDK downloads to build.
- [ ] Bind prepared IO, convert rates and execute typed hosts on one master.
- [ ] Verify callback ownership, cancellation, preallocation and cleanup.

## Task 3: Service integration

Files: `moiren-app/src/service/model`, mod, mailbox, managed, graph/preparation,
windows and `ui/state`.

- [ ] Test requested/active separation and per-endpoint graph translation.
- [ ] Dispatch validated settings and configure actual f32/f64 hosts.
- [ ] Restart when immutable audio settings change; retain explicit failures.
- [ ] Project available ASIO drivers and observed stream data.
- [ ] Run service lifecycle and UI state tests.

## Task 4: Overlay and device overrides

Files: `moiren-app/src/ui/window`, window/settings, devices, chrome, tests/capture.

- [ ] Replace settings page routing with a modal overlay and draft.
- [ ] Add system, direction defaults, appearance and diagnostics sections.
- [ ] Add per-device override editing and reset in Devices.
- [ ] Keep pinned Save/Restart actions and visible validation/error feedback.
- [ ] Test overlay isolation, dismissal, saving and device override resolution.
- [ ] Capture and inspect desktop and compact layouts.

## Task 5: Integration verification

- [ ] Run `cargo check --locked --workspace --all-targets`.
- [ ] Run `cargo test --locked --workspace`.
- [ ] Run `cargo test --locked -p moiren-app --features gpui-test`.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --locked --workspace --all-targets -- -D warnings`.
- [ ] Review scoped diff and document hardware verification limits.
