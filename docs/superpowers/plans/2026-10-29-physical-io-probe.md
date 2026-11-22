# Physical IO Bus probe implementation plan

> **For agentic workers:** Use test-driven-development, scoped subagent implementation and independent review. No native audio or remote push in this slice.

**Goal:** Implement S1's actual supported physical input -> Bus -> physical output probe with bounded independent measurement capture, not a standalone recorder mistaken for a routed graph.

**Architecture:** Add a separate `live-physical` entry under the existing live module, sharing physical mode/period intent and validation policy. The production ProfessionalBackend/EngineHost owns the exact three-node graph. An independent MeasurementSession starts first; immutable admission plus pinned final native/professional snapshots and bounded raw PCM/packet files survive faults and cleanup. External changing signal and physical/common-reference wiring are caller responsibilities, never inferred.

**Tech Stack:** Existing Rust workspace, EngineHost, ProfessionalBackend, MeasurementSession and serde. No new dependencies or driver changes.

## Global constraints

- No native audio, VM, driver install/deployment, host driver install or push.
- Preserve existing Process/VAD probe modes/defaults; new command always unqualified.
- Distinct exact nonempty input, output and recorder endpoint IDs; no default-device selection or name matching. No path overwrite; new artifact directory only.
- Exactly physical Input(Follower) -> Bus -> physical Output(Master), stereo processing at 48 kHz, no gain/tone/hidden output. Preserve actual native rates/formats and explicit request versus acceptance evidence.
- Shared/default mode and native/default period when omitted. Capture and render mode/period independent. Only 64..4096 power-of-two explicit periods; explicit period refuses compatibility fallback, missing/mismatched actual observations.
- Compatible input budget by default. Explicit target/capacity/jitter are validated and forwarded to actual backend LatencyBudgetPolicy; no synthetic balancing/fallback preset.
- `--seconds` is requested steady collection duration (1..600); startup is bounded separately by one 10-second window plus correctness grace, anchored before preparation, with an overall fixed ceiling. Grace is 0..3000 ms additional progress allowance, not audio delay, queue expansion or indefinite test continuation.
- Correctness retains timing/quality faults and continues while operational; latency-sensitive aborts those faults. Identity/generation/schema/storage/native failure and failed teardown remain hard errors for both policies.
- Recorder native join before sink finish; all teardown steps attempted after failures/panics. Pin exact observation handles before host drop; closed/running flags alone do not prove teardown.
- Do not invent source timestamps/sample age, calibration, physical route, independent reference or stable sub-10-ms outcome. Source is externally supplied, not owned/launched.
- Atomic local source/docs commits, batch remote checkpoint later; leave unrelated gate.log unchanged.

## Task 1: CLI contract and topology

Files: new `examples/e2e_latency_probe/physical.rs`, `physical/topology.rs` under moiren-windows-audio; module declaration in `live.rs`; dispatch/help in `cli.rs`.

CLI: `live-physical --input ID --output ID --recorder-config FILE --artifact-dir NEWDIR --seconds 1..600 [--capture-mode shared|exclusive --capture-period-frames N] [--render-mode shared|exclusive --render-period-frames N] [--input-target-frames N --input-capacity-frames N --input-jitter-frames N] [--render-target-frames N|capacity --wake-margin-frames N --callback-budget-frames N] [--engine-block-frames 1..4096] [--validation-mode latency-sensitive|correctness --jitter-tolerance-ms 0..3000] [--dry-run]`.

Recorder config schema1 denies unknown fields and requires input_endpoint_id, ring_frames, max_pcm_bytes, max_metadata_bytes, max_packets; <=64KiB JSON, existing MeasurementConfig limits. Preserve original bytes in recorder-config.json; validate source/output/config path aliases and all values before creating artifacts.

- [x] Add actual command/parser/dry-run tests first: missing/duplicate/invalid inputs, request and queue conflicts, mode separation, config bounds, existing output/path alias, no native evidence/qualified claim; observe RED.
- [x] Implement portable Options/RecorderConfig/read_config validation, new-directory files manifest.json/report.json/config copy using create_new, no native on dry-run. Every operational failure after directory reservation still writes a failure report.
- [x] Add topology test first then build/validate exact three-node graph; ensure options only apply to intended direction and clocks/rate/Bus remain intact.
- [x] Run focused physical tests GREEN before handing Options interfaces to session/native implementer. Initial parser/topology RED ->6GREEN; review found option-token swallowing and impossible jitter, fixed with runtime8PASS/2FAIL ->10PASS. Checked-add minimum-transfer/FIR boundary regression had genuine RED ->12GREEN; collector panic receipt regression had genuine RED ->13GREEN. Independent Task1 re-review found no remaining concrete finding. Native execution is not verified by these tests.

## Task 2: Actual orchestration, health and cleanup

Files: new `physical/session.rs` and `physical/native.rs`; narrowly expose existing Process collection's exact recorder outcome validator and diagnostics for sibling reuse rather than duplicate its quality reconciliation.

Interfaces: physical Options/RecorderConfig are parent-visible; `run_native(options: &Options, config: &RecorderConfig, directory: &Path) -> Value` returns complete structured result even on operational failure. Parent report merges this object without fabricating native records. Portable controller uses injectable actual graph/recorder operations and monotonic elapsed observations for native-free lifecycle tests.

- [x] Test recorder-start-before-graph, bounded startup/steady duration, current generation/exact nodes/accepted periods, loss of progress with policy grace, quality vs operational failure, no-generation adoption; record RED.
- [x] Test final pinned observation validation and independent cleanup attempts after prepare/run/stop/join failure or unwind; preserve admission/first failure/final reports separately.
- [x] Implement real recorder adapter and production graph adapter only; apply options to actual backend, retain raw native/professional observations plus actual recorder info. No separate custom PCM bypass.
- [x] Request recorder stop before synchronous host drop/join (host has no separate asynchronous stop API); retain exact handles and verify Closed/no fault/cleanup_error and same generation; finish recorder native/sink even if graph teardown fails. Final checked deltas are visible, no failure converted to zero.
- [x] Run focused native-free tests GREEN; report source scope and RED/GREEN commands. Final portable session15 plus native projection1 passed; exact stage/terminal/first-quality regressions and Linux portability repair are recorded in the slice receipt.

## Task 3: Verification, review and receipt

- [x] Full probe suite471PASS, audio regression831PASS/3ignored, strict audio all-target/test-profile Clippy, fmt/diff; Linux compile-only default/test profile passed with existing cfg dead-code warnings1/4. Final logs in `target/s1-s3-20261028/physical-io-*`.
- [x] Independent spec/correctness/lifecycle review, fix concrete findings with regression evidence and re-review. Terminal interval and test-profile portability repairs passed scoped re-review; no remaining concrete finding.
- [x] Commit exact source paths as `f83f3a5`; fresh [receipt](../../validation/2026-10-29-physical-io-probe.md), S1-S3 ledger and this plan prepared for a separate docs commit. No push, native audio, VM or driver operation.

## Completion boundary

This closes physical IO orchestration software only. Actual supported endpoint acceptance, acquired changing waveform/common-reference calibration, S2 lower-setting qualification and S3 density/freshness provenance remain open until independently proven.

## Implementation rulings

- Ruling: validate exact graph-native periods after native activation, retire immediately on refusal - production EngineHost automatically activates its physical workers and exposes no parked-host gate - this does not establish pre-audio admission.
- Ruling: request recorder stop before synchronous host drop/join, then finish recorder/native/sink even on graph failure - EngineHost has no public separate asynchronous stop - do not describe this as broadcasting a nonexistent host stop API.
- Ruling: expose available quality counters with quality_complete=false and unavailable capture discontinuity/timestamp/silence counters as null - production professional observations do not propagate them - successful available-counter checks cannot imply complete waveform continuity.
- Ruling: outer artifact runner catches an escaped collector unwind and writes failure receipts with cleanup unverified - internal native supervision owns actual independent cleanup attempts - catching an unwind is not proof that teardown succeeded.
- Ruling: a terminal health observation must validate prior progress age before refreshing it - review reproduced both frozen and recovered counters escaping the final interval - bounded collection completion must not hide unobserved over-budget gaps.
- Ruling: project Windows backend observations into portable typed admission DTOs while preserving the complete raw snapshot and immutable metadata - Linux test-profile compilation exposed Windows-only imports in the controller - do not hide portable tests behind Windows cfg or drop evidence to compile.
