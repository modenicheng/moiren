# Mixer Strip Redesign Plan

**Status:** Implementation is present. Native 1440 x 900 and 1024 x 720 layout and state captures were rerun on 2026-10-26, including the optimized build. Rust and gpui Mixer checks pass. Current high-refresh measurements and their remaining frame-pacing limits are recorded below; the earlier two-display performance numbers are historical.

**Goal:** Rebuild the main Mixer channel strips around the vertical layout in the user's Studio Pro 8 screenshots, using Moiren's flat visual language, real incoming/outgoing graph edges, one fader/meter scale, and documented digital peak display ballistics.

**Architecture:** Keep `LogicalGraph` and the application service authoritative. Split the existing Mixer view into focused routing, control, and level sections; share a pure dB-to-position mapping between the fader, meter, ruler, and pointer input. Add stereo balance to explicit channel-strip configuration, while retaining the existing graph apply and confirmation flow. Drive meter presentation from window frames, with a faster demand-gated measurement/delivery path that supports at least 120 Hz independently of ordinary application-state polling.

**Tech Stack:** Rust 2024, gpui-kit, existing Moiren theme/action components, existing sample-peak/RMS signals and channel-strip DSP.

## 1. Scope and decisions

The user's 2026-10-25 request takes precedence over older Mixer design notes:

- Use the reference's vertical order and compact channel-strip organization.
- Replace the Insert area with a list of actual incoming edges.
- Keep Sends as actual outgoing edges, with enable/mute, direct gain adjustment, and PRE/POST switching.
- Keep Mute and Solo; arrange channel mode and Dim in the second control row.
- Provide one cycling channel-mode button. Stereo is the normal state; the four transformations are Mono, Left only, Right only, and Swap.
- Use a flat Moiren fader, with a shared ruler and shared geometry for the level meter.
- Make peak motion more responsive and configurable, based on a documented DAW convention.
- Keep meter data and presentation responsive at the window's native refresh cadence, with 120 Hz as a required acceptance target. A nominal 120 Hz repaint loop over low-rate data does not satisfy this requirement.
- Use a simple stereo balance control.
- Retain strip ordering, resizable strip width, right-docked Bus/Output strips, selection, Inspector, and project persistence.

Recording, input monitoring, plug-in inserts, automation controls, and the duplicate input/output selector rows from the reference are outside this change. The incoming/outgoing lists already explain routing.

This is a main Mixer redesign. The tray flyout keeps its compact layout; shared model or ballistics changes require tray regression checks.

### Approach selection

| Approach | Result | Cost and trade-off |
| --- | --- | --- |
| Focused Mixer rebuild, shared scale, explicit strip balance | Recommended; delivers the requested layout and functional controls | Small DSP/model extension for balance; retains explicit Apply |
| View-only rebuild, balance only on existing Balance nodes | Smaller change; routing, scale, and meters can still improve | Most strips would have no functional balance control |
| Full mixer runtime-control redesign | Could also make every strip control immediately live | Requires parameter bindings, acknowledgement, solo dependency updates, and runtime transitions; separate engine/service scope |

## 2. Current implementation findings

Checked against the working tree on 2026-10-25:

- `src/ui/window/mixer.rs` renders every logical node as a strip, with Bus/Sink strips in a separate right dock.
- The view currently puts a large title above Sends, then vertically stacked controls, separate ruler/fader/meter widgets, and a gain readout.
- Sends already target actual `EdgeId` values and expose gain and PRE/POST. The main view does not expose the existing send mute field.
- There is no incoming-edge list.
- The fader uses a linear-in-dB `-60..+24` slider, while the meter uses a separate `-60..0` mapping and different track dimensions. Aligning their containers alone cannot produce a shared scale.
- `MixerState` already owns gain, mute, dim, solo, and channel mode. Dim currently applies -12 dB.
- Channel mode is serialized numerically as Normal, Mono, each channel Only, then Swap. Preserve these stored values.
- A normal strip has no balance field. `GraphEdit::SetPan` applies only to an explicit Balance/Pan node.
- `GraphEdit::SetMixer` changes candidate configuration and has no live parameter mapping. `ChannelStrip` reads compiled configuration. Main Mixer sliders use `release_applies = false`.
- Managed-runtime send gain/pan/mute have parameter mappings; changing PRE/POST still changes prepared routing.
- UI ballistics are currently in `src/ui/monitoring.rs`: peak-bar fall 11.8 dB/s, marker hold 500 ms, marker fall 30 dB/s, RMS attack/release 150/700 ms.
- `advance_display` repeatedly passes cached windows through the observation method. The redesign must distinguish a new measurement from a display tick.
- Main and tray windows already call `request_animation_frame()` and pump meters while rendering. Preserve that frame-driven path; the desktop's 33 ms state poll is not the current animation clock.
- Implicit UI meters use 480-frame windows: 100 publications/s at 48 kHz and about 91.9/s at 44.1 kHz. This is below the new 120 Hz data target.
- Managed measurements also pass through the host signal-bus pump, the service `MeterBridge`, and the application signal-bus pump. Host and application pumps currently have 5 ms scheduling intervals; the service can otherwise park for up to 10 ms. Reducing the analysis window alone cannot establish end-to-end freshness.
- The original meter used -18/-9 dBFS boundaries while `FRONTEND.md` described -12/-3. The latest request supersedes both with -6/0 dBFS boundaries and +12 dB of visible headroom.

Paths in this section are relative to `crates/moiren-app/` unless another crate is named. Archived application screenshots are historical references, not proof of the current UI.

## 3. Strip layout and appearance

Proposed order, from top to bottom:

```text
+------------------------+
| category color         |
| Inputs              v  |
| upstream node / port   |
| upstream node / port   |
+------------------------+
| Sends               v  |
| on  Bus 1       -6 PRE |
| [flat gain track]      |
+------------------------+
| L ------|------- R  C  |
| [     M    ][    S   ] |
| [   ST     ][ DIM   ] |
|        -1.7 dB         |
|                        |
|  L R    dB    fader    |
|  | |   +12      |      |
|  | |     0 -----|----- |
|  | |    -6     [=]     |
|  | |   -12      |      |
|  | |   -24      |      |
|  | |   -48      |      |
|  | |   -72      |      |
|  | |   mute     |      |
| peak readout / clip    |
+------------------------+
| type / order          |
| channel name          |
+------------------------+
```

This diagram specifies hierarchy, not final pixel rendering.

- Use the existing dark neutral theme, existing fonts, restrained accent colors, and real action components.
- Use solid fills, fine separators, and a tall flat rectangular thumb with one center line. No metal texture, bevel, gradient, glow, or decorative shadow.
- Use category color in a thin strip and small identity accents. Keep the working surface neutral.
- Put the channel name in the bottom identity area, matching the reference. That area remains the selection/Inspector and drag-reorder handle; retain the existing test identifiers where practical.
- Default new layouts to 160 px strip width; retain the supported 136..280 px range and every saved width. Do not overwrite a saved project layout.
- Inputs and Sends each have a shared height across the entire mixer. Their collapse toggles apply to that section across strips, preserving level-area alignment.
- Lists scroll internally. A strip with many routes cannot move its fader below neighboring faders.
- Remove the permanent bottom drag-help row; expose that help through the relevant tooltip.

### Height budget

| Region | Comfortable | Compact |
| --- | --- | --- |
| Inputs | 88 px | 64 px |
| Sends | 116 px | 80 px |
| Balance | 28 px | 28 px |
| Two control rows | 64 px | 56 px |
| Gain readout | 24 px | 20 px |
| Bottom identity | 52 px | 44 px |
| Level area | Fill remaining height | Fill remaining height, target at least 240 px |

Include padding in these regional budgets. Use a 40 px Mixer toolbar. Fit the controls and level area at 1024 x 720; remove the current blanket 720 px minimum strip height. At smaller heights, collapse route sections before requiring vertical scrolling. All strips share one resolved level-area top and bottom.

## 4. Inputs and Sends

### Inputs

- Enumerate edges where `edge.dst() == strip_node`.
- Each row identifies the upstream node and the relevant destination port; distinguish multiple connections between the same pair of nodes by their ports.
- Clicking a row selects the existing edge for Inspector. Use the same edge identity as Graph/Matrix.
- With no incoming edges, display the actual state. A Source strip can show its external resource binding as a distinct resource row; do not represent that binding as a graph edge.
- Unbound and disconnected states use explicit empty-state text.
- Input rows are informational in this iteration. They do not introduce a second incoming gain control for the same edge.

### Sends

- Enumerate edges where `edge.src() == strip_node`; retain all outgoing edges because the graph has no separate primary-output role.
- Use a compact row with an enable button, target name, gain value, and PRE/POST button at the far right. Put the slim gain track immediately beneath that row.
- The enable button writes `SendParams::mute`; turning a send off preserves its gain.
- Dragging the horizontal track changes send gain. Double-clicking its value opens precise numeric entry; double-clicking the track resets gain to 0 dB.
- PRE/POST remains visible at the minimum strip width. Its tooltip explains the actual prepared tap.
- Use stable `EdgeId` interaction identities. An edge deleted during editing cancels the edit safely.
- Keep connection creation/removal in the existing Graph/Matrix workflows for this iteration; do not add decorative `+` controls.

PRE remains after node DSP and after the strip's channel transform, mute/solo, and balance, before strip gain and Dim. POST is after strip gain and Dim. Adding balance to `StripConfig` must preserve that boundary.

## 5. Channel controls and balance

- First row: Mute and Solo.
- Second row: one mode button and one Dim button.
- Preserve the current serialized cycle: `ST -> MONO -> L -> R -> SWAP -> ST` for stereo.
- Left/Right Only copies the selected input channel to both outputs. Mono averages the input channels. Swap reverses channel order.
- A tooltip expands the abbreviated mode label. Active modes and Dim are distinguishable without relying only on color.
- Keep Dim at -12 dB in this iteration. The tooltip states the attenuation.
- Retain existing multichannel mode semantics. Show the simple balance control only for stereo; do not interpret an arbitrary multichannel stream as a stereo pair.

Add a default-centered `balance: f64` to `MixerState` and `StripConfig`, validated to finite `-1..1`. Apply the existing `Pan::coefficient` stereo balance law, with no center attenuation or boost. The strip order is channel transform, stereo balance, then strip gain/Dim. Mute/solo gates both taps.

The compact balance control is one horizontal track with a center tick and a short numeric label (`C`, `L 25`, `R 25`). Double-click resets it to center. It controls the strip's balance, not another node's Pan parameter.

Keep explicit Balance nodes intact. Their DSP position remains a separate graph parameter; when one also has noncentral strip balance, Inspector must identify the two parameters clearly.

Old project documents missing `balance` load it as zero through the existing serde defaults. Add round-trip and legacy-load coverage.

## 6. One scale and one level-area geometry

Use a pure `FaderScale` mapping for ruler placement, meter height, fader thumb center, and inverse pointer mapping. Do not interpolate the meter independently.

Proposed anchors, with `y` measured from the top of the usable track:

| dB value | Normalized y |
| --- | --- |
| +12 | 0.00 |
| +6 | 0.10 |
| 0 | 0.24 |
| -6 | 0.38 |
| -12 | 0.51 |
| -24 | 0.67 |
| -36 | 0.78 |
| -48 | 0.86 |
| -60 | 0.92 |
| -72 | 0.97 |
| Digital zero / mute endpoint | 1.00 |

Interpolate piecewise between finite anchors. Reserve the final short segment for the transition to mute; the bottom endpoint writes exactly zero gain. Positive finite values below -72 dB use the finite floor position with an accurate numeric readout, rather than being rewritten to zero during synchronization.

- Use +12 dB UI headroom; retain the `0..16` strip gain domain in the model.
- Give normal working levels around 0..-24 dB more travel than a linear dB slider.
- The same numeric dB position aligns across fader and meter, but fader gain dB and meter dBFS remain different quantities. State both units in tooltips.
- Mark meter clip at 0 dBFS while retaining visible positive headroom for internal floating-point signals.
- Use actual peak values for clip detection and numeric readouts; clipping the drawn geometry cannot erase an overload.
- Draw major ruler labels once. Use a stronger 0 line across the shared area.
- Align the fader thumb's center line, not its top edge, to the mapped position.
- Keep real silent data distinct from unknown/stale/unavailable data.
- For mono or multichannel strips, render actual available channels within the same level geometry; do not duplicate channels to manufacture a stereo meter.

Do not silently change the meaning of `gain_db`, `linear_gain`, or `sync_sliders` for the tray and send controls. Give each control an explicit mapping; main Mixer faders can use normalized slider state with inverse mapping.

## 7. Meter response and reference basis

### Evidence

Checked on 2026-10-25:

1. [Fender Studio Pro 8.2, Metering](https://fenderstudiopromanual.fender.com/en/Content/Mixing_Topics/Metering.htm) documents Peak/Peak-RMS modes and adjustable Peak Hold/Hold Length. That page does not publish an exact exponential release constant, so it cannot substantiate a claim of matching Studio Pro's complete algorithm.
2. [Ardour, Metering in Ardour](https://manual.ardour.org/meters/) distinguishes digital sample-peak meters from IEC PPM/VU types and notes that digital peak fall-off/hold have conventions rather than one exact specification.
3. [Ardour's metering preference implementation](https://github.com/Ardour/ardour/blob/master/gtk2_ardour/rc_option_editor.cc) lists 13.3 dB/s, fast 20 dB/s, and very fast 32 dB/s choices, among others.

Use **digital sample peak with a DAW-style selectable fall-off**. The recommended default is Ardour's documented fast 20 dB/s convention. This is a Moiren display preset, not a claim of IEC certification or an exact Studio Pro clone.

### Proposed behavior

| Part | Default |
| --- | --- |
| Peak rise | Immediate on a newly received peak |
| Peak-bar fall | 20 dB/s |
| Marker hold | 500 ms |
| Marker fall after hold | 20 dB/s, never below the peak bar |
| RMS attack/release | Keep existing 150/700 ms for this iteration |
| Clip | Latched from raw received peak >= 1.0; click to reset |

Expose response choices `13.3 / 20 / 32 dB/s` and peak-hold choices `Off / 0.5 / 1 / 2 s` through the meter context menu. Store this global presentation preference through existing preference persistence; it is not a per-project audio parameter.

For a current peak amplitude `p`, elapsed seconds `dt`, release rate `r`, and current raw target `x`, use:

```text
decay = 10 ^ (-r * dt / 20)
next_peak_bar = max(x, p * decay)
```

This is exponential decay in amplitude and a constant slope in dB. Do not average the rising sample peak with a slow one-pole attack. The `max` also prevents a falling display from undershooting its current target and bouncing back on the next tick.

Separate `observe_window` from `advance_display`:

- Only a new validated sequence updates measurement targets and renews marker hold.
- Display ticks advance decay using elapsed time; replaying cached data cannot renew hold.
- Apply the exact partial interval after a hold deadline when a frame crosses that deadline.
- Keep marker >= bar, including while a release crosses a new measurement.
- Reset presentation history on session/topic/channel changes.
- Retain the existing stale timeout and subscription lifecycle.

The detector remains a sample-peak/RMS detector. Do not label this as True Peak, LUFS, or a calibrated VU meter. The current Latest transport can replace analysis windows before the UI reads them; display hold cannot recover a peak from an unreceived window. Validate this explicitly and record the limit separately from display-ballistics correctness.

### Refresh scheduling and 120 Hz acceptance

The user's follow-up requires both responsive measurements and smooth drawing.

1. **Window frames drive presentation.** Read available meter data and advance ballistics once per native animation frame, including at 120/144 Hz. Do not introduce fixed 16/33 ms UI timers as the animation clock, and do not perform duplicate advancement for the same frame from a background state poll.
2. **Use a 240 Hz measurement target for implicit UI meters.** Derive `window_frames = max(1, floor(processing_sample_rate / 240))` during preparation. Examples are 200 frames at 48 kHz, 183 at 44.1 kHz, and 400 at 96 kHz. This supplies headroom for 120/144 Hz presentation and avoids baking a 48 kHz assumption into the setting. Keep explicitly configured Meter-node integration windows unchanged.
3. **Deliver without a slow application-state hop.** Prefer a generation-scoped direct engine-meter subscription delivered through the service's existing ownership/ticket flow. The UI reads that prepared subscriber on each frame; subscription creation and lifecycle work stay on the control side. Keep the high-rate payload out of ordinary snapshot polling. If the compatibility bridge is retained, give its demand-gated meter path an explicit scheduling budget of at most 4 ms rather than relying on the current generic poll timing.
4. **Include the engine signal pump.** A direct subscriber still needs the host bus pump. Establish the same at-most-4-ms scheduling budget while visible meter demand exists; leave lifecycle work bounded and avoid permanently accelerating unrelated idle polling. No busy-waiting, allocation, locking, or UI notification is added to the audio thread.
5. **Use real measurements for rises.** Apply a new peak immediately when delivered, and advance release/hold smoothly between measurements. Do not invent an unseen rising peak or smooth it away merely to make a curve look fluid.
6. **Retain visibility gating.** Release demand when meters leave the viewport or their window is hidden/minimized. High-rate delivery runs only while there is actual meter demand. Cache unchanged strip structure, labels, and control synchronization so the frame loop spends its budget on changing meter state.

The publication target is an analysis-window rate, not a promise that every audio backend invokes its callback 240 times per second. Windows can be produced in a batch at the end of an audio block. Measure callback cadence and actual consumer delivery as well; changing the measurement window cannot overcome a long backend block. Do not silently change the user's audio buffer configuration to meet a UI target.

Required validation:

- Verify frame-driven presentation at 60, 120, and 144 Hz, plus irregular frame intervals; add 240 Hz coverage when the native test environment supports it.
- At 44.1/48/96 kHz, verify the implicit analysis-window interval is at most 1/240 second, independently of audio-block boundaries. Verify explicitly configured Meter nodes retain their chosen window size.
- Measure sequence numbers and timestamps across measurement, engine pump, optional relay, subscriber read, and draw. A smooth decay animation over repeated old data is not evidence that the data path meets the requirement.
- With visible meter demand and a test backend producing audio at intervals no longer than 4 ms, require no configured delivery stage slower than 4 ms and a 95th-percentile age of the newest measured window at UI consumption of at most one 120 Hz frame (8.33 ms).
- In an actual 120 Hz window with eight visible stereo strips, target at most 1 ms of 95th-percentile UI work specifically for meter reads, ballistics, and meter element construction. Capture actual frame pacing and missed-frame counts; do not infer native performance from a timer or headless test.
- Exercise larger backend blocks separately. Report their actual fresh-data cadence while verifying that per-frame release/hold remains smooth and stale data is still identified correctly.
- Check one-window impulses, overwritten Latest windows, clipping, stale transitions, and session replacement while delivery and rendering run at different rates. Preserve the documented distinction between received sample peaks and peaks from windows that were not delivered.
- Repeat hide/show, scroll-out, main-window/tray handoff, and idle checks to ensure high-rate work stops when no meter is visible.

## 8. Module and file boundaries

Paths are relative to `crates/moiren-app/` unless indicated:

| File | Responsibility |
| --- | --- |
| `src/ui/window/mixer.rs` | Mixer page, dock, strip composition |
| `src/ui/window/mixer/model.rs` | Ordering, widths, control state, explicit slider mappings |
| `src/ui/window.rs` and `src/ui/window/tray_mixer.rs` | Native frame scheduling and one meter advancement per frame |
| New `src/ui/window/mixer/routing.rs` | Input rows and send rows |
| New `src/ui/window/mixer/strip_controls.rs` | M/S, mode/Dim, compact balance, identity footer |
| New `src/ui/widgets/fader.rs` | Flat fader, shared ruler, pure scale mapping |
| `src/ui/widgets/meters.rs` | Meter painting with supplied scale and geometry |
| New `src/ui/monitoring/ballistics.rs` | Pure time-based display envelopes and presets |
| `src/ui/monitoring.rs` | Validated observations and subscription lifecycle |
| `src/ui/signals.rs`, `src/service/signals.rs`, `src/service/mailbox.rs` | Prepared subscription delivery and high-rate meter transport |
| `src/service/managed/meters.rs` | Direct managed subscription ownership, or bounded compatibility relay |
| `src/service/signal_preparation.rs` | Sample-rate-derived implicit UI measurement windows on the legacy path |
| `src/ui/theme.rs` | Scoped Mixer metrics and semantic meter tokens |
| `src/service/graph/mixer.rs` | Strip balance validation and prepared config |
| `src/project/model.rs` and preference storage | Meter display preference with defaults |
| `src/project/tests.rs`, `src/project/v2_tests.rs` | Legacy load and round-trip checks |
| `src/ui/window/mixer/tests.rs` | User actions and rendered geometry |
| `src/ui/window/tray_mixer/tests.rs` | Shared-change regression checks |
| `crates/moiren-engine/src/processor/builtin/strip.rs` | Actual stereo strip balance |
| `crates/moiren-engine/src/compiler/plan.rs` | Verify new balance in both PRE and POST lowering |
| `crates/moiren-engine/src/meter.rs` | Pure sample-rate-to-UI-window calculation |
| `crates/moiren-engine/src/host/signals.rs` | Implicit UI window preparation and demand-aware signal pumping |
| `crates/moiren-engine/src/host/supervisor.rs` | Bounded high-rate signal scheduling during meter demand |
| `crates/moiren-engine/tests/compiler/sends.rs` | Compiled strip/tap audio behavior |
| `FRONTEND.md` | Update approved Mixer layout and metering decisions |

Reuse existing action components and icons. Add the new widget/module exports only where consumed. Keep route UI, mapping math, subscription state, and DSP independently understandable.

## 9. Implementation order and acceptance

These are review-sized phases. Each phase should carry its own focused checks.

- [x] **Scale and flat level section:** Implement and test mapping/inverse mapping; draw one ruler and aligned fader/meters. Shared anchor tests cover the working dB positions and snapshot synchronization. The compact-layout test also checks the actual 0 dB reference against pointer bounds and keeps peak readouts above the identity footer.
- [x] **Strip skeleton and route lists:** Implement aligned Inputs/Sends, direct send gain and enable, rightmost PRE/POST, lower identity handle, and compact controls. Focused gpui tests cover multiple routes, deleted edges, ordering, dock resizing, and 136 px width.
- [x] **Functional stereo balance:** Extend defaults/validation/serialization and actual strip processing. Engine and service tests cover balance with channel mode, Dim/mute/solo, and PRE/POST behavior. A gpui regression verifies that dragging uses only the originating strip's bounds and double-click resets to center.
- [x] **Meter response:** Extract configurable ballistics, separate window arrival from display time, add context-menu preferences, and retain demand-gated observation. Focused meter and signal tests pass.
- [x] **High-refresh data delivery:** Add the 240 Hz sample-rate-derived implicit measurement cadence, remove the slow state-poll dependency from the meter payload path, and budget every required pump/relay while demand exists. Portable signal tests and the explicit 120 Hz delivery benchmarks pass. Current native pacing is measured separately below.
- [x] **Integrated functional and visual acceptance:** Verify gain/mode/Dim/send states against the service and prepared audio path; check candidate edits and activation failure; verify the tray still applies according to its existing policy. Native captures cover both target sizes, selected/pending, applied mute, stopped/unknown, and routing-list overflow.

Keep explicit Apply semantics in this change. Main-strip gain, balance, mute, Dim, solo, and mode remain candidate edits; show pending feedback until engine confirmation. Do not auto-apply unrelated pending topology edits during a fader drag. Preserve managed send runtime updates where already supported.

> **Superseded 2026-10-26:** the explicit-Apply decision above is replaced by
> `2026-10-26-auto-apply-graph.md`. All edits apply automatically (live
> parameters for strip controls, automatic rebuild for structure edits), the
> Apply button and pending badges are removed, and meters are decoupled from
> revision equality. The layout, scale, balance, and metering decisions of
> this document remain in force.
### Current performance evidence (2026-10-26)

- The explicit short-callback benchmark consumes at 120 Hz with 96-frame callbacks. Across 120 post-warmup ticks, window age P95 was 7.8014 ms, below the 8.333 ms budget, with one tick without a fresh delivery. This is the portable fake backend, not a physical device-latency measurement.
- The 960-frame callback benchmark reports the expected batching: window age P95 23.3055 ms, with 72 of 120 ticks without a new window. Its processing capacity is the supported 1024-frame preset; the actual callbacks remain 960 frames.
- Native optimized capture reports a 6.944 ms display interval. With eight visible stereo strips, meter reads, ballistics and element construction take 0.0468 ms P95. Render-to-render intervals have median 7.036 ms and P95 9.1716 ms; three intervals exceed 12.5 ms, including transitions.
- The platform-present samples collected within the settled Comfortable window have median 6.972 ms and P95 9.161 ms; Compact has median 7.031 ms and P95 9.655 ms. Neither settled sample contains an interval over 12.5 ms. The median supports high-refresh presentation, but the P95 still exceeds a 120 Hz frame; this run does not establish uniformly smooth 120 Hz pacing.
- Unoptimized debug captures are unsuitable for the production pacing claim: this run measured about 81 ms median per render there. Optimized results above supersede that diagnostic capture for performance evaluation.

Native artifacts are in `.tmp/mixer-release-acceptance-20261026/`; command logs for this run use `.tmp/mixer-*-20261026.log`. These are local review artifacts.

The native [Comfortable](../../designs/screenshots/mixer-strip-redesign/comfortable.png)
and [Compact](../../designs/screenshots/mixer-strip-redesign/compact.png) captures
are checked in for pull-request review.

## 10. Verification commands

Run Cargo serially with the repository's local concurrency limits:

```powershell
cargo test --locked -p moiren-engine -j 2 -- --test-threads=2
cargo test --locked -p moiren-app -j 2 -- --test-threads=2
cargo test --locked -p moiren-app --features gpui-test -j 2 -- --test-threads=2
cargo check --locked --workspace --all-targets -j 2
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings
```

Use narrower test filters during each phase, then run the integrated checks once changes are stable. Document-only planning requires no Cargo run.

The initial planning pass encountered unrelated Windows-audio, VAD-driver, and code-quality worklist edits; they were no longer uncommitted at the start of this follow-up. Recheck the working tree at implementation time and preserve any unrelated changes. If they prevent workspace checks, identify the failure and validate the exact relevant tree without staging or changing those files.
