# Mixer meter render isolation

Measured on Windows on 2026-10-27 using the production GPUI renderer in
Release with `ui-profiler`. Baseline is commit `88d2774`; both sides use
the same eight-node fake-backend graph, display, viewport, and capture path.
No physical audio devices are opened and no preferences are written.
The percentile summary and event counts are retained in `timings.json`.

## Render boundary

- `MixerControls` caches the existing strip tree at its allocated page size.
- A sibling `MixerMeterFrame` pumps observations during prepaint, after
  controls report their current geometry and visibility. Its painting-only
  canvas reuses the original bar painter and has no input hitbox.
- Track/readout placements retain their scroll-container content masks.
  Real control changes invalidate the cached tree; elapsed-time publications
  do not. Slider model entities have explicit invalidation subscriptions;
  GPUI child notifications still invalidate input/hover/scroll state.
- Graph and tray rendering, audio processing, and meter ballistics are unchanged.
  The shell itself still renders; this is not whole-window render isolation.

## Release measurements

Three seconds per viewport, including startup/resize/inspector settling.
Values are milliseconds, p50 / p95. These are single-run local measurements,
not hardware-independent guarantees.

| Viewport | Metric | Baseline | Isolated |
| --- | --- | --- | --- |
| 1440 x 900 | CPU draw | 11.691 / 18.021 | 1.710 / 2.693 |
| 1440 x 900 | Prepaint | 7.461 / 11.886 | 0.331 / 0.663 |
| 1440 x 900 | Present interval | 12.540 / 18.863 | 4.248 / 6.998 |
| 1024 x 720 | CPU draw | 11.613 / 14.432 | 1.486 / 2.115 |
| 1024 x 720 | Prepaint | 7.537 / 9.613 | 0.320 / 0.529 |
| 1024 x 720 | Present interval | 12.126 / 15.242 | 4.181 / 6.116 |

CPU draw p95 is below the 4.17 ms budget for 240 Hz in both captures.
**Stable physical 240 Hz is not established**: GPUI reports a 6.944 ms display
refresh interval (144 Hz), and Windows scheduling/presentation still has tails.
Present events are not a measurement of physical scan-out refresh rate.

The snapshots below match the same-commit baseline pixel-for-pixel after
excluding the elapsed clock area. This includes meter labels and peak readouts.
The compact capture has five visible/live nodes; the comfortable capture has
eight. Do not compare their subscription counts as an error.

![Comfortable Mixer](comfortable.png)

![Compact Mixer](compact.png)

## Reproduce

```powershell
$env:MOIREN_CAPTURE_METERS = '1'
$env:MOIREN_CAPTURE_STATES = '1'
cargo run --locked --release -p moiren-app --features ui-profiler --example ui_capture -- <output-directory>
```

Outputs include native screenshots, `meter-frames.json`, and
`platform-{comfortable,compact}.json`. Use platform `draw_ms` for total CPU
render cost; meter-only timing now includes actual canvas painting, unlike
the old element-construction-only instrumentation.

Regression tests cover cached animation frames, unrelated shell notifications,
control invalidation, live managed observations, clip reset, scroll geometry,
offscreen demand, settings pause, stopped observations, phase-only legacy
activation, slider notifications, vertical decoration culling, and existing Mixer
editing/dragging/routing/multichannel layout behavior.

Local verification: `cargo test --locked -p moiren-app --features gpui-test`
(389 passed, four explicit opt-in tests ignored), strict Clippy with the same
feature and all app targets, workspace/all-targets check, and rustfmt check.
