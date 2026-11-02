# Design 18: Node cards and wiring interaction

Full rewrite of the graph-canvas node card plus the connection interaction
model, on top of the design-17 two-tier cache (cached panel + hot layer).
The goal is fewer steps for the common operations: wiring that tolerates
occupied ports and imprecise drops, and node cards that expose the one or
two controls a node is actually about, instead of deferring everything to
the inspector.

Assessed against `main` at `df7d488`.

## Principles

- **The cache architecture is a constraint, not a target.** Nothing here
  adds time or audio observations to `ControlsKey`. Static card content is
   painted by the cached canvas and rebuilds only when its semantic inputs
  change (snapshot, positions, selection, connection drag). Live pixels
  (meters, spectrum) stay in the hot layer at the shared frame's geometry.
- **A card shows at most four parameters by default.** Everything else
  lives behind one click in the detail window. The card answers "what is
  this node doing right now"; the detail window answers "let me change
  everything".
- **Canvas-painted, canvas-hit.** Node cards remain quads in the single
  canvas paint closure; knobs, chips and curve areas are hit-tested
  through `CanvasFrame` exactly like ports, so the drag/scroll gesture
  arbitration stays in one place.
- **Theme tokens only.** No new hardcoded colors; category tints stay,
  signal cyan stays reserved for meters and status, track fills stay
  neutral (`TRACK_FILL`).

## Part A — Wiring interaction

### A1. Displacing an occupied input (`ConnectReplacing`)

Input ports are strictly single-connection in `LogicalGraph`; today a
second connection to an occupied input fails with `InputAlreadyConnected`
and surfaces as a fault strip entry. Users then have to find and delete
the old wire first — the exact friction this design removes.

New `GraphEdit::ConnectReplacing { source, destination }`: in one
candidate revision, disconnect the edge currently feeding `destination`
(if any) and connect `source → destination`. One undo step; if the
connect half fails (cycle, channel mismatch), the candidate edit fails as
a whole and the old edge survives. Plain `Connect` keeps its strict
semantics for the other callers (matrix page, monitoring feeds).

The canvas connection commit uses `ConnectReplacing` exclusively. Output
ports keep unlimited fan-out, so displacement is only ever needed on the
input side.

### A2. Drop on the node body

Releasing a connection drag anywhere inside a node card connects to that
node, no port-dot precision required:

- Dragging from an **output** port → released over a node: connect to
  that node's input port. One input: obvious. Several (Bus): the input
  port nearest the release point.
- Dragging from an **input** port → released over a node: connect that
  node's output port to the dragged input.
- The dragged port's own node is excluded (self-connection is always a
  cycle). Channel mismatch also disqualifies a candidate.

`CanvasFrame::hit_test` gains a `DropTarget` (port or node, owned) for the
release path; node-body drops resolve through the same world model.

### A3. Live feedback during the drag

Today a drag paints only the ghost line; compatibility is checked at
release and failures surface after the fact. New paint, derived entirely
from the connection cursor at paint time (already in the panel key, so no
new key inputs):

- **Compatible free port** under the cursor: accent ring on the port.
- **Occupied input** that a drop would displace: warning ring on the
  port, plus a subtle strike/fade on the wire that would be replaced.
- **Invalid target** (same direction, channel mismatch, cycle): danger
  ring, no ghost-line snap.
- **Node body** with a resolvable port: soft accent halo around the card
  and the ghost line snaps to the resolved port anchor.
- Cycle pre-check runs on the app side over the snapshot's edges (a DFS
  from the candidate source); commit-side validation stays authoritative.

Both drag directions (out → in and in → out) get identical treatment;
reverse drags displace an occupied input exactly like forward drags.

### A4. Picking a wire up off an input (`Reconnect`)

Pressing an **occupied** input does not start a new reverse connection:
it picks that edge's head up. The drag returns to the connecting stage
with the edge's source output as its origin and the cursor as the free
head; the canvas projection drops the edge for the drag's duration, so
the old line disappears, the origin input paints empty, and the
candidate verdicts never see the edge being moved (its own input reads
`Valid`, not `Displacing`; cycles ignore it). Release:

- on another connectable input (port or body drop): one atomic
  `GraphEdit::Reconnect { edge, destination }` — the edge leaves its old
  input, lands on the new one, and whatever fed the new one is displaced.
  One candidate revision, one undo step, wholesale rollback if the
  connect half fails; the departed node is re-prepared too.
- back on its own input: nothing is committed.
- anywhere else — empty canvas, outside the window, an incompatible or
  cycling target: `Disconnect`. A free head let go is the
  drag-to-disconnect gesture; Escape restores instead.

Pressing an **empty** input keeps the reverse drag of A1–A3 unchanged.

## Part B — Node card rewrite

### B1. Card states

Every card wears a 1.5 px outline (window pixels, unscaled, so stacked
cards keep a crisp edge at every zoom) and the outline color alone
carries state. A state never changes the card body: a faulted or
re-preparing card keeps every knob, chip and meter exactly where and how
it is, so the user can fix the problem in place.

| State | Trigger | Visual |
|---|---|---|
| Normal | — | `SURFACE_3` card, 3px category-tint left rail seated inside a `GRIP` outline |
| Selected | in `selected_nodes` | `PRIMARY` outline |
| Error | a `GraphFault` whose subject is this node, or a route touching it | `DANGER` outline; hovering the card shows a tooltip under it with every fault's cause and hint |
| Loading | node in the new `GraphSnapshot::preparing_nodes` set | `WARNING` outline |

Precedence when states overlap: error > loading > selected.

The fault tooltip is painted by the graph page's hot layer, in window
pixels (a tooltip reads the same at every zoom), below the card and
flipping above it near the canvas bottom. Hover is tracked on idle
pointer moves into `StudioShell::hovered_node` outside every panel key:
a hover change costs one notified frame, never a page rebuild. The card
carries its faults as the service's catalog keys (`CanvasNode::faults`)
and the tooltip translates them exactly as the error strip does.

(Acceptance round 2 revision: the first cut marked error with a header
glyph and loading with a body scrim plus chip; both touched the body and
read as two different languages, so states collapsed onto the outline.)

`GraphFault` already carries `GraphSubject::Node(id)` end to end (service
`edit_subject` + driver `project_fault`), so the error set is a pure
projection of the snapshot. `preparing_nodes` is new service state:
populated from the subjects of structural edits while
`pending_revision` is set, cleared when the revision confirms or a fault
lands — that is the per-node granularity "compile is applying my change"
has today.

Custom display name (`SetLabel`, already persisted) renders in the
header. Custom color is a new `GraphNodeState::tint: Option<u8>` — an
index into a fixed theme palette (not a free color), so a user override
can never produce an off-theme card. `GraphEdit::SetNodeTint` sets it;
category tint is the default.

### B2. Card anatomy

World-space sizing (270 wide default; height by content, ports floor the
height):

```
│▌ [icon] Gain · "Vocal"            [!][⤢]     header 30px
│▌  ┌ knob ┐  ┌ knob ┐   1.0 dB                 body by kind
│▌  └──────┘  └──────┘
│▌  ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬  meter (horizontal)      shared strip
● in 2ch                                ● out    port rails
```

- **Header**: icon (tinted), display name, state glyphs at the right
  (error glyph, loading chip, expand button for the detail window).
- **Left rail**: 3px full-height tint strip — the "guiding decoration"
  that carries the category (or custom) color around the whole card.
- **Port rails**: ports straddle the edges as today, each with a tiny
  direction/channel hint (`in`, `out`, `2ch`) painted inside the card
  next to the anchor. Occupied inputs render filled, free inputs hollow.
- **Meter**: every node has one. Horizontal strip above the port rail by
  default; vertical right rail inside the card for tall curve nodes.
  Same shared `meter_spans` painters, same hot-layer repaint contract as
  today.

### B3. Per-kind bodies (the ≤4-parameter rule)

| Kind | Card body | Detail window |
|---|---|---|
| Input (Source) | current binding chip (click → picker) + meter | endpoint/process/tone picker |
| Output (Sink) | bound endpoint chip + meter | endpoint picker |
| Tone | frequency knob + meter | — |
| Gain | gain knob (dB) + meter | — |
| Balance (Pan) | balance knob + meter | — |
| EQ | frequency-response thumbnail (read-only) + vertical meter | curve editor with a draggable control point (freq × gain), Q wheel, numeric fields |
| Compressor | transfer-curve thumbnail + key readouts + vertical meter | full parameter panel + live curve |
| Gate | gate curve thumbnail + vertical meter | parameter panel |
| Limiter | limiter curve thumbnail + vertical meter | parameter panel |
| Delay | time knob + meter | feedback/mix fields |
| ChannelMapper | matrix preview grid (weight-intensity cells) + meter | matrix editor |
| Recorder | capacity/usage line + meter | buffer controls |
| Meter | large per-channel bars + peak/RMS readout | — |
| SpectrumAnalyzer | live spectrum plot on a square card (hot layer) + vertical meter | FFT/hop fields |
| Bus | name + meter | — |
| Rack | stage chain icons + meter | stage list |

Knobs are a canvas-painted control hit-tested through `CanvasFrame`:
wheel notches step the value, vertical drags adjust, double-click resets
— the `SliderBehavior` semantics on the canvas's gesture arbitration.
Wheel over a knob is consumed before the canvas zoom/pan handler sees
it; wheel anywhere else pans/zooms exactly as today. Knob-driven value
edits dispatch the same `GraphEdit`s the inspector uses, coalesced like
fader drags.

One dial painter serves every control. It is parameterized by the sweep
(`DIAL_SWEEP_START`/`DIAL_SWEEP_DEG`: bottom-left over the top to
bottom-right) and by the control's arc origin (`KnobSpec::origin`): the
value arc fills from that origin to the value in whichever direction the
value lies. Floor-anchored knobs (frequency, delay) fill from the left
end; bipolar knobs fill from their neutral point — pan from the center,
gain from 0 dB unity — so a neutral value paints no arc at all. Track,
value arc and the slim pointer are pill-ended (built as filled geometry,
since the stroke builder exposes no line cap).

The binding chip on source and sink cards is a hit target of its own:
pressing it selects the card and hangs an endpoint menu off the chip
rect (mounted at the shell root like the detail window, since the chip
is canvas paint). The menu lists exactly what the inspector's picker
lists — clear, tone, physical endpoints, exports, processes — with the
current binding highlighted; a pick dispatches the same edit and
closes, Escape or a press outside closes without one.

Curve thumbnails are analytic and static per settings (peaking-band
magnitude for EQ, static transfer curves for the dynamics family), so
they repaint only when the snapshot changes — no per-frame cost. The
spectrum node is the one live body: its bins arrive on a per-node signal
topic (mirroring `node/{id}/level`), and the hot layer paints them next
to the meters.

The analyzer rides the unified signal bus end to end. The engine's
`SignalSpectrumAnalyzer` publishes demand-gated FFT windows on its host
directory; the managed bridge relays the latest window onto an
application `node/{id}/spectrum` stream topic (depth 4) registered at the
shape the active plan publishes — a changed FFT size re-registers the
topic under a new identity, which is how subscribers learn to follow —
and the UI's `NodeSpectra` subscribes per visible card, on the Graph page
only. Demand flows back the same way: no visible card, no subscriber, no
FFT on the audio thread.

The plot is drawn to the industry's logarithmic conventions, never a
linear axis. Frequency runs on the ISO 266 base-2 fractional-octave grid
— 1/12-octave points anchored on 1 kHz from 19.7 Hz to 20.2 kHz (121
points), clamped to the running Nyquist — so the points are equally
spaced on the axis and the decade-standard grid lines (20, 50, 100, 200,
500, 1 k, 2 k, 5 k, 10 k, 20 k; 100/1 k/10 k labeled) fall where an
analyzer's do. Level is dBFS over the meters' −96 dB floor with lines
every 24 dB. A band takes the maximum bin magnitude across channels (the
engine's magnitudes are coherent-gain normalized, so a tone reads its
amplitude at its peak bin); a band narrower than a bin interpolates on
the dB-versus-log2-frequency line between its bracketing bins, and bands
under the first non-DC bin hold its level. The display rises instantly
and falls at 30 dB/s in the dB domain; at the floor the curve counts as
at rest, so a silent analyzer stands the frame loop down exactly as a
silent meter does, and only a moving projection or a status change wakes
the interface. The card is square — the body is sized so header, body,
meter reserve and port margins add up to the card width — with the
curve-node vertical meter down its right edge; the curve is a tinted
fill under a stroke, repainted whole by the hot layer (and by the cached
canvas as a snapshot, so a connection drag never blanks it).

### B4. Detail window

A settings-style in-window overlay (scrim + occlusion + Escape and
outside-click close), mounted at the shell root next to the settings
overlay — outside the cached canvas subtree, so editing a parameter
never re-projects the graph just to move a slider. The EQ editor is a
canvas with one draggable control point; dragging dispatches `SetBuiltin`
(the 200 ms auto-apply debounce and undo coalescing already handle the
ramp). Dynamics panels reuse the inspector's per-parameter rows.

## Implementation phases

Each phase lands as its own commit and keeps the workspace green; the PR
is pushed after phase 1 so CI carries the load from there.

1. `ConnectReplacing` service edit + canvas commit path (A1)
2. Node-body drop targets + drag-time highlighting (A2, A3)
3. Service state: `preparing_nodes`, node tint + `SetNodeTint` (B1)
4. Card shell rewrite: header, rail, states, port hints (B2)
5. Card meters, horizontal + vertical, through the hot layer (B2)
6. Canvas knob widget + knob bodies (B3)
7. Remaining bodies: bindings, matrix, rack, recorder, meter (B3)
8. Curve thumbnails (B3)
9. Detail window + EQ point editor (B4)
10. Spectrum topic + subscription + hot-layer paint (B3) — bus relay,
    ISO fractional-octave projection, square card
11. Icons: dedicated Tabler glyphs for the kinds that share fallbacks,
    registered in the `ActionAssets` allowlist
12. Capture states + acceptance screenshots

## Acceptance

`ui_capture` (gpui-test feature) screenshots remain the visual gate —
graph-page captures gain node states (selected/error/loading), each new
body kind, and the detail window. Headless gpui-test suites cover the
interaction contracts: displacement, node-body drops, invalid drops,
knob wheel consumption, and the cache invariants (no rebuild on
meter-only frames).
