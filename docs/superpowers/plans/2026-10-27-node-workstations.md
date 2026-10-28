# Node Workstations Implementation Plan

> **For agentic workers:** Use subagent-driven-development for independent service and Inspector work; integrate and verify the native graph controls in this worktree.

**Goal:** Make graph nodes directly operable and turn the Inspector into a structured, secondary editor without replacing the authoritative graph.

**Architecture:** Keep camera, ports, wires and hit testing in the shared canvas geometry. Add node-specific native GPUI control surfaces over card bodies, with presentation-only control state and service-backed edits. Recording sessions belong to the service, never the window or audio thread.

**Tech Stack:** Rust 2024, gpui-kit 0.7.3, existing engine host and recording worker.

**Current status (2026-10-28):** All requested node/Inspector work and local acceptance are complete. Earlier checkpoint counts and pending notes are historical; see Final Integrated Verification for the current result. PR #23 remains open, without merge; remote CI status is reported separately.

## Global Constraints

- Work only in the isolated `feat/node-ui-workstations` worktree based on `88d2774`.
- Preserve the original workspace and its untracked screenshots.
- Reuse local theme and ActionButton/Input/slider components; do not modify Figma masters.
- Keep actual graph topology and runtime confirmation in the existing service.
- Show unsupported formats and unavailable observations honestly; do not manufacture meters.
- Never perform file IO, allocation or blocking work on the audio thread.
- Run Cargo serially, with `-j 2` and tests using `--test-threads=2`.
- The user requested atomic commits, push and a PR on 2026-10-28. Do not merge without a separate request. Keep screenshot artifacts local and never stage them.

## Tasks

### Initial visual acceptance (2026-10-27)

- [x] First finish physical input only: device selection, digital trim, adaptive vertical per-channel meter; capture and inspect before advancing.
- [x] Replace fixed kind-specific card heights with intrinsic native-body measurement shared by canvas painting, ports and hit testing. Derive widths from control columns and information density.
- [x] Place each Clip latch at its channel's loud end. Meter track, scale and held marker share one height-dependent geometry; reduce tick density on short tracks.
- [x] Align knob labels, dials, numeric fields and units. Avoid sliders for DSP parameters; use plots, knobs, exact entry and mode buttons.
- [x] Iterate each remaining node separately with native screenshots. Remove empty flex space; grow Rack/Bus with their actual rows.

- [x] Baseline: run existing canvas tests, map current card geometry and native control patterns.
- [x] Node workstations: replace uniform fader cards with typed content, variable height, device selectors, real gain/processor controls and meters. Add deterministic schema/geometry tests before implementation. Preserve pan, zoom, dragging and connection behavior.
- [x] Inspector: split the monolithic renderer into responsibility-based sections, improve identity, advanced parameters, routing and diagnostics, and retain existing edit functionality. Add focused UI regression tests.
- [x] Recording: connect managed recording sessions to application requests and snapshots; expose configured path, supported WAV format, actual progress, errors and remaining capacity. Stop/finalize asynchronously and refuse accidental overwrite. Add lifecycle and estimate tests.
- [x] Integration: render recorder controls directly on its node; support numeric entry and safe event propagation. Update translation strings and headless UI tests.
- [x] Verification: run app tests including gpui acceptance, workspace check, strict Clippy and formatting. Capture the native graph/Inspector and inspect presentation. Document supported capabilities and remaining limitations.

## Ownership

- Main worker: canvas model/layout/painting, graph node body controls, shared parameter schema, shell state integration, translations, final tests and captures.
- Inspector worker: `ui/window/graph/node_inspector.rs` and its new child modules/tests only.
- Recording worker: service/model/managed recording integration and its tests; no graph UI edits.

## Acceptance

With the Inspector closed, a physical input can select its device, adjust digital level and show its own live meter. DSP cards expose their useful parameters rather than a generic balance track. A recorder shows its actual destination, supported format and lifecycle, with independent start/stop. Controls never drag a node or delete graph content while typing. Unknown observations are visibly unknown. The Inspector remains usable for advanced details and existing routing edits.

## Baseline Verification Results (2026-10-28)

All commands ran in the isolated worktree with `CARGO_TARGET_DIR=D:\coding\moiren\target`, `-j 2`, and test concurrency limited to two. The final sequential verification command exited successfully.

- `cargo test --locked -p moiren-app --features gpui-test --lib -j 2 -- --test-threads=2`: 465 passed, 0 failed, 3 ignored.
- `cargo test --locked --workspace -j 2 -- --test-threads=2`: 47 suites, 1,317 passed, 0 failed, 6 ignored, including the longer clock-control, fault and startup matrices.
- `cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings`: passed.
- `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -j 2 -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Native captures: 23 JSON manifests and 98 PNGs; each referenced image exists. Individual node cards and Inspectors were visually reviewed at both densities, with additional long-name, populated Bus/Rack, overview, recording and 32-channel cases. See `docs/designs/16-node-workstations.md` for adaptive-height measurements and real recording evidence.

The startup-import test previously unwrapped an intentionally nonblocking snapshot read while service publication could own its lock. It now uses the existing bounded test wait helper; first-driver-poll assertions and the production snapshot contract are unchanged.

Ignored tests require explicit hardware, machine-specific registry, native-window or idle-host benchmark conditions. Native screenshot audio uses the test backend, not physical hardware. At this baseline WAV float32 was the only recording format; the Recorder completion below supersedes that limitation. Realtime Spectrum and Tone/Mapper domain alignment were completed in later increments; GR/LUFS remain unavailable. Destinations are session-local.

This verified milestone is being delivered as responsibility-scoped commits and a PR at the user's request. Capture source is versioned; PNGs and generated JSON manifests remain local and are excluded through Git's local exclude file. No Figma master or original-workspace source edits were performed.

### Pre-PR Integration Check

The branch was rebased onto `origin/main` at `05e340e`, retaining the upstream Mixer render isolation and test-job improvements. On that integrated tree, app GPUI tests passed with **477 passed, 0 failed, 3 ignored**; default and GPUI-feature workspace strict Clippy, formatting and diff-whitespace checks also passed. The upstream live-Mixer cache test was measuring while responsive panels were still animating; it now uses the existing panel-settle helper before recording meter-only frame counts, with the original assertions unchanged. The earlier 1,317-test workspace result above describes the pre-rebase baseline, not a fresh complete post-rebase run.

The PR diff contains no screenshots, generated capture manifests or binary files. The 98 local screenshots predate the rebase; fresh post-rebase integration evidence is the headless GPUI suite, not a claim that all native captures were repeated.

## Follow-up Audit

This is the historical baseline audit, superseded by the completed Recorder, Rack, Spectrum, Tone, Mapper and precision-editor increments below. Milestone counts describe their respective trees, not the final integrated result. Final integrated verification is recorded separately.

## Recorder Completion (2026-10-28)

Keep this increment limited to the Recorder node, then capture and inspect it before proceeding to another node. Preserve the existing float32 default and service ownership. Do not commit screenshots or generated capture manifests.

- [x] Add real float32 / PCM24 / PCM16 WAV encodings, off the realtime thread, with tested frame sizes, headers, RIFF padding/limits, saturation and non-finite handling.
- [x] Expose service-owned format intent; allow edits before recording, reject changes while starting/recording/finalizing, preserve the selected format when changing destination and invalidate stale capacity estimates.
- [x] Put three explicit encoding choices on the card, with concise conversion/headroom information and shared format state in Inspector. Do not expose unsupported containers or editable rate/channel controls.
- [x] Verify UI selection with Inspector closed, busy-state input suppression, selected-format capacity calculations and real recordings for all formats.
- [x] Capture each encoding in both densities and inspect layout/adaptive meter height: 22 fresh local PNGs and three manifests; six format-test WAVs verified and removed.
- [x] Deliver focused engine, service, UI and acceptance commits to PR #23, excluding every generated capture artifact. Commits `10a8ae6`, `9ee0e6c`, `b491624`, `bc33e6b` and documentation `175a3aa` are pushed; PR remains open, not merged.

Validation: 488 app GPUI tests passed (three ignored); 350 engine tests passed across eight suites; final capture regression 17 passed; strict workspace Clippy with default and gpui-test features, fmt and diff checks passed. Review caught a render-time selected-state race; a failing rapid-click test reproduced it, and always dispatching the final enabled click made it pass. Native capture also exposed unsupported disabled metadata and hover-tooltip obstruction; acceptance now uses real busy clicks and moves the pointer before screenshotting. Recorder vertical bounds are explicitly verified against each measured card body.

## Rack Stage Workstation (2026-10-28)

Continue one node at a time. A compact ordered stage list selects one inline editor; do not expand every stage into a wall of knobs. Card shows useful primary controls, Inspector adds the full parameter set. Share selected stable slot identity, but never mount one InputState on both surfaces. Reuse existing theme, relative-drag dials and exact numeric fields. Mapper uses a paged signed-weight matrix instead of rotary controls. No fake stage meters or GR.

- [x] Add field-level Rack service intents that merge with latest candidate state, validate actual stream widths and preserve parameter-specific undo. A service-owned epoch rejects stale editor callbacks after stage replacement, undo or project load.
- [x] Reuse parameter/dial rendering through explicit keys. Keep stage selection presentation-only, and separate card/Inspector inputs, error text and drag ownership.
- [x] Render selected stage parameters inline for all eight leaf processor types; Pan toggles and paged Mapper weights remain directly accessible. Replace Inspector's add-Gain-only stub with the shared stage view.
- [x] Verify queued independent edits, stale identity rejection, undo granularity, focus/typing/drag behavior, route order, signed weights and invalid input.
- [x] Capture empty, selected Gain/EQ/Compressor/Mapper and populated Rack at both densities, inspect and iterate before moving to another node. Keep PNG/JSON local, then commit/push focused changes to PR #23.


Rack validation: 540 app GPUI tests passed, zero failed, three ignored; targeted Rack regression 64 passed. Native capture completed 28 local PNGs / three manifests (eight leaf stages, empty/populated Rack, both densities), followed by a fresh 20-frame sweep after the Overview occlusion fix. No screenshot or JSON manifest is staged. Review-driven RED/GREEN checks cover logarithmic/feedback endpoints, draft loss across selection/page changes, and rapid Bypass/Mono/Swap clicks. Geometry tests cover actual Input bounds and dial centers at 0.85 / 1.0 / 1.25 zoom. Service changes validate real multi-route ports and retain route membership on clear. All capture observations are backed by the running test engine, not physical-device verification.

Focused commits: `e29b2b3` service contracts, `2e25355` Rack card/Inspector editor, `b5edddc` minimap occlusion and `e369d48` native capture acceptance. The broader redesign remains active: real spectrum observation, Tone / independent Mapper consistency and complete non-Rack Inspector precision input are not claimed finished.

Final checks: default and gpui-test workspace/all-targets strict Clippy, rustfmt and diff checks passed. Shared Input/Gain/EQ/Compressor fields were recaptured in 16 local PNGs/four manifests; these are separate from the 28 Rack frames. Overview test relocation is `dacc302`; its targeted regression passed after relocation.

## Spectrum Workstation (2026-10-28)

Complete this node before moving to another card. Use the existing engine Hann-window FFT publication, not a synthetic trace or another FFT on the UI thread. Retain explicit stopped, waiting and stale states; current candidate settings are not evidence of active analyzer settings.

- [x] Relay active, demand-gated engine spectra through capacity/owner-aware application topics; test source peak, shape changes, restarts and demand release.
- [x] Add presentation subscriptions with freshness and identity fences. Keep all preparation/copying off the audio thread and bound maximum-channel payloads.
- [x] Replace the placeholder with a log-frequency magnitude plot and real per-channel selection, peak/bin readout, FFT/hop choices and measured resolution. Use the same controls in Inspector; merge settings edits against the latest candidate.
- [x] Capture real tone spectra and stopped/active/settings-change states at both densities; inspect plot labels, control geometry and adaptive meter layout before acceptance.
- [x] Run focused and integrated tests, strict Clippy/fmt, then atomic commits/push/update PR #23. Keep every generated PNG/JSON local and do not merge. Spectrum commits `93aa416` through `24885a4` are pushed; PR #23 remains open.

Verification: 577 app GPUI tests passed, zero failed, three ignored; 351 engine tests passed across eight suites. Default and gpui-test workspace/all-targets strict Clippy, rustfmt and diff checks passed. The final native run completed 16 frames, with every vertical-meter check true and exact FFT/hop fields inside Inspector bounds. Real 937.5 Hz stereo input at 48 kHz produced the expected -20 dBFS peak amplitude. The first full app run exposed old fixed capture grouping; width-aware grouping fixed that failure without shrinking the viewport or weakening bounds.

Review regressions cover invalid-input recovery, queued FFT/hop edits, fast preset reversal and stale-source fences. This completes real Spectrum observation, superseding earlier pending-FFT notes. Tone / independent Mapper domain consistency and remaining Inspector precision input are still open.

## Remaining Precision Workstations

- [x] Tone: share exact frequency validation with the existing service domain, separate it from ergonomic knob travel, provide an independent Inspector input, preserve dirty/error state and keyboard ownership; capture card and opened Inspector in both densities before advancing. Six native frames pass geometry/visual review; 18 Tone-related tests pass, including three RED/GREEN queue/rebinding regressions. Commits `79814bc`, `75aed33`.
- [x] Independent Mapper: finite signed exact weights with bounded 4x4 paging on both surfaces, real topology-based presets, field-level service merge/undo, separate surface drafts; capture stereo and paged multichannel cases. Twelve native tests pass, including loaded-document/history/collapse lifecycle regressions. Twenty native frames captured and visually reviewed; commits `b5a05e6`, `741ad9d` pushed. Fixed ruler/CLIP scrolling and common vertical verification subsequently landed in `79718ba` / `91c40a8`, with fresh captures.
- [x] EQ / Gate / Limiter / Delay: Inspector precision fields share card parameter contracts without sharing mounted input entities. Preserve percent conversions and prepared capacities; verify one node at a time with native captures.
- [x] Meter / Recorder: exact integer Inspector settings retain existing engine validation and storage/preparation contracts; capture each independently. Integer regression: 19 passed; Meter's prior four frames and Recorder's fresh four frames accepted locally.
- [x] Final review, integrated tests and per-responsibility commits/push/PR update. No screenshots or generated manifests in Git; no merge. Remote CI may still be running and is not represented as local verification.

Prerequisites complete: latest-candidate scalar/Mapper field intents and per-field undo (`71087ce`, 12 tests); nonallocating recorder admission (`86aa822`, 43 recorder-related engine tests); confirmed-window meter freshness (`470310f`, five tests including real topic delivery). The precision UI increments and focused lifecycle acceptance are recorded below. Final full-suite/Clippy gates will be repeated on the integrated result.

Mapper acceptance adds 16 real 32-channel frames, including native signed input on both surfaces, paging to channel 32, fast presets and horizontal meter scrolling; four additional stereo frames verify the smaller intrinsic card. Capture geometry measures stereo at 440 x 256 logical pixels and 32-channel at 516 x 382, with meter heights 187.2 and 313.6 respectively. Generated images/manifests remain ignored and untracked. Generic capture manifests initially omitted vertical verification; this was fixed in `91c40a8` and the generic frames repeated. The dedicated 32-channel scenario verifies all 16 frames.

At the initial EQ checkpoint, four native layout frames and seven precision tests passed. Review then exposed queued wheel increments, band-drag undo granularity, pending-load callbacks and logarithmic maximum endpoints; the subsequent EQ acceptance fixes these. Gate / Limiter / Delay and Meter / Recorder precision editors were completed in the following increments.

### Precision progress after Gate

- EQ accepted: 14 focused tests and four fresh native frames; queued Q, compound band undo, pending-load fences and gesture endpoints fixed. `2511a8e`, `1b77844`.
- Gate accepted: seven focused tests and four native frames. Screenshot found wrapped ms units; mono/no-wrap fix recaptured successfully. `2d642e4`, `c5921de`.
- Node meter gutter fixed and recaptured: Mapper16+stereo4 retain ruler/CLIP at both scroll endpoints; three geometry tests pass. `79718ba`.
- Common scalar capture has five tests, side-meter verification two and pixel-boundary tolerance two; EQ/Gate/stereo Mapper vertical flags now all true. `91c40a8`.
- At this checkpoint Limiter / Delay and Meter / Recorder precision inputs were still pending; subsequent increments complete them. No generated artifacts are tracked; PR #23 remains open, not merged.


### Precision progress after Delay

- Limiter and Delay accepted locally: five focused tests and four native frames each, including Compact Inspector and Comfortable card visual review. Commits `7005ad3`, `8d9dbd9` pushed.
- Meter baseline: seven focused tests and four native frames pass; Recorder exact capacity hookup subsequently passes its independent screenshot gate.
- Review-driven completion gates added and subsequently accepted below: node rotary pending/load/lifetime acknowledgement, cross-surface explicit retyping intent, legacy Gain/Pan/strip focus and untouched precision, Compressor draft/queue lifecycle, connected Bus removal affordance. These are concrete interaction regressions, not new DSP features.
- Integrated tests and final PR validation remain pending. No generated screenshot/JSON/WAV is staged, and PR #23 remains open without merge.

### Integer and scalar lifecycle acceptance

- Integer regression: 19 passed. Meter accepts exact 1..=1,048,576 frames; Recorder capacity uses the actual-channel engine limit and rejects busy phases. Pending-load/entity fences, retries, explicit baseline reversal and redundant Change notifications are covered.
- Scalar intent: 12 passed; rotary lifecycle: 10 passed; legacy Inspector: 14 passed. Coverage includes cross-surface intent, untouched precision, queued/partially acknowledged edits, stale callbacks, Compressor drafts and connected Bus removal admission.
- Removed approximately 600 lines of obsolete `node_inspector/builtin/` implementation; existing responsibility-based editors retain the parameter entry points.
- Local native acceptance: Recorder four fresh frames, Meter four previously accepted frames, final Gain/Balance/Compressor/Bus four frames each, and `meter32` six frames. PNGs and generated manifests remain local only; prior Meter captures are not a fresh rerun.
- Final review, integrated full-suite/Clippy results and CI completion remain pending. These focused results do not close the final checklist item; the main worker will add integrated verification separately.

### Final native acceptance

- Fresh Meter stereo four-frame and 32-channel six-frame captures pass all native bounds and vertical-meter checks. The validator now compares Meter with its observed readings/status sibling below presets; 20 capture regressions pass. Commit `13dfa1a`.
- Library-created EQ / Mapper use real native precision inputs in integration tests. A true 1024 x 720 Compressor test scrolls to Mix and Delete, edits only Mix and preserves graph/camera/layout. All three targeted tests pass; the resize callback is required to update the test viewport. Commit `6d73736`.
- Local evidence inventory: 262 PNGs / 50 manifests across baseline and follow-up runs, every referenced image present. These are cumulative, not a claim that all frames were recaptured together. No generated artifacts are staged or tracked by this PR.

### Queued-interaction final review

- Reproduced five native failures before fixing stale whole-strip edits and Pan/Mute double-click intent. Seven native tests now pass, including Inspector precision, wheel accumulation and mode/toggle composition without publication.
- Added service-local `MixerEdit` and Pan toggles, accepted-state RT mapping and independent undo keys. Real audio regression exposed and fixed empty-cache Solo fan-out using pre/post transaction effective configs. Twelve service tests pass; no parameter-triggered rebuild is introduced. Commits `dda5241` / `02129e3`.
- Linux CI at `6d73736` exposed an existing repeated-Apply test's transient-barrier race. `643df62` synchronizes parameter submission with round budgets and an unsent tail; no increased timeout or production ACK change. Distinct Apply IDs, superseded receipts, pending topology, final confirmation and single-start assertions remain.
- The earlier final Windows gate run (721 GPUI tests, 1,429 default workspace tests and both strict Clippy configurations) predates these review fixes. The complete gates are rerun on the new integrated tree; do not reuse those counts as its final result.

## Final Integrated Verification (2026-10-28)

Final production/test commit: `643df62`; subsequent changes only reconcile documentation. All local gates completed sequentially with exit zero:

- App GPUI library: **740 passed, 0 failed, 3 ignored**.
- Default workspace: **47 suites, 1,441 passed, 0 failed, 6 ignored**, including clock/fault/startup matrices and doctests.
- Default and GPUI-feature workspace/all-targets strict Clippy: passed with `-D warnings`.
- Rustfmt, working-tree and complete PR whitespace checks: passed.
- Final repeated-Apply test additionally passed 20 consecutive executions. Five failing native queued-input regressions were reproduced before their fix; the final seven-test group and twelve service-intent tests pass within the integrated suites.
- All 15 kinds / 17 entry variants have one-node capture/review evidence; the latest Meter ten-frame rerun verifies adaptive meter bounds. All 262 cumulative PNGs and 50 manifests stay local, with zero new generated artifacts anywhere in the PR commit history.

PR #23 is open and unmerged. Remote CI is still running when this record is written; its result must be read from PR checks rather than inferred from local success. Hardware/native-only ignored tests remain explicitly unexecuted. No further node redesign work is pending in this plan; future DSP features and physical-device validation are outside this delivery.
