# Application screenshots and mixer cadence measurements

The five-page screenshots (`comfortable-*.png` / `compact-*.png`, 1440×900 and
1024×720 logical) come from the `ui_capture` example:

```powershell
cargo run --locked -p moiren-app --features gpui-test --example ui_capture
```

It runs the real shell with the native renderer and writes into this
directory by default (pass another directory as the first argument to keep a
run out of the tracked tree); physical pixels scale with the display factor.
Before every shot the harness waits for the window to come to rest — panel
springs and animations settled, no request pending at the service, identical
consecutive probe frames — so a same-commit recapture is byte-identical.

The `mixer-*.png`, `platform-*.json` and `meter-frames.json` files are the
2026-10-28 mixer display-cadence measurements. Regenerate with:

```powershell
cargo run --locked --release -p moiren-app --features gpui-test,ui-profiler --example ui_capture -- docs/designs/screenshots/application-release
```

with `MOIREN_CAPTURE_METERS=1` set (see `crates/moiren-app/src/ui/shell/capture.rs`).
`MOIREN_CAPTURE_SECONDS` bounds the steady-state window (default 3 s). The
current code writes `platform-{name}.json` and `meter-{name}.json` per layout
plus a `mixer-{name}.png` capture; `meter-frames.json` is the combined naming
used by the revision that produced the committed evidence.

The `application-release/` sibling holds the release-build run of the same
measurement; its PNGs are byte-identical to the debug-run captures here and
are not duplicated.
