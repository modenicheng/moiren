# Design 17: Two-tier UI components — hot surfaces and cached panels

Status: implemented (2026-10-31, commits 6c38f30..70ef4bc). User decisions
D1–D3 confirmed 2026-10-31. Validation: the full gpui-test suite, the
workspace gates, and an `ui_capture` run — meter motion on the Mixer page
rebuilds zero strips while the hot layers paint at display rate (190
frames, meter work p95 3.3 ms debug, zero drops).

Baseline: `main` at `c47035c` — P1 (platform frame pacing, #46) and P2 (graph
render isolation, #42/#48/#50) landed. This is the consolidation step of
`16-render-architecture.md`: it absorbs that document's P3 (frame-demand
arbiter) and P4 (data plane + flyout unification) into a single component
model, and adds the caching coverage they assumed.

## Problem

P2 fixed the Graph page by hand-porting the Mixer pattern. Every place that
mixes cached controls with per-frame meter motion now hand-writes the same
mechanics, and every surface without meters skips caching entirely. The
complexity is organizational, not fundamental: gpui's primitives (entity
notification, gpui-fast `.cached()` splice/replay, `canvas`) are the right
ones — the app layer has simply written the same two patterns several times,
slightly differently each time, and left the pages without meters paying
per-frame costs.

### Hot (per-frame) painting: four paths, three mechanisms

| Path | Mechanism | Geometry source | Continuation predicate | Painted |
| --- | --- | --- | --- | --- |
| `MixerMeterFrame` (`ui/window/mixer/meter_frame.rs`) | sibling overlay canvas | private `Geometry` register written by `track()`/`readout()` prepaints | `meters_frame_active && meters_in_motion`, inlined | three-zone spans + RMS tail + marker + clip lamp + dB readout |
| `GraphMeterFrame` (`ui/window/graph/meter_frame.rs`) | sibling overlay canvas | `PreparedFrame` slot (`graph_canvas_frame`) written by canvas prepaint | same predicate, inlined again | `bar` only, node tint, no marker/RMS/clip |
| `TrayMixer` (`ui/window/tray_mixer.rs`) | DOM strips rebuilt in `render`, self-continuing | none | `!node_meters.at_rest()` only | `compact_stereo_meter` divs |
| Inspector meter (`ui/window/graph/node_inspector.rs`) | DOM, rides shell re-renders | none | none of its own | `compact_horizontal_meter` divs |

The cached Graph canvas paints a fifth variant (`paint_meter_bars`: live at
rebuild frames, frozen between them — the reason the overlay exists). The
zone/RMS/marker math is already shared through `meter_spans` (APP-3's math
half is done); the painting carriers and everything around them are not.

### Cached panels: six key/invalidation styles

1. **Semantic key + shell observer + diff + conditional notify** — written
   twice, line-for-line parallel, already drifted apart in field sets:
   `MixerControls` and `GraphControls`.
2. **a11y bypass wrapper** — REMOVED 2026-10-31, third machine round: the
   live-under-a11y escape hatch was written against stock-gpui semantics,
   but gpui-fast's splice gates on a11y staleness itself (`a11y::stale`) —
   it rebuilds until the a11y nodes are recorded and then replays them from
   the cache's records. The forced live rebuilds instead melted the whole
   window whenever a platform client probed accessibility (an alt-tab
   through certain setups does): every panel rebuilt per frame, the page
   dropped frames, and the Mixer burned ~3× a core until the window was
   recreated. Panels stay cached under a11y now; `cached_or_live` became
   `cached_splice`.

### Uncovered surfaces

Matrix, Devices, Settings, the sidebar (inspector + library), the error
strip, and the tray flyout have no caching: every 33 ms poll notify rebuilds
them in full. Worse, during meter motion **every drawn frame re-runs the
shell root render**, and all uncached children of the root pay full price at
display rate: the error strip clones `faults` and scans all nodes × ports per
route fault (`window.rs`), and the inspector/library trees are built even
while collapsed (`sidebar.rs` evaluates the panel content argument eagerly).

Wasted work inside the cached pages: both `ControlsKey`s deep-compare the
whole `GraphSnapshot` on every shell notify (no pointer fast path);
`can_start_transport` clones the entire logical graph per toolbar rebuild
while idle (`state.rs`); the matrix performs an O(edges) scan per cell.

### Structural risk with no defense

The invariant "a cached panel's output is a pure function of its key" exists
only in comments. Any state consumed by a cached tree but missing from its
key freezes on screen — exactly the #48 freeze that PR #50 fixed. Nothing
prevents the next occurrence except discipline.

## Goals

- Exactly one mechanism per tier. New pages declare *what their key is* and
  *what they paint*; they never re-implement how caching or frame
  continuation works.
- Dynamic pixels reach the screen through exactly one channel, so the #48
  failure class becomes structurally impossible rather than disciplined.
- Full caching coverage for static content, including the surfaces listed
  above.
- Delete the duplicated implementations rather than wrap them.

## Decisions (user, 2026-10-31)

- **D1 — unify meter semantics.** Graph card meters adopt the standard
  display: three-zone spans (`meter_spans`), dark RMS tail, held marker, clip
  latch. Node tint no longer colors meters; `meter_spans` stays the single
  segmentation source.
- **D2 — flyout goes canvas.** The tray flyout migrates to the hot-surface
  painter; the DOM meter implementation (`compact_stereo_meter`, `meter_bar`)
  is deleted. No compatibility shims. (The 2026-10-27 audit's "flyout meters
  are static, DOM is fine" premise no longer holds: the flyout animates at
  display rate by rebuilding its DOM every frame — the exact anti-pattern
  the canvas surfaces exist to avoid.)
- **D3 — inspector meter is a hot surface.** The inspector's horizontal meter
  becomes a small canvas hot layer, so it stays live while its panel caches.

## Tier H — `HotSurface`: per-frame painting

```rust
/// One per-window answer to "does this window need display-rate frames".
/// Ballistics motion, active drags, spring animations, and pending scroll
/// register here. The 33 ms poll is the complement: the at-rest pump that
/// keeps observations current and re-arms the loop when fresh data moves a
/// layer (#37/#40 semantics, preserved).
pub(super) struct FrameDemand { /* registered sources */ }

pub(super) trait HotSurface: 'static {
    /// Geometry shared with the sibling cached panel.
    type Frame;
    /// Drain fresh observations once per painted frame. Runs inside the
    /// shell update. Returns whether anything display-bearing changed.
    fn pump(&mut self, shell: &mut StudioShell, cx: &mut Context<StudioShell>) -> bool;
    /// Whether this surface's content is still moving. Registers with the
    /// arbiter; window-level eligibility is supplied by the mount site.
    fn wants_frames(&self, shell: &StudioShell, cx: &App) -> bool;
    /// The geometry the sibling cached panel's prepaint last wrote.
    fn frame(&self) -> &SharedFrame<Self::Frame>;
    /// Paint the live layer above the cached panel. Reads observations at
    /// paint time; must not mutate shell state.
    fn paint(&self, frame: &Self::Frame, shell: &StudioShell, window: &mut Window, cx: &mut App);
}
```

One generic view renders the standard overlay for every surface: a
hitbox-free `canvas`, absolute over the page; its prepaint pumps inside the
shell update and requests the next animation frame iff the mount site's
window predicate (visible, not minimized, settings closed, session running)
holds and the surface reports motion; its paint calls the surface with the
shared frame. The two `MeterFrame` structs become short `HotSurface` impls —
the pump/gate/continue skeleton, the no-hitbox placement, and the trace
hooks exist exactly once.

Window-level eligibility loses its `page ∈ {Graph, Mixer}` clause: eligibility
follows mounting, which lets the flyout and inspector surfaces use the same
view instead of their own loops.

### Shared geometry channel

```rust
/// Written by the cached panel's canvas prepaint; read by the hot layer at
/// paint time. Stale by design: between cache rebuilds the geometry the
/// cached panel painted stands (the contract currently documented on the
/// shell's `graph_canvas_frame` field, now named and shared).
pub(super) struct SharedFrame<F>(Rc<RefCell<Option<Rc<F>>>>);
```

`MixerMeterGeometry` (today's `Geometry` register) and `CanvasFrame` (today's
`PreparedFrame`) both become `F`. Writer and reader sides are documented once.
Per-surface paint policies stay in the impl; the standard hazards are the
occlusion skip (#50) and the drag yield, listed in the checklist below.

### One meter painter (D1)

`paint_meter_bars` (`pages/graph_canvas_view/nodes.rs`) is replaced by the
shared canvas painter over `meter_spans` — the same one the mixer overlay
uses — extended with the dark RMS tail, the held marker, the clip lamp, and
an orientation parameter (vertical for tracks and cards, horizontal for the
inspector). `NodeLevelsFn` widens from bar heights to
`Option<&[DisplayLevel]>` plus clip latches, so the overlay and the cached
canvas consume identical data. Graph cards lose their tint-colored bars by
design (D1).



The clip indicator itself is one painter for every surface
(`paint_clip_lamp`): a small lamp at the loud end of the track — above a
vertical track, right of a horizontal one — lit red while a clip latches,
dim otherwise, with click-to-reset on the surrounding element (mixer
strips, inspector meters, flyout strips). The graph cards' meter fill
never changes background for clipping; they carry the same lamp, and the
flyout's under-bar dot and the inspector's corner dot are gone.
### Flyout (D2) and inspector meter (D3)

- The flyout becomes a `CachedPanel` (strip controls) plus one `HotSurface`
  whose frame is its own small layout register;
  `compact_stereo_meter`, `meter_bar`, and the per-frame DOM rebuild path are
  deleted.
- The inspector panel's meter track registers geometry like the mixer's
  `track()`; a micro hot layer paints it. `compact_horizontal_meter` is
  deleted once the horizontal painter covers it.
- a11y trade-off, accepted as on the main window: canvas meters leave the
  accessibility tree; the fader/slider elements remain fully labeled.

## Tier C — `CachedPanel`: semantic-key caching

```rust
pub(super) trait CachedPanel: 'static + Sized {
    /// Inputs the panel's tree consumes. Elapsed time and audio observations
    /// must never appear here.
    type Key: PartialEq + 'static;
    fn key(shell: &StudioShell, cx: &App) -> Self::Key;
    /// Build the tree. Pure function of `Self::Key`: between key changes the
    /// cached subtree replays last frame's paint.
    fn render(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) -> AnyElement;
    /// Per-rebuild side state: re-register meter demand, reset geometry scratch.
    fn on_rebuild(shell: &mut StudioShell) {}
    /// Nested model entities (sliders, search inputs) whose notifications
    /// cannot invalidate this cached ancestor by themselves.
    fn observe_nested(shell: &StudioShell, cx: &mut Context<PanelHost<Self>>) {}
}
```

A single `PanelHost<P>` entity implements, once: the shell observer → capture
key → diff → conditional notify; the render-time key refresh (self-healing);
the a11y live path (one shared `cached_or_live()` helper, replacing four;
the helper's live branch itself was removed once gpui-fast's splice proved
to serve a11y from the cache — see hazard note above)
copies); the `on_rebuild` hook (replacing three demand-clear copies); the
nested-model subscription pool (replacing the mixer slider map and the graph
search observer); and a per-panel rebuild counter for the test harness.

Keys remain hand-written per panel — the semantics differ by page; only the
mechanics are shared. Two shared helpers keep keys honest:

- `ArcSnapshot<T>` — an `Arc` wrapper whose `PartialEq` tries pointer
  identity before walking the value. Every `Arc<GraphSnapshot>` field in a
  key uses it; the current per-notify full-graph deep comparison disappears.
- Chrome joins the tier with a key of (snapshot identity, page, compact,
  menu/focus flags), replacing the notify-all `CachedChrome`. Its rebuild
  rate stays "snapshot changed" (at least once per second while running, via
  the quantized elapsed), matching today; the "meter frames never notify the
  shell" convention remains the reason that rate is honest.

## Invariants and hazard checklist

1. A cached panel's render output is a pure function of its key. Dynamic
   pixels reach the screen only through a `HotSurface`.
2. Never nest cached panels. Chrome, page panels, and hot layers are siblings
   of the root; a cached parent forces its nested caches fresh.
3. Elapsed time and audio observations never appear in a key.
4. Never `request_animation_frame` from inside a cached subtree (it bypasses
   the scroll-layer caches — 2026-10-27 audit).
5. Hot layers paint above the panel; they yield during drags whose overlay
   must stay on top, and skip tracks occluded by later cards (#50).
6. Cache tests assert **paint output changes** while rebuild counters stay
   flat — never `shell_renders` (the root rebuilds every drawn frame), and
   never counters alone (the #48 lesson: broken states can pass both).
7. A scrollable container inside a cached panel needs the mixer's
   scroll-under-cache treatment — a shell-owned `ScrollHandle`, a wheel
   handler that notifies, and the offset in the panel's key — and even then
   a rebuild's scroll clamp can reset the tracked offset between frames
   (observed on the devices page's Exports list). Surfaces that must scroll
   freely stay live until this has a framework-level answer; the Exports
   tab deliberately keeps its rebuild-every-notify behavior.
8. A hot surface receives its sibling panel's `SharedFrame` at construction.
   Building a second frame of the same type compiles, runs, and paints
   nothing forever: the flyout shipped exactly this way (machine acceptance
   2026-10-31), its pump still running the frame loop against an
   always-empty track table. The pipeline has no runtime way to notice; the
   regression test asserts the surface sees and paints every registered
   track.

## Machine-acceptance follow-ups (2026-10-31)

Findings from the first real-machine pass over the implemented design,
each with its fix landed on the same branch:

- **Flyout without meter bars, CPU high while it showed nothing.** The
  hazard-8 disconnect. Fixed by constructing one `SharedFrame` and handing
  clones to both the strips and the hot surface.
- **Flyout strips showed the bare default labels ("Input"/"Output").** The
  flyout rendered `entry.label` where the main mixer renders
  `display_label` (export binding, input device, output endpoint). The
  flyout now uses the same helper, so both surfaces name a strip
  identically.
- **Mixer fader drag stuttered; the flyout's stayed smooth.** Every
  `SliderState` notification invalidated the whole cached panel, so a
  sustained drag paid a full strip-tree rebuild per display frame — cheap
  on a three-strip flyout, over budget on a full mixer page. `PanelHost`
  now gates nested-model notifications (faders, sends, inline editors,
  search inputs) behind a leading-edge throttle: the first notification of
  a burst rebuilds immediately, the tail coalesces into one trailing
  rebuild a dispatch interval later, and release/wheel paths repaint
  through their own publishes regardless. The interval matches
  `FADER_DISPATCH_INTERVAL`, so a drag never rebuilds the panel faster
  than its service confirmations actually change it.

### The cache splits per item (2026-10-31, second acceptance round)

The throttle bounded the *notify*-driven rebuilds, but a confirmed publish
still rebuilt the whole strip tree — and the reason was not the key. gpui
pre-windows' cached views invalidate whenever an entity they *read* is
notified, before any key compare runs: every panel read its owner during
render (key capture, and the owner access itself), and the shell is
notified on every publish, so keys were decorative for publish-driven
changes. Three changes landed together:

- **Per-strip panels.** `CachedPanel` gained per-instance state
  (`panel: P`): the Mixer page is a light container splicing one
  `StripPanel { id }` per node as siblings, each keyed on its own projected
  inputs. The page panel and its 19-field `ControlsKey` are gone.
- **Keyed render.** A panel can declare `KEYED = true` and build from its
  key alone; `PanelHost::render` then never touches the owner, so the
  framework's dependency tracking cannot invalidate it when the owner is
  merely notified. `StripKey` carries everything the strip renders — the
  entity handles for its fader, editors and send sliders, the shared
  geometry/visibility handles, the derived booleans — and a hand-written
  `PartialEq` compares only what the strip renders: its own node state,
  kind, channels, the edges touching it (in graph order), each far end's
  *display identity* (label/source/endpoint — muting node 0 must not
  rebuild node 1 for showing node 0's name), and the exports. Session
  bookkeeping (revisions, faults, undo flags) is deliberately excluded;
  the derived `PartialEq` on `GraphSnapshot` would have dragged every
  sibling into a rebuild on every edit. Chrome (toolbar, status bar,
  error strip) keeps the owner path — its content follows publishes and
  its keys change with them anyway.
- **Publish-driven keys run in the notification effect.** Key capture and
  nested subscriptions moved from `render` to the owner observer, which
  also captures the initial key at construction.

`UiState` now derives the observation source/sink pair itself (in `poll`,
a write-only entity — nothing that renders reads a notified entity for
it), replacing the render-ordered `refresh_observation_nodes` whose value
was always one publish stale at key-capture time. `SharedFrame`'s
update-on-empty is a documented no-op, so per-strip rebuilds create the
accumulating geometry frame when missing instead of resetting it — a
whole-table reset here would wipe siblings that are not rebuilding.

Regression: `mixer_publish_rebuilds_only_the_changed_strip` dispatches a
mute edit on one node and asserts exactly one strip render — the whole-page
panel produced one per strip.

Still true: chrome rebuilds on every publish (small, and its content is
publish data). The flyout keeps its single cached panel — two or three
strips measured smooth.

### Third machine round: the page that never stood down (2026-10-31)

Non-drag Mixer fps still read low, and at silence the page burned about
three times a core while the Graph page idled. The frame arbiter was
never the problem — the *demand* never cleared: the Mixer page mounts
the legacy capture-tap monitor meters (the Graph page mounts none), and
the tap is pre-fader, so it keeps producing signal after the user mutes
the fader. `meters_in_motion` therefore never rested and the page ran
the full frame loop at display rate forever; a window drag measured the
same difference. The legacy meters only display where a strip has no
node level topic, so the pump now subscribes them exactly for that case
(probe mounted AND the node topic missing); a managed strip renders
from NodeMeters alone, and the page stands down like every other page.
Regression: `mixer_stands_down_while_the_capture_tap_still_has_signal`.

The remaining per-frame budget went to text: the peak readout re-shaped
every track's dB string every frame. The hot surface now keeps one
shaped line per track and re-shapes only when the text changes — during
peak hold (most of the time) frames replay the shaped line.

## Deletions

- `compact_stereo_meter`, `meter_bar`, and the flyout's DOM rebuild path.
- `TrayMixer`'s private `NodeMeters` (a second ballistics state for the same
  nodes): the flyout shares the shell's `NodeMeters`, promoted to an entity
  both windows observe; per-window demand feeds the same `seen` table.
- The three hand-maintained continuation sites and every inlined
  `meters_frame_active && meters_in_motion` expression (replaced by the
  arbiter and the generic view).
- The duplicated `ControlsKey`/observer/diff blocks, the four a11y copies,
  the three demand-clear copies, and `CachedChrome`.
- `PresentationDemand`/`VisibleSignal` (`ui/signals.rs`): the demand gate is
  hardcoded true in `poll_windows`; the canvas demand registration it
  pretends to offer already exists and is the real mechanism.

## Rollout and acceptance

Each step lands separately, workspace suite green, with
`MeterTrace`/`ui_capture` evidence per step.

1. **Tier C machinery** — `PanelHost`, `ArcSnapshot`, `cached_or_live`; the
   mixer, graph, and chrome panels move onto it. Acceptance: rebuild
   counters behave identically (strip/graph/toolbar/status flat during
   motion-only frames); no visual change.
2. **Tier H machinery + D1** — `HotSurface`/`HotLayerView`/`SharedFrame`/
   `FrameDemand`; the mixer and graph frames become impls; the unified
   painter. Acceptance: `graph_meter_paints`/`meter_paints` advance while
   `graph_renders`/`strip_renders` stay flat during motion; ballistics-decay
   paint assertions green; graph cards show zones, marker, and clip.
3. **Flyout + inspector (D2/D3) and the data plane** — shared `NodeMeters`;
   flyout and inspector meters onto hot surfaces; DOM meters deleted.
   Acceptance: one ballistics state per node (assert in tests); flyout
   parity (open/close, clip reset, focus-loss dismissal); suite green.
4. **Coverage and cleanup** — error strip, inspector/library, matrix (with a
   port→edge index replacing the per-cell scan), devices, and settings
   panels; `can_start_transport` memoized per revision; `ActionIcon` static
   paths; the fader `gestures()` hint hoisted out of per-build `t!`;
   `VisibleSignal` deleted. Acceptance: per-panel rebuild counters flat
   across poll notifies that do not touch their keys; `ui_capture` on every
   page.

## Non-goals

- No framework or scene-graph replacement (`16-render-architecture.md`,
  "Assessment").
- No accessibility-tree caching (RESOLVED: gpui-fast serves a11y nodes
  from the cache's records and gates on staleness itself; the
  live-tree
  fallback stands).
- Keys remain per-page declarations; the model does not derive them.
- The root still rebuilds its element every drawn frame. The tiers keep that
  rebuild cheap — splice cached panels, paint hot layers — isolation, not
  elimination.

## References

- `docs/designs/16-render-architecture.md` — P1/P2 landed; P3/P4 absorbed
  here.
- `docs/audit/2026-10-27/04-app.md` — APP-3 (meter segmentation duplication);
  `05-cross-crate-workspace.md` — X-1 (RT→UI level topic directory
  duplication, addressed alongside the shared `NodeMeters`).
- Counters: `ui/window/mixer/performance.rs`; capture harness:
  `examples/ui_capture.rs` with `MOIREN_CAPTURE_METERS=1` and the
  `ui-profiler` feature.
