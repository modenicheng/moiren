# Render architecture: full-rate on demand

Assessment and roadmap for two rendering goals that the current UI only
partially meets:

- **Full rate.** Every visible window tracks the refresh rate of the display
  it is on, including after the user changes a display's refresh rate or
  moves the window between displays with different rates (for example
  60 Hz + 240 Hz). A single window straddling two displays follows its
  dominant output; no single swapchain can scan out at two rates at once.
- **On demand.** Nothing renders when nothing changed. No full-window
  repaint loops, no wakeups for minimized or hidden windows, and animation
  loops park the moment their content stops moving.

Assessed against `main` at `c719546` (includes #37, #40, and the
`perf/mixer-240hz-frame-clock` / `perf/mixer-display-refresh-cadence` line).
Measurements referenced below live in
`docs/designs/screenshots/mixer-render-isolation/README.md` and PR #40.

## Current architecture

### Frame clock (platform layer, `gpui-pre-windows` 0.3.9)

One process-global `VSyncProvider` thread drives every window:

- The thread blocks on `DwmFlush()` (`vsync.rs`), then calls
  `RedrawWindow(RDW_INVALIDATE)` on **every** window, including iconic ones
  (`platform.rs`, `begin_vsync_thread`). `IsIconic` is consulted only to drop
  the frame signal, not to skip invalidation. #40 mitigated the app-side
  cost (skip tree construction while iconic) but ~12% of one core remains in
  the platform frame path (issue #38, zed#66221).
- The fallback interval is measured once at startup and never refreshed, so
  runtime display-mode changes are not followed.
- Presentation is `Present(0, DXGI_PRESENT(0))` — no per-swapchain vertical
  sync. Pacing is entirely the global DwmFlush thread's.
- The core scheduler (`gpui-pre` `window.rs`) skips drawing clean windows
  (`invalidator.is_dirty()`) and throttles inactive-but-visible windows to
  ~30 fps while animation callbacks are pending.

Consequences: all windows share one cadence; on mixed-rate displays either
the fast window is dragged down or the slow window is spun; a user-changed
refresh rate is picked up only incidentally; background-visible windows are
capped at 30 fps.

### Pruning (three levels, structurally sound)

1. **Window level**: a clean window does not draw at all.
2. **View level**: only notified entities re-render; `request_animation_frame()`
   is "notify me next frame" — so whatever view owns the animation loop
   re-renders in full every frame.
3. **Subtree level**: `Entity::cached()` (gpui-fast) reuses an element
   subtree until that entity is notified.

### Application layer: one pattern, two maturity levels, plus a third variant

**Mixer page** (shipped, `mixer/meter_frame.rs`): the validated pattern.

- `MixerControls` caches the strip tree behind a semantic `ControlsKey`
  (~20 inputs; elapsed time and audio observations deliberately excluded).
- A sibling `MixerMeterFrame` is a hitbox-free painting canvas; the
  animation loop lives in *its* render (pump observations, continue frames
  only while `meters_in_motion()`), so animation frames never notify the
  shell.
- Toolbar/status are cached siblings (`CachedChrome`), never cached parents.
- Measured: CPU draw p95 18.0 → 2.7 ms; steady ~230–235 fps on a 240 Hz
  display; zero strip rebuilds; idle silence parks the loop (#37, REST_DB).

**Graph page**: the animation loop lives in `StudioShell::render` itself
(`window.rs`, `meters_frame_active` branch). Every animation frame
re-renders the shell root: full `CanvasWorld::positioned` re-projection
(O(V·E) edge depth plus per-node string formatting), ~14 interaction
closures, the error strip, and uncached toolbar/status (chrome caching is
Mixer-only, `chrome.rs`). This is the structural root of the "layer cache
bypassed by the owner's animation frame" finding from the 2026-10-27 audit.

**Tray flyout**: a third variant — DOM meters (`meter_spans` divs) with its
own in-render frame loop (`tray_mixer.rs`). Audit APP-3 tracks the
duplicated −6/0 dBFS span math between this and the canvas painter.

### Wakeup inventory (idle steady states, after #40)

| Source | Rate | State |
| --- | --- | --- |
| Global vsync `RedrawWindow` | DwmFlush cadence, all windows | upstream; ~12% core while minimized |
| Desktop 33 ms poll (UI thread) | 30/s | quiet while idle after #40 dedup |
| Animation-frame self-continuation | ≤ display rate | parks at rest (#37) |
| Service signal pump / snapshot publish | 1 ms / deduped | publish deduped (#40); pump still runs |

## Assessment

- Goal 2 (on demand) is **structurally supported** by the framework and
  proven on the Mixer page. The gap is consistency: the Graph page rebuilds
  everything per frame, chrome caching is page-specific, the flyout uses a
  third meter implementation, and stopping frames relies on each animation
  site remembering `at_rest` on its own.
- Goal 1 (full rate, dynamic, per-display) is **blocked at the platform
  layer**. No application-level change can deliver per-window pacing while
  one global DwmFlush thread paces every window and presentation is
  unsynchronized. The required platform work is small and local
  (see below); it does not need a framework migration.

No framework replacement is warranted. The plan is: platform patches
(fork), generalize the validated Mixer pattern app-wide, then collapse the
data-plane duplication.

## Roadmap

### P1. Platform patches (fork `gpui-pre-windows`, `[patch.crates-io]`)

Track as a fork; carry via `[patch.crates-io]` until upstreamed (upstream
awareness exists: zed#66221).

1. **Iconic windows**: skip `RedrawWindow` when `IsIconic` (the check is
   already computed). Kills the residual minimized cost from #40.
2. **Per-window frame pacing**: replace the global thread with a per-output
   waiter (`IDXGIOutput::WaitForVBlank`, one waiter per distinct output
   refresh rate is enough) driving the windows on that output; rebind on
   window move and `WM_DISPLAYCHANGE`. Present with sync interval 1 (or keep
   `Present(0)` with output-paced invalidation). A window spanning outputs
   follows its dominant output.
3. **Dynamic refresh rate**: derive per-window frame budgets from the
   window's current output; re-derive on display changes. Removes the hard
   240 Hz assumption (already observed disagreeing with GPUI metadata in
   the isolation measurements).
4. **Inactive 30 fps throttle**: opt-out for windows whose app declares
   background-visible animation demand (a mixer's meters while unfocused).

Acceptance: minimized = 0% app-side; two windows on 60 Hz + 240 Hz displays
each sustain their display's rate; changing a display's refresh rate in
Windows Settings is followed without restart; unfocused visible window
sustains full rate when opted in.

Decision point (user): fork location — vendor under `third_party/` as a
path patch, or a GitHub fork referenced by git. Either way the patch set
stays minimal and upstream-able.

### P2. Graph page render isolation (the Mixer pattern, generalized)

- Move the Graph animation loop out of `StudioShell::render` into a
  dedicated canvas view; animation frames notify that view only.
- Cache the world projection (`CanvasWorld::positioned`) behind a semantic
  key (graph identity, camera, node positions, selection, viewport, scale);
  animation frames repaint meter bars from live observations without
  re-projecting.
- Extend chrome caching to all pages (safe once no page's animation loop
  notifies the shell).
- Keep the pump contract: the 33 ms poll remains the at-rest pump that
  re-arms the loop when fresh data moves a layer (#37/#40 semantics).

Acceptance: ui-profiler + `MOIREN_CAPTURE_METERS` on the Graph page show
CPU draw p95 within the current display's frame budget during meter motion;
projection rebuilds = 0 during motion-only frames; the 368 gpui-test suite
and cadence tests pass unchanged in behavior.

### P3. Frame-demand arbiter (centralize "is anything moving")

One component owns, per window, the decision "does this window need
frames right now": ballistics `at_rest`, active drags, spring/transition
animations, and pending scroll all register. Replaces the three
hand-maintained continuation sites (shell render, `MixerMeterFrame`,
tray render). Pure refactor; behavior preserved.

### P4. Data plane + flyout unification

- Collapse the RT→UI meter path (engine `SignalHub` → app `SignalService`
  → `MeterSubscription`, three copies and three poll layers; audit X-1)
  to a single shared-window layer with frame-rate-independent pumping.
- Port the tray flyout to the canvas painter (`paint_channel_meter`),
  deleting the DOM span implementation (APP-3).

## Ordering rationale

P2 first among app changes because it is self-contained, measurable with
existing tooling, and required regardless of platform decisions; P1 needs a
fork-location decision and touches build infra for every crate; P3/P4 are
follow-on consolidations whose design is stabilized by P2's outcome.
