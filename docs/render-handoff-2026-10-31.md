# Render architecture handoff — P1 platform patches

Written 2026-10-31 for the session that executes **P1** of
`docs/designs/16-render-architecture.md`. Everything needed to start without
re-deriving this conversation's research; file:line references point into the
unmodified crate sources.

## Status snapshot

- `main` at `c719546` includes #37 (idle meter frames), #40 (idle wakeup
  quiesce), and the Mixer 240 Hz isolation line.
- **PR #41** (branch `docs/render-architecture`, worktree
  `.worktrees/docs-render-architecture`): the design note, this handoff.
- **PR #42** (branch `perf/graph-render-isolation`, worktree
  `.worktrees/perf-graph-render-isolation`): P2 graph-page render isolation,
  complete and validated (369 gpui-test green). Merge before or independently
  of P1; no code overlap with the platform work.
- P3 (frame-demand arbiter) and P4 (data-plane/flyout unification) remain,
  per the design note. Out of scope here.

## Chosen approach

**GitHub fork as patch carrier + `[patch.crates-io]` git dependency.**
Rationale: the patch set stays minimal and reviewable for upstreaming (the
iconic-invalidation fix is already tracked upstream as zed#66221, and #40's
description promised to file it), and a git dependency avoids vendoring
~10k lines into this repo. Fallback if the user prefers self-containment:
vendor the patched crate under `third_party/` and use a `[patch]` path
dependency — every patch below applies identically.

Bootstrap procedure:

1. `cargo package` is not needed — extract the exact 0.3.9 source already on
   disk: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-windows-0.3.9/`.
2. Create a GitHub repo (e.g. `modenicheng/gpui-pre-windows`) seeded from that
   source tree, **keeping `version = "0.3.9"`** — `[patch.crates-io]` requires
   the patched crate to keep the same version. One commit = one patch, so
   upstream PRs can be cut per commit.
3. In the workspace `Cargo.toml`:
   ```toml
   [patch.crates-io]
   gpui-pre-windows = { git = "https://github.com/modenicheng/gpui-pre-windows" }
   ```
   `gpui-pre` 0.3.9 (unpatched) picks the patched dependency up by name.
   Regenerate `Cargo.lock`. GitHub access needs the Clash proxy
   (`127.0.0.1:7897`) for both `gh` and cargo's git fetch — see the
   github-proxy-dependence note in agent memory.
4. Keep the patch commits rebased on pristine 0.3.9 so a future gpui-kit
   upgrade can rebase them.

## Patch 1 — skip iconic windows in the vsync loop (do first)

`platform.rs`, `begin_vsync_thread` (line 437): the loop already computes
`IsWindowVisible` and `IsIconic` (lines 483-491) but only uses them to
decide whether to record the frame signal — `RedrawWindow(RDW_INVALIDATE)`
runs on **every window unconditionally**. Move the invalidation under the
same visible-and-not-iconic branch.

- Effect: eliminates the ~12% one-core minimized cost measured in #40 (issue
  #38, upstream zed#66221). This is the one-line upstream fix.
- Verify: gpui re-shows a hidden window with a fresh invalidation (the same
  loop's comment says "Hidden windows may not consume WM_PAINT until shown
  again"); confirm no stale first frame after un-minimize on the rig.

## Patch 2 — per-output frame pacing

Today one global `VSyncProvider` thread (`vsync.rs`) blocks on `DwmFlush()`
(process-wide composition clock, follows DWM's choice of output), then
invalidates all windows. Presentation is `Present(0, DXGI_PRESENT(0))`
(`directx_renderer.rs:251`) — no per-swapchain vertical sync. Consequence:
on mixed-rate displays all windows share one cadence; neither display gets
its native rate reliably.

Design (from the P1 note):

- One waiter thread per **distinct output refresh rate**, using
  `IDXGIOutput::WaitForVBlank` (enumerate via `IDXGIAdapter::EnumOutputs`;
  `DirectXDevices` in `directx_devices.rs` already owns the factory/adapters).
- Map HWND → output with `MonitorFromWindow` matched against each output's
  `IDXGIOutput::GetDesc`. Monitor topology is available app-side already —
  `native.rs` has `monitor_geometry_at` — but the mapping belongs in the
  platform crate.
- Rebind on window move (WM_MOVE/WM_DISPLAYCHANGE handlers in `events.rs` /
  `window.rs`) and on display changes; a window spanning outputs follows its
  dominant output (single swapchain cannot scan out at two rates — accepted
  limitation, stated in the design note).
- Keep `Present(0)`; pacing comes from the waiters, not the swapchain.
  Alternative considered and deferred: `Present(1)` per swapchain — simpler
  but couples frame *submission* to composition and behaves worse for the
  multi-window case.

## Patch 3 — dynamic refresh rate

`VSyncProvider::new` measures the interval **once at startup**
(`get_dwm_interval` via `DwmGetCompositionTimingInfo`; fallback sleep
interval, `vsync.rs:23-38`). Runtime refresh-rate changes are never picked
up. With patch 2's per-output waiters, derive each waiter's period from its
output's `DXGI_OUTPUT_DESC` / `GetDisplayModeList` and re-derive on
`WM_DISPLAYCHANGE`. Also feed gpui's frame-budget metadata so the app's
4.167 ms (240 Hz) assumption stops being a constant — the Mixer isolation
measurements already recorded GPUI metadata disagreeing with DWM about the
actual rate.

## Patch 4 — inactive-window throttle opt-out (check before patching!)

gpui-pre core throttles inactive-but-visible windows with pending animation
callbacks to ~30 fps: `window.rs:1816-1826` picks `inactive_frame_interval`
when `!active && !high_rate_input`; the default is `Some(33_333 µs)` at
`platform.rs:2676` of the **gpui-pre** crate (note: core crate, not
-windows). The field is `pub` (`platform.rs:2530`) and a core test sets it
per-window (`window.rs:7967`).

**First check whether `gpui-kit` 0.7.3's `WindowOptions` (what
`desktop.rs::show_window` constructs) passes `inactive_frame_interval`
through.** If it does, the opt-out is app-side only — no platform patch:
set it for the main window and tray flyout (a mixer's meters must keep
running while unfocused). If gpui-kit drops the field, extend the fork to
gpui-pre core or add the pass-through to gpui-kit instead (gpui-kit is
already a direct dependency, `gpui-kit = "0.7.3"` in
`crates/moiren-app/Cargo.toml` — patching it is the same `[patch.crates-io]`
mechanism).

## Verification

- **Hardware**: frame pacing is only observable on the local rig with real
  displays (240 Hz primary; the 60 Hz + 240 Hz dual scenario is the
  acceptance case). The remote test machine (ssh alias `moiren-lan`,
  `D:\moiren`, use `CARGO_BUILD_JOBS=4`) is headless — its vsync loop never
  ticks (confirmed in #40), so it validates only the no-crash/no-regression
  path.
- **Method**: the throwaway `meas-autostart` automation from #40 drove
  per-scenario measurement on both machines; PR #40's table (stopped/running
  × visible/minimized, one-core %) is the template. `ui-profiler` feature +
  `MOIREN_CAPTURE_METERS` give frame cadence journals; DWM's nominal rate is
  recorded separately from GPUI metadata (see the mixer-render-isolation
  README for the measurement protocol).
- **Acceptance** (from the design note): minimized ≈ 0% app-side; a window
  on each of 60 Hz + 240 Hz displays sustains its own rate; changing a
  display's refresh rate in Windows Settings is followed without restart;
  unfocused visible window sustains full rate when opted in.
- Headless gpui-test cannot cover pacing; keep platform verification manual
  and hardware-bound, per the repo's "keep hardware verification separate
  from portable tests" rule.

## Environment and workflow constraints

- Workflow: worktree + PR mode; worktrees live only under `.worktrees/`;
  never touch `.git/` (including `info/exclude`); batch commits need explicit
  user authorization; verify the branch before committing.
- Commit style: `fix:`/`perf:`/`chore:` scoped prefixes; PRs describe
  behavior, report validation, require green CI.
- CI: PRs run only surfaces the diff touches (`.github/scripts/
  detect-changes.sh`); `Cargo.toml`/`Cargo.lock` changes pull a wide
  closure — expect the full suite on the patch-wiring commit. `gpui-pre-windows`
  compiles on Windows targets only; Ubuntu CI is unaffected but still
  resolves the lockfile.
- Windows-Miri has known std-allocator false positives; Linux CI is the Miri
  gate — irrelevant for this crate but don't panic at local Miri noise.
- `cargo` must be on PATH in this shell: `export PATH="$HOME/.cargo/bin:$PATH"`.

## Mechanics learned this session (avoid re-deriving)

Recorded in agent memory as `gpui-root-rerender-per-draw`; the short form,
in case P1 tests touch gpui-test:

- Every `Window::draw` rebuilds the root view's element regardless of dirty
  state; isolation tests must assert on cached-child counters (chrome /
  controls render counts), never on shell render counts.
- `.cached()` views lay out from their StyleRefinement as a root: use
  `size_full()` inside a sized slot, never `flex_1()` (collapses to 0).
- gpui-fast's retained prepaint re-requests animation frames itself when a
  retained layout changes (`gpui-fast-0.1.7/src/fast/retained.rs:1790,1797`)
  — a hidden self-continuation source when auditing wakeup attribution.

## Beyond P1

- P3 (centralized frame-demand arbiter) and P4 (meter data plane, flyout →
  canvas painter) are specified in `docs/designs/16-render-architecture.md`;
  P2 (PR #42) established the per-page pattern they build on.
- Upstream: file the iconic fix against the gpui-pre lineage's upstream
  (zed#66221 covers the mechanism); reference the fork commits.
