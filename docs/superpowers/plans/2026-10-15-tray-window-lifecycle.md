# Tray and Window Lifecycle Implementation Plan

**Goal:** Destroy closed windows without stopping audio, restore window state, and use supplied Moiren logos for tray/application icons.

**Architecture:** An application owner retains the service projection and tray, never closed window views. Save geometry/maximization/workspace independently of preferences; keep lightweight presentation state in memory. Rasterize supplied SVGs at build time, embed an ICO, and follow Windows system theme for the tray only.

## Constraints
- Preserve unrelated uncommitted changes and preference-write ordering; make one scoped atomic commit as requested, without creating a branch.
- No changes on the audio thread. Release window views and meter subscriptions on close.
- Native close and X destroy the window; minimize remains minimize; tray Exit shuts down orderly.
- Without a tray, close exits. Restore valid visible bounds, never minimized state.

## Tasks
- [x] Implement validated atomic window storage with regression tests.
- [x] Move service/tray ownership outside the window; test close/recreate and orderly shutdown.
- [x] Generate transparent light/dark tray logos and rounded application icon; embed Windows resources.
- [x] Fix Windows maximize/restore and reserve native caption drag regions without intercepting interactive controls.
- [x] Run app/default and headless UI tests, strict workspace Clippy and formatting on the isolated commit snapshot; verify native behavior.

## Validation
- Validate a HEAD-based snapshot with only this change, excluding concurrent audio/latency and graph-canvas development.
- `cargo test --locked -p moiren-app` and `cargo test --locked -p moiren-app --features gpui-test` pass.
- `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings` and `cargo fmt --all -- --check` pass.
- Explicitly run the ignored `native_chrome_toggles_maximize_hits_drag_regions_and_releases_closed_window` test: small/large icons present, maximize/restore toggles, both spacers hit `HTCAPTION`, actual HWNDs are destroyed, and maximization survives minimize/close/reopen.
- Native verification uses the portable service and a task-owned tray/window, not the user's running audio session.
