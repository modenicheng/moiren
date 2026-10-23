# Tray Mixer Flyout Implementation Plan

**Goal:** Left-click the tray icon to open an EarTrumpet-style flyout that
adjusts graph mixer volume quickly, without opening the main window.

**Architecture:** A `WindowKind::PopUp` window (Windows:
`WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, borderless, no taskbar entry) renders
`TrayMixer`, a compact root view sharing the desktop-owned
`Entity<UiState>`. Strips list the audio-relevant nodes (Source / Bus /
Sink); DSP shaping stays in the main mixer. Placement anchors to the tray
icon rectangle (physical pixels from `tray-icon` events) via Win32 monitor
geometry, then hands gpui logical coordinates for the matched display.

## Interaction

| Event | Behavior |
|---|---|
| Tray left-click (up) | Toggle flyout: live window closes, otherwise opens anchored to the icon |
| Tray left double-click | Open the main window (flyout closes on focus loss) |
| Tray right-click | Native menu unchanged (Show / Stop Session / Exit) |
| Focus loss / Escape | Close flyout |
| Fader drag | `Change` → candidate `SetMixer`; release → `ApplyGraph` |
| M / S click | `SetMixer` followed immediately by `ApplyGraph` |
| Candidate ≠ active | "Pending" badge in the header |

## Semantics and scope decisions

- **Auto-apply mirrors the Apply button.** A release or button click fires
  `ApplyGraph` whenever no apply is in flight, even if the local snapshot
  has not polled the drag yet — the same condition the main window's Apply
  button uses. This also means a flyout apply commits any other unapplied
  candidate edits made in the main window; that is existing Apply
  semantics, applied to one graph.
- **Not an OS volume mixer.** Moiren taps process audio passively and has
  no `IAudioSessionControl` volume control, so the flyout adjusts graph
  strip gain/mute instead of per-app OS volume. EarTrumpet parity would
  first need the per-app routing line.
- **No realtime strip parameters.** `ChannelStrip` bakes gain into its
  config and ignores `ProcessParameters`, so quick edits re-apply the
  engine project (the standard Apply path). Making strips
  parameter-addressable is a separate engine-side follow-up.
- **Fader drag needs a `SliderIndicator`.** `SliderState` maps pointer
  positions through bounds recorded only by `SliderIndicator::on_prepaint`;
  without one the custom faders (main mixer included) kept zero bounds and
  drags mapped to NaN, which `SetMixer` validation silently rejected. Both
  faders now overlay a transparent indicator matching the track geometry.
- **Focus-loss dismissal runs from the desktop poll, not the activation
  callback.** Destroying the window inside gpui's activation callback made
  gpui keep using it (`bounds_changed`/`refresh` logged invalid-handle and
  "window not found"). `Desktop::poll` closes the flyout within 33 ms once
  it has lost activation after having held it. The flyout is also
  force-activated via `activate_window()` right after opening: shown from a
  poll timer it can lose the foreground race to the shell, and a window
  that was never active has no first focus loss to detect.

## Tasks

- [x] Generalize `sync_sliders` over the owner entity with a
      `release_applies` flag; main mixer passes `false` (explicit Apply
      unchanged).
- [x] Route tray left-click to `TrayAction::ShowMixer(TrayAnchor)` carrying
      the physical icon rectangle; double-click keeps `ShowWindow`.
- [x] `TrayMixer` view: strip filter, compact fader/meter/M-S controls,
      immediate apply, Pending badge, focus-loss and Escape close, frame
      loop for meters, own `NodeMeters` instance.
- [x] `Desktop` lifecycle: toggle, stale-handle cleanup, poll cadence 33 ms
      while any window lives, flyout meter pump.
- [x] Placement: physical-space pure function (`tray_flyout_bounds`) over
      Win32 monitor geometry (`MonitorFromPoint` / `GetMonitorInfoW` /
      `GetDpiForMonitor`, matching gpui's own queries); logical conversion
      and display-id match; work-area fallback.
- [x] Fix the shared fader drag NaN (indicator) in both mixers.
- [x] Tests: five placement unit cases (bottom/top taskbar, 150% secondary
      monitor, edge clamp, off-screen anchor); two headless flyout tests
      (strip filter + immediate mute apply; track press → mapped gain →
      release apply).

## Validation

- `cargo test --locked --workspace`, `cargo test --locked -p moiren-app
  --features gpui-test` (283 passed), `cargo clippy --locked --workspace
  --all-targets -- -D warnings`, `cargo fmt --all -- --check` all pass.
- Native manual checklist (needs hardware): taskbar bottom/top/side,
  125%/150% DPI, secondary-monitor tray, toggle, focus-loss close, live
  meters during a running session, double-click opens the main window,
  right-click menu unaffected.
