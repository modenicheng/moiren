# Design 22: Fader feel — local-first values and a revision-fenced write-back

Status: implemented on `feat/fader-feel` (2026-11-10). The investigation
ran read-only on `main` @ `27aa57d7`; the implementation is based on `main`
@ `66dd66ef`. Measurements in §1.5, §1.5b and §4.

User report: the fader "doesn't follow the hand" and "occasionally jumps".
Hypothesis on the table: the decoupled-but-synchronous data bus makes the
thumb lag, or per-frame render cost is too high. This document audits both
chains at code level, measures the render budget and the drag-time CPU,
and records the async interaction design that shipped.

## 1. What the audit found

### 1.1 The thumb is local-first — but not every gesture painted at frame rate

`gpui-base`'s `SliderState::update_value_by_position` maps every pointer
move to a value immediately and notifies. The strip's `PanelHost` observes
its slider (`observe_nested`), so **no bus round trip is involved in
moving the thumb**. The 33 ms `DispatchGate` throttled *dispatches* of
graph edits, not the visual value; wheel and double-click commits bypassed
it.

Two qualifications the read-only audit missed, found while implementing
with a live-gesture trace (§4):

- **Pointer drags repaint everything, every frame.** The kit slider drags
  through gpui's drag-and-drop (`on_drag(DragSlider)`), and gpui calls
  `Window::refresh()` on every mouse move while a drag is active so the
  drag preview can follow the cursor. A refresh bypasses every cached
  view (`view.rs`: cache reuse requires `!window.refreshing`), so each drag
  frame rebuilds all strips and the inspector — about 7.6 strip renders
  per frame in the eight-strip capture. A pointer drag therefore always
  painted the thumb at display rate; the price is the drag frame cost in
  §1.5 (follow-up 1 in §6).
- **Wheel and touchpad gestures did not.** They write the slider state
  without a gpui drag, so the change reached the screen only through
  `PanelHost::notify_nested`, which throttled nested rebuilds to one per
  33 ms plus a trailing one (`NESTED_REBUILD_INTERVAL`). The throttle was
  added when the whole Mixer page was one cached panel and a drag rebuilt
  every strip per frame; eighty minutes later the page split into
  per-strip panels, and the throttle stayed. A precision-touchpad pan at
  144 Hz painted the thumb about 49 times per second (R7).

### 1.2 The echo chain is where the bus tax lives

Per dispatched drag tick (~30 Hz while dragging, before D4):

1. UI thread: `dispatch_edit` (now `SliderDispatch::send`) →
   `try_request_tracked` → mpsc SyncSender(64) + unpark (`mailbox.rs`).
   Non-blocking.
2. Service thread: `ServiceGraph::edit` (`graph.rs`) — per edit:
   `snapshot.clone()` **twice** (before + next) plus `graph.clone()`, a
   full `LogicalGraph::validate()`, `validate_exports`, and an undo-journal
   push retaining another full snapshot. `SetMixer` is *not* structural,
   so no re-prepare, but the effective gain/mute resync
   (`managed/parameters.rs`) reruns `mixer_configs()` — an O(n²) `find`
   per node (`graph/mixer.rs`).
3. `publish` (`mailbox.rs`): a **whole-`AppSnapshot` `PartialEq`** dedup,
   then an Arc swap. The service loop also builds a full `snapshot(&c)`
   every iteration (≤10 ms park) even when idle.
4. UI: the control-side poll runs every 33 ms (`desktop.rs` `spawn_poll`,
   shared with the headless twin) → `UiState::poll` → `cx.notify` → the
   next frame's `sync_mixer_controls`. **Publishes reach the UI at most
   ~30 times per second, whatever the dispatch rate.**
5. The strip **dB readout rendered from the snapshot**
   (`pages/mixer.rs`, `db_text(entry.mixer.gain)`), so during a drag it
   trailed the thumb by the whole loop: dispatch + service edit + ≤33 ms
   poll + frame ≈ **40–100 ms**. Thumb arrives, number chases (R2).

### 1.3 The "occasional jump" — release echo race

`sync_slider_range` stood the write-back down while `slot.dragging` was
set and lifted the flag in the `Release` handler *before* the release's
edit had been consumed. Sequence: the release dispatches request N+1; the
next poll can still deliver the snapshot that consumed N (the last
coalesced drag tick) → the write-back snaps the thumb back by up to one
dispatch interval of travel → the snapshot that consumed N+1 publishes →
the thumb snaps forward. **Two visible snaps, magnitude = the pointer
travel of the last tick.** That is the reported "偶尔跳动" (R1). The same
window existed after a wheel commit.

The regression test reproduces it: with the fence disabled, a stale
publication moves a just-released thumb from 0.7435 to 0.7477.

### 1.4 Wheel stepping folded every Lines delta to a whole notch

`scroll_gesture` (`widgets/scroll_step.rs`) turned every `Lines` delta into
`Notch(±1)` via `signum`, regardless of magnitude; only `Pixels` panned
proportionally. The fork's `WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL` handlers
always emit `ScrollDelta::Lines`; precision-touchpad pans arrive as
`Pixels` through DirectManipulation. Real wheels report whole lines and
were fine, but high-resolution wheels and touchpads whose pans arrive as
wheel messages report fractions of a line many times a second and climbed
a ±1 dB staircase per event (R3). PR #28 fixed only the Pixels half.

A wheel up from a fader gain below the −72 dB floor (which only a drag
reaches) multiplied the gain by one step, stayed below the floor and
snapped to digital silence (R10) — with fractional steps that would have
muted the strip on every small wheel-up event.

### 1.5 Render budget

The investigation's steady-state capture (`ui_capture` + `ui-profiler`,
release, synthetic meters, Mixer page, 2048×1280 @ 144 Hz, cache perfect):

| Window | draw p50 | draw p95 | draw max | layout p50 | prepaint p50 | paint p50 | present gap p50 | drops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| comfortable 1440×900 | 3.15 ms | 4.39 ms | 5.45 ms | 0.68 ms | 1.28 ms | 0.92 ms | 6.94 ms (144.0 fps) | 0 |
| compact 1024×720 | 3.21 ms | 4.19 ms | 5.09 ms | 0.70 ms | 1.44 ms | 0.87 ms | 6.95 ms (108.5 fps) | 0 (one 103.5 ms resize-settle outlier) |

**The renderer is not the bottleneck at 144 Hz**, during gestures either
(§4: every live-gesture capture holds a 6.94 ms present gap with no
drops). A pointer drag costs about 1–1.5 ms more per frame than the steady
state because gpui rebuilds every cached view per move (§1.1); at 240 Hz
(4.16 ms budget) that drag cost is the one to cut first.

Upstream residue: gpui vsync input latency (~12% of idle wakeups,
zed#66221) bounds the minimum event→photon delay; not addressable here.

### 1.5b Drag-time CPU: sampled, nothing is throughput-bound

samply (ETW, 1 kHz, release with line tables) over ~80 s with the audio
session running (WASAPI shared 96 kHz, two outputs) and a scripted storm of
~60 fader drags + wheel steps on the Mixer page. CPU time via per-sample
`threadCPUDelta` (raw sample counts are wait-inclusive and meaningless for
load):

| Thread | CPU | Verdict |
| --- | --- | --- |
| `moiren-host-control` | 5.69 s | 5.43 s of it blocked in a kernel wait; **0.18 s real work, all in `moiren_signal::registry::Bus::pump`** — the single largest real-work function in the process |
| UI main | 4.29 s | 1.08 s message-wait; the rest scattered — biggest items `BoundsTree::insert` (0.019 s), glyph `AtlasState::get_or_insert` (0.015 s), taffy/slotmap inserts, a btree `clone_subtree` (0.044 s) — **no function-level hotspot** |
| `app-service` | 3.13 s | 3.06 s blocked; ≤ 0.07 s work — the clone/validate/journal tax is real in code but **not user-observable at this graph size** |
| RT master + device | 2.25 s | audio threads healthy |

Total process CPU ≈ 21% of one core under an artificial drag storm.
**Neither the render pipeline nor the data bus is throughput-bound; the
reported feel issues are latency-structural** (§1.6). The 2026-10-29
audit's "service loop busy" concern does not reproduce as CPU load; it
survives only as echo latency (R2).

### 1.6 Root causes

| # | Cause | Symptom | Evidence | Fixed by |
| --- | --- | --- | --- | --- |
| R1 | Release echo race (§1.3) | occasional jump on mouse-up | code + regression test | D2 |
| R2 | Readout echo lag 40–100 ms (§1.2) | the number chases the thumb | code + loop arithmetic | D1 |
| R3 | Lines `signum` staircase (§1.4) | laddered motion on fine wheels and wheel-message touchpads | code + unit tests | D3 |
| R7 | 33 ms nested-rebuild throttle (§1.1) | wheel/touchpad thumb painted at ~30 fps | trace: 49 thumb repaints/s during a 144 Hz pan | D0 |
| R8 | The drag gate flushed a stashed value only on the next move or at release | a paused drag left the service (the audio) on an older value while the button was held | regression test: service 0.833 vs thumb 0.784 | D4 |
| R9 | Double click on the thumb never reset (pre-existing) | reset only worked beside the track | regression test | double-click fix |
| R10 | Wheel up from below the −72 dB floor snapped to silence | a wheel up muted the strip | unit test | D3 |
| R4 | Per-publish full sync walk | hygiene only — measured immaterial | code | — |
| R5 | Service-side per-edit deep clones + full validate + O(n²) mixer resync | hygiene only — ≤0.07 s CPU under a drag storm | code + profile | deferred (D5) |
| R6 | Upstream vsync latency residue | fixed ~1 frame floor | zed#66221 | — |

R9 surfaced through D0: the existing tray double-click test only passed
because its two clicks fell inside the 33 ms throttle, before the first
click's thumb jump repainted under the pointer.

## 2. Design (as implemented)

The direction: since the value space is already decoupled, decouple it
completely — the UI never waits for the bus, and the bus never pushes
stale values back into an active gesture. The `SliderState`-owns-the-thumb
model stays; the echo side is fixed. No new thread or timer.

### D2 — Revision-fenced write-back (kills R1)

Every slider edit — drag Changes, the release, wheel and double-click
commits — now goes through one `SliderDispatch` per slot, which builds the
request against the live snapshot, dispatches it, and records the
`RequestId` the mailbox assigned (`sent`). `SliderSlot::leads(consumed)` is
`dragging || consumed < sent`; the snapshot write-back in
`sync_slider_range` only reconciles a slot that does not lead the
snapshot. The request-id plumbing already existed: `publish` stamps
`consumed_request_id` after the round that consumed it.

- `sync_mixer_controls` keys on `(candidate_revision, consumed_request_id)`
  instead of the revision alone: an edit the service refuses publishes no
  new revision, and the slot that sent it must still fall back to the
  published value once the fence clears.
- A refused dispatch never sets the fence, so nothing needs clearing on
  dispatch errors.
- `cancel_drag` (undo under a live drag, SD-7) keeps the fence: the
  journal step is a newer request, so its publish lands while the drag's
  own older echoes cannot.

### D1 — Readouts render the slot's own value while it leads (kills R2)

`fader_readout_gain(slot, published)` returns the slider's own value while
the slot leads the snapshot (a drag, or an edit not yet consumed) and the
published gain otherwise; `send_readout_gain` is the send bars' twin
through their own gain pair (`send_gain`). The
edit dispatches exactly that gain, so the switch back on confirmation does
not move the number. Wired into the strip readout, the inspector's Fader
row, the tray flyout and the send rows. Each surface reads its own slot,
so mirrors show what their own thumbs show.

### D0 — Rebuild a panel for every nested change (kills R7)

`PanelHost::notify_nested` now simply notifies. gpui folds every
notification of a frame into one render, so the moved panel rebuilds at
most once per displayed frame, and with per-strip panels that is the one
strip that moved.

### D4 — Frame-paced drag dispatch (kills R8)

`DispatchGate` is now `{ armed, pending }`. The first Change dispatches at
once and arms a frame boundary through `Window::on_next_frame`; later
Changes supersede one pending value, which the boundary dispatches and
re-arms for, until a frame passes without a Change and the gate reopens.
A moving drag sends one edit per frame; a paused drag lands its last value
one frame later. Release still dispatches its final value (fenced per D2);
`cancel_drag` and the release drop the pending value so a cancelled
gesture's tail cannot land.

### D3 — Fractional wheel lines glide (kills R3, R10)

`ScrollGesture::Notch` carries `f32` notches. Whole-line deltas keep one
notch per event at any lines-per-notch setting, so a real mouse wheel is
unchanged. Fractional deltas step by their share of the Windows default
three-line notch (`LINES_PER_NOTCH`). This differs from the proposal
("convert to proportional Pan"): a share of a notch keeps every surface's
own step semantics — dB in gain space for faders and send bars (both
step through `fader_step`), the normalized step for compressor
parameters, a tenth for balance — instead of mapping wheel travel onto
the track length. `fader_step` enters just above the −72 dB floor on any
up-step from below it, as it already did from exact silence.

### Double click on the thumb (R9)

The gesture surface resets on the double click's second press in the
capture phase, ahead of the thumb and the track, and stops the press so
the track does not jump again and no drag starts. Covers every slider on
the shared surface: strip and tray faders, inspector rows, send bars.

### D5 — Service-side edit slimming (deferred)

Measured immaterial at current graph sizes (§1.5b). Ranked for later:
`SetMixer` skips the full `LogicalGraph::validate()`; drop the `before`
snapshot clone from the journal entry; pre-index `mixer_configs()`.

### D6 — Frame-budget guardrail

The live-gesture captures (§4) are the regression baseline: steady-state
and pointer-drag frames must not regress against an interleaved baseline
run, and any change that adds per-frame gesture work has to show its
draw p95 against the same table.

## 3. Implementation (`feat/fader-feel`)

| Commit | Change |
| --- | --- |
| `test(app): trace live fader gestures in the ui_capture meter run` | `MOIREN_CAPTURE_DRAG` harness (§5) |
| `fix(app): fence slider write-backs on the slot's own last request` | D2 |
| `fix(app): show the slider's own value in its readouts while it leads` | D1 |
| `fix(app): reset a slider on the double click's second press` | R9 |
| `fix(app): rebuild a panel for every nested change, not every 33 ms` | D0 |
| `fix(app): flush drag dispatches once per frame instead of every 33 ms` | D4 |
| `fix(app): glide fractional wheel lines instead of stepping a notch` | D3 |

## 4. Validation

Regression tests, each confirmed to fail on the code it replaces:

- `mixer_fader_small_drag_tracks_the_pointer_and_never_snaps_to_silence`
  now requires the released thumb to hold its exact value through a stale
  publication (R1; fails without the fence: 0.7477 vs 0.7435).
- `fader_readout_tracks_the_thumb_until_the_service_confirms_it` (R2).
- `a_touchpad_pan_rebuilds_the_panned_strip_every_frame` (R7; the throttle
  let 3 of 5 frames through).
- `double_click_resets_the_fader_after_the_first_click_moved_the_thumb`
  (R9).
- `a_paused_fader_drag_lands_its_last_value_at_the_next_frame` (R8; the
  33 ms gate left the service at 0.833 with the thumb at 0.784).
- Unit tests: the frame-paced gate (ported from the interval gate's four),
  fractional line classification, fractional and sub-floor fader steps,
  fractional compressor steps.

Full suites on the LAN machine: `cargo test -p moiren-app --features
gpui-test` (559 lib + integration), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, `cargo fmt
--check`.

Live-gesture captures, baseline (`b2f9f7e9` + the harness commit) against
`feat/fader-feel` before its rebase onto the two send-bar commits (which
do not touch the fader drag or pan path), interleaved in one round on the
same machine (5 s per window size, Mixer page, 2048×1280 @ 144 Hz,
synthetic meters). Draw times are p50 / p95 in ms; "thumb repaints" counts
strip rebuilds; "edits" counts graph revisions:

| Scenario | Window | draw base | draw fixed | thumb repaints/s | edits/s |
| --- | --- | --- | --- | --- | --- |
| Pointer drag, inspector closed | comfortable | 4.57 / 6.44 | 4.28 / 6.46 | whole window every frame, both | 28.8 → 143.6 |
| | compact | 3.99 / 4.55 | 4.10 / 4.65 | whole window every frame, both | 29.0 → 144.2 |
| Pointer drag, inspector open | comfortable | 4.61 / 5.20 | 4.69 / 5.26 | whole window every frame, both | 29.0 → 144.4 |
| | compact | 4.53 / 5.24 | 4.59 / 5.24 | whole window every frame, both | 29.0 → 144.0 |
| Touchpad pan | comfortable | 2.79 / 3.49 | 3.01 / 3.55 | 48.8 → 147.2 | 143.8 → 144.0 |
| | compact | 2.59 / 3.13 | 2.84 / 3.43 | 47.8 → 144.4 | 144.0 → 144.0 |
| Steady (next round) | comfortable | 3.67 / 4.47 | 3.70 / 5.32 | — | 0 |
| | compact | 3.32 / 4.13 | 3.20 / 4.00 | — | 0 |

Every run presented at 144 fps (present gap p50 6.90–6.96 ms); the few
present gaps above 12.5 ms (0–3 per run) show up on both sides alike.
Reading: pointer drags and the steady state cost the same within noise
while edits now reach the service once per frame instead of ~29 times a
second; a pan pays ~0.25 ms per frame to paint the thumb at display rate
instead of ~49 fps.

Still to do by hand on real hardware: (a) a slow drag below −72 dB, then
release — the thumb never snaps back; (b) a wheel spin and immediate
release — no stale echo; (c) touchpad pans on the DirectManipulation and
wheel-message paths; (d) undo mid-drag still lands (SD-7); (e) the tray
flyout mirrors the main window.

## 5. Measurement protocol (for reruns)

Live gestures, through the real input path:

```bash
cargo build --release --locked -p moiren-app --features ui-profiler \
  --example ui_capture
# MOIREN_CAPTURE_DRAG: strip (pointer drag, inspector closed),
# inspector (pointer drag, inspector open on the dragged strip),
# pan (precision-touchpad pixel scrolls); unset = steady state.
MOIREN_CAPTURE_METERS=1 MOIREN_CAPTURE_SECONDS=5 MOIREN_CAPTURE_DRAG=pan \
  target/release/examples/ui_capture.exe <output-dir>
```

The run prints strip and inspector rebuilds and graph revisions per window
size and writes `platform-<size>[-drag|-drag-inspector|-pan].json`. Three
pitfalls:

- **Interleave baseline and candidate binaries.** Absolute frame times on
  the development machine drift by about a millisecond over a day; only
  back-to-back pairs compare.
- **The capture window must stay in the foreground.** gpui throttles an
  inactive window to ~30 fps; discard a run whose meter frame interval
  p50 reads ~34 ms.
- **Real input lands in the capture session.** A steady run that reports
  revisions saw a stray pointer over the window; discard it. Run captures
  only on an idle machine (the windows also take focus).

Drag-time CPU (§1.5b): samply's ETW sampler counts blocked-in-wait
threads, so raw sample counts read like a full core — weight samples by
the profile's per-sample `threadCPUDelta`. samply did not resolve the
release PDB automatically (empty `nativeSymbols`); two workarounds that
both work:

```bash
# 1) build with function names regardless of profile.release.debug:
cargo build --release -p moiren-app \
  --config 'profile.release.debug="line-tables-only"'
# 2) record (UAC prompt once):
~/.cargo/bin/samply.exe record -r 1000 -d 150 --save-only \
  -o profile.json -- target/release/moiren-app.exe
# 3) if the profile still has raw offsets, symbolize offline:
_NT_SYMBOL_PATH='D:\coding\moiren\target\release' \
  "c:/Program Files (x86)/Windows Kits/10/Debuggers/x64/cdb.exe" \
  -z target/release/moiren-app.exe -c "ln 140000000+<rva>; q"
```

Closing the app window parks it in the tray, which keeps samply waiting —
end a recording with `taskkill //IM moiren-app.exe //F` (samply saves on
child exit; a graceful tray exit does not end it).

## 6. Follow-ups

1. **Pointer drags rebuild every cached view per move.** gpui refreshes
   the whole window on each mouse move during a drag-and-drop, which the
   kit slider uses for its thumb and track. Tracking the slider drag with
   mouse listeners instead of drag-and-drop (kit), or letting a drag opt
   out of the full refresh (fork), would bring drag frames back to the
   steady-state cost — the prerequisite for 240 Hz on the Mixer page.
2. **The 33 ms control poll bounds confirmation latency.** Mirrors (the
   inspector row while the strip is dragged, the tray flyout) and the
   readout's switch back to the published value move at the poll rate.
   Polling once per frame while frames are wanted would tighten both.
3. D5, when graph sizes make it observable.
