# Common physical render request implementation plan

> **For agentic workers:** Use test-driven-development and scoped review. Steps use checkbox syntax.

**Goal:** Complete S1's common physical render mode/period controls in the ordinary Process, owned Process, and VAD physical-output probe paths.

**Architecture:** A portable control-side `PhysicalRenderRequest` retains explicit intent and applies only mode/period to each caller's existing DeviceOptions. The actual WASAPI owner and VAD host admission validate observed timing before source activation. Requests and negotiated evidence stay separate; this is not native qualification.

**Tech Stack:** Existing Rust workspace, serde, WASAPI, EngineHost; no new dependency.

## Global constraints

- No VM, native audio, driver installation, deployment or push in this slice.
- Preserve omitted defaults, graph/clock roles, existing native format choices and RT contracts.
- `--render-mode shared|exclusive`; `--render-period-frames N` accepts only engine BUFFER_SIZES (64 through 4096 powers of two).
- Reject duplicate/invalid controls, Exclusive target queues and physical controls on the virtual microphone route before artifact creation.
- Shared explicit period requires observed Client3 exact acceptance without compatibility fallback/conversion; Exclusive explicit period requires the actual exact buffer/period. Unknown or mismatched observations refuse activation.
- Keep requested configuration in report/manifest even on preparation failure; do not turn configuration or dry-run into delay evidence.
- Preserve unrelated gate.log. Atomic local commits; remote updates remain batched.

## Task 1: Portable request and parser contract

Files: create `crates/moiren-windows-audio/examples/e2e_latency_probe/render_request.rs`; modify `live.rs` module/parser/defaults/validation and report/manifest, `cli.rs` help.

Interface: `PhysicalRenderRequest { mode: Option<DeviceMode>, period_frames: Option<u32> }`, `validate(queue) -> Result<()>`, `apply(DeviceOptions) -> DeviceOptions`, `validate_observed(NativeStreamTiming, Option<SharedPeriodReport>) -> Result<()>`. Absent mode means the existing Shared default; absent period remains native default. Use serde snake_case DeviceMode.

- [x] Add parser/dry-run tests using real run/parse/validate: both source kinds, omitted defaults, malformed/duplicate/bounds, Exclusive target conflict, microphone rejection, retained request in report and manifest.
- [x] Run `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe physical_render -- --nocapture` with host target cache; record behavioral RED before parser implementation.
- [x] Add helper tests first: apply preserves backend/clock/rate/format, supported periods, queue conflict, exact/unknown/mismatch/fallback/conversion observations. Record RED then implement minimal helper.
- [x] Implement CLI and JSON fields; validate before artifact creation, keep physical request absent for microphone route. Shared/omitted-period preserves existing native compatibility semantics.
- [x] Repeat focused tests until GREEN (9 focused tests; parser/helper behavioral RED logs retained).

## Task 2: Actual owner and host wiring

Files: `render/owner.rs`, `live_native.rs`, `process_collection/graph.rs`, and portable `vad_route.rs` if needed for a testable physical project helper.

- [x] Extend RenderOptions with the typed request; both Process callers pass the same LiveOptions field.
- [x] Owner report retains request before open; open applies it to the existing Shared Float32 DeviceOptions. Retain native timing/shared_period before exact acceptance check, then configure queue and send ready.
- [x] VAD physical output applies the identical request to its physical sink while preserving Master and Native format. Do not apply physical options to virtual IO.
- [x] After host running but before opening VAD gates, inspect exactly one physical render observation, retain it on failure, and validate actual timing/shared_period. Missing/ambiguous/wrong observation refuses source activation; use existing cleanup.
- [x] Add regression tests for actual options/project preparation and preparation refusal evidence without opening native audio. Record focused RED then GREEN. Independent review found live/final overwrite; immutable admission snapshot plus actual validation verdict/failure now survives changed/null later observations, with separate behavioral RED/GREEN coverage.

## Task 3: Verification, review and local receipt

- [x] Run full probe tests (442 passed), audio regression (831 passed / 3 ignored), audio all-target strict Clippy and test-profile Clippy, workspace fmt/diff checks, and Linux compile-only checks using established cache/log paths.
- [x] Have independent reviewer inspect the complete scoped diff and actual caller seams; fix concrete findings and rerun covering tests. Snapshot-overwrite P2 fixed and independently re-reviewed clean.
- [x] Commit exact source paths once verified (`32b537d`); update software ledger with fresh counts and native limitations, then commit docs separately. Receipt accuracy independently reviewed; no push or native operations.

## Completion boundary

This slice closes the common request forwarding/refusal software gap only. S1's physical-input path and target/native/waveform evidence, S3 density/freshness provenance and stable sub-10-ms qualification remain open.
