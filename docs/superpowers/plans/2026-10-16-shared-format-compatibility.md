# Shared Format Compatibility Implementation Plan

**Goal:** Allow the default Realtek microphone array and endpoints with other
sample rates/PCM depths to work without changing Windows device settings.

**Approved scope:** Keep the graph at 48 kHz stereo f32. Retain supported native
render/capture formats; otherwise request Windows Shared conversion to 48 kHz
stereo f32. Do not change Exclusive mode or the existing process capture path.

**Architecture:** Prepare and validate owned mix descriptors on the COM owner.
Use Client3 only for the existing native path. Conversion uses legacy Shared
initialization with AUTOCONVERTPCM and SRC_DEFAULT_QUALITY. Driver packet layout
always matches the selected client descriptor; processing performs no new
allocation, blocking, or software conversion.

## Tasks

- [x] Add failing tests for PCM16/24/32, 44.1/96/192 kHz, four-channel capture,
  malformed descriptors, native preservation, conversion flags, and errors.
- [x] Implement descriptor selection in `src/shared_stream/format.rs` and shared
  initialization in `src/shared_stream/wasapi.rs`; connect render and capture.
- [x] Record mix/client descriptors and conversion in `SharedPeriodReport`.
  Normalize engine period frames to client-rate frames with checked ceiling
  arithmetic. Do not claim Client3 low-latency support on conversion streams.
- [x] Keep explicit hardware-rate requirements tied to the endpoint mix rate,
  not the converted client rate. Adjust endpoint discovery and CLI documentation
  so untested conversion is not reported as confirmed native support.
- [x] Run focused tests, crate tests, workspace checks/Clippy/fmt, then zero-gain
  finite Realtek/96 kHz tests. Compare default endpoint, volume and mute snapshots
  before and after. Do not save captured PCM.

## Verification

`cargo test --locked -p moiren-windows-audio --lib`

`cargo test --locked -p moiren-windows-audio`

`cargo check --locked --workspace --all-targets`

`cargo clippy --locked --workspace --all-targets -- -D warnings`

`cargo fmt --all -- --check`

Preserve the pre-existing Exclusive probe, README/lib edits and graph documents.
No commits, pushes, device reconfiguration, or audible testing are requested.

## Integration Findings

- Converted capture still receives endpoint-rate device positions, even when
  packet data is at 48 kHz. Do not compare native positions against client
  frame counts. Disable position-derived gaps, age and rate for cross-rate
  streams; preserve driver discontinuity flags and inventory-servo correction.
  Regression coverage exercises both immediate and deferred capture setup,
  and fails with the previous per-packet false-discontinuity behavior.
- Windows main-thread cleanup overflowed when every nested report embedded a
  complete fixed trace array. Keep real-time recorder storage fixed, but own
  only valid snapshot entries in a boxed slice after Stop. Tests bound report
  sizes and retain zero-allocation/deallocation coverage during processing.
- Workspace validation exposed a pre-existing race in two UI signal tests:
  `try_snapshot()` may be busy during publication. A bounded test-only wait
  replaces unconditional unwraps; the service API remains non-blocking.
  Thirty repeated runs of the signal-visibility binary passed after the fix.
- Linux strict Clippy exposed pre-existing unused/non-idiomatic QPC fallbacks.
  Only Windows builds construct QPC clocks; portable `now()` remains unknown.

## Hardware Verification

All runs used application gain zero or independent output zero-gain guards.
Only aggregate JSON diagnostics were saved, never microphone PCM.

| Run | Result |
| --- | --- |
| Realtek four-channel 48 kHz capture to Realtek stereo render, 2 s | Capture converted to stereo; 95,520 captured / 93,344 output frames; zero resets, drops and underruns; both Stop calls succeeded |
| VB CABLE 96 kHz capture/render, 2 s | Both converted to 48 kHz; 95,489 captured / 93,600 output frames; zero resets, drops and underruns; both Stop calls succeeded |
| VB CABLE 96 kHz capture/render with Trace, 2 s | 200 capture and 199 render trace records; 93,120 output frames; zero resets, drops and underruns; both Stop calls succeeded |
| Realtek master, CABLE follower, both physical inputs, Compatible | Healthy; compatible graph edit activated; clean shutdown |
| CABLE 96 kHz master/input plus Realtek follower, Balanced, 2 s | Healthy; compatible graph edit activated on same owner/epoch; zero bridge resets, drops and underruns; clean shutdown |
| Standalone CABLE 96 kHz render, 1 s | Completed; raw engine period 960 frames, normalized client period 480; Stop succeeded |
| Default-device desktop service automatic startup | Reached Running without a UI Start request, gain zero; reached Exited after requested exit |

Live endpoint mix formats available here were float32. PCM16/24/32,
valid-bit/container combinations, float64 and additional sample rates have
descriptor/fake-client regression coverage, not a claim of physical hardware
validation. Mix bit depth is not proof of the DAC/ADC hardware's bit depth.

Evidence is under `target/shared-format-check/`, especially
`realtek-clock-final.json`, `cable96-clock-final.json`,
`cable96-trace-final.json`, `routing-clock-final.json`, and
`device-state-comparison-final.json`.

The five original endpoints retained their mix formats, volumes and mute
states. Default console/multimedia capture and render remained Realtek.
A display-audio endpoint appeared during verification, and the system's
communications render default changed from CABLE In 16ch to that endpoint.
No implementation/test command calls device-setting setters, and no attempt
was made to undo the external topology/default change.

## Validation Status

- Passed Windows workspace all-target `check` and strict Clippy.
- Passed audio-crate all-target strict Linux cross-target Clippy.
- Passed 119 audio unit tests and 10 latency tests.
- Passed the final `cargo test --locked -p moiren-app --features gpui-test`,
  including all six UI acceptance tests.
- Passed final `cargo build --locked --workspace`.
- Passed final `cargo test --locked --workspace` after the capture-clock
  correction and portability cleanup (exit 0); evidence is
  `target/shared-format-check/workspace-test-accepted.log`.
- Passed rustfmt and `git diff --check`.
