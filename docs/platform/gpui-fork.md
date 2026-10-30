# The gpui-pre-windows fork

The workspace redirects `gpui-pre-windows` 0.3.9 to a fork through
`[patch.crates-io]` in the root `Cargo.toml`:

```toml
[patch.crates-io]
gpui-pre-windows = { git = "https://github.com/modenicheng/gpui-pre-windows" }
```

The fork (public) starts from the pristine 0.3.9 sources as published on
crates.io and carries one commit per patch, so every patch can be read —
and upstreamed — on its own. The mechanism behind the first patch is
already tracked upstream as zed#66221.

- `main` of the fork is the patch line; `Cargo.lock` pins the exact rev,
  and `--locked` builds do not float.
- The crate keeps `version = "0.3.9"`; a `[patch.crates-io]` entry must
  not change the version of the crate it replaces.

## Patches

1. `fix(vsync): skip invalidation of hidden and minimized windows` — the
   vsync thread called `RedrawWindow` on iconic windows every tick
   (~12% of one core while minimized). Invalidation now happens only for
   windows that can present; re-show and restore already redraw through
   `WM_SHOWWINDOW` and the `WM_SIZE` swap-chain recreation path.
2. `feat(vsync): pace each window on its output's vblank` — one waiter
   thread per distinct output refresh rate (`IDXGIOutput::WaitForVBlank`
   on a representative output) invalidates the windows on that output's
   group, instead of one `DwmFlush` thread pacing every window at the
   composition clock. Monitors with unknown rate or no DXGI output stay
   on a `DwmFlush`-paced fallback group. The former thread remains as a
   controller that owns GPU device-loss recovery and pauses the waiters
   during it.
3. `feat(vsync): follow display and refresh-rate changes` — the
   controller recomputes the rate groups on `WM_DISPLAYCHANGE` (and
   every 500 ms as a safety net), respawning waiters as rates appear or
   disappear. `PlatformDisplay::refresh_interval` already re-queries the
   current mode, so per-window frame budgets follow without more work.
4. Inactive-window ~30 fps throttle: no platform patch needed —
   `WindowOptions::inactive_frame_interval` is public and passed through
   by gpui-kit; the app sets it to `None` for the main window and the
   tray flyout (a mixer's meters must run while unfocused).

## Rebase procedure (gpui upgrade)

When the workspace moves to a newer gpui-pre / gpui-pre-windows:

1. In the fork, branch from the tag/commit of the pristine import.
2. Rebase the patch commits onto the new pristine import commit; each
   patch touches only `platform.rs`, `vsync.rs`, and `display.rs`, so
   conflicts stay local to the frame-pacing code.
3. Push, then `cargo update -p gpui-pre-windows` in this workspace to
   pin the new rev.

If the patches land upstream, delete the `[patch.crates-io]` section and
the lockfile entry reverts to the registry source.

## Validation notes

- The fork's standalone test suite needs `--features test-support`
  (without it, `render_to_image` is not on the `PlatformWindow` trait —
  a packaging quirk of the published crate, pre-existing), and
  `platform::tests::test_clipboard` requires an interactive session.
- Frame pacing itself is only observable on hardware with real displays;
  see `docs/designs/16-render-architecture.md` (P1 acceptance) and the
  handoff `docs/render-handoff-2026-10-31.md` for the measurement plan.
