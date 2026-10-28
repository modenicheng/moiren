# Playback native sequence gaps: software evidence

Recorded 2026-10-29 (Asia/Shanghai). Continues S1-S3 independently of VM timing.
Implementation `df6ea7e` and diagnostics `011289b` are separately committed,
pushed and remote-SHA verified. No native audio/VM run, deployment, driver
build/install or package change occurred. Production buffers/defaults, strict
mode default and the sub-10-ms target are unchanged.

## Observed sequence provenance

Playback READ returns the driver's native packet cursor. The driver advances
that cursor even when a render packet was not ready; it may also have no
current request to serve. Therefore later accepted completions can have a
forward hole. A short wall-time completion interval, especially after batching,
does not prove an uninterrupted native packet sequence.

Microphone WRITE completion is different: it returns the host's submitted
request stamp. Treating its gaps as missing native supply would be false.
Source anchors are `driver/vad/src/audio/Streams.cpp`,
`driver/vad/src/Pcm.cpp`, `crates/moiren-windows-audio/src/vad/pcm.rs`, and
`crates/moiren-windows-audio/src/vad/provider/pcm_worker.rs`. The driver is
inspected, not changed by this slice.

## Implemented contract

`PacketOrder` now returns typed First/Contiguous/Gap/Stale outcomes within an
explicit reset epoch. A first observed packet establishes a frontier; its
unobserved prefix is not counted. Increasing sequence with equal QPC is valid.
A duplicate/regressed sequence or regressed QPC does not advance the accepted
frontier. Forward-gap arithmetic handles the entire u64 domain without wrap.
A new epoch establishes a fresh frontier even when sequence/QPC restart.

Playback rechecks epoch readiness after completion and before publishing its
order/accounting. Existing two-way quiescence still fences a reset racing
later PCM publication; the new check is not claimed as an atomic global
snapshot against all subsequent control operations. Existing carrier-overflow
recovery remains unchanged. A forward gap alone never resets or stops audio.
Microphone keeps its existing stale-order refusal without counting native gaps.

Telemetry schema3 adds two independent saturating u64 fields:

- `sequence_gap_observations`: accepted playback forward-hole observations.
- `missing_sequence_slots`: total unobserved native sequence slots between
  accepted playback predecessors and successors.

Neither increments `dropped_packets`, driver/quota/carrier drops, silent
frames, epoch resets or recovery events. Missing slots do not establish
non-silent sample loss, source sample age or physical latency, and no frames
or milliseconds are manufactured from them. Zero adds no observation. JSON
keeps legacy aliases/defaults, upgrades older/missing schema versions to 3,
preserves future versions and exact integers above 2^53.

The real native health path and summary quality projection include missing
slots once as delivery-quality evidence, not as dropped frames. Gap-event
count is not summed again. Correctness mode records the raw observation and
continues with failed quality; latency-sensitive mode rejects the same fault.
This reuses the existing policy rather than forcing resets, relaxing hard
lifecycle errors or changing collection deadlines.

Desktop export diagnostics show exact counters only for Playback, explicitly
label unobserved slots and the unknown first prefix, and state that they do
not prove signal loss. Zero counters are not a continuity pass. Microphone
diagnostics do not display a misleading native playback gap row.

## Test-first and final verification

Host-only software commands use `CARGO_TARGET_DIR=D:\coding\moiren\target`.
Logs are in the worktree's `target/s1-s3-20261028/s2-vad-sequence-*.log`.
Overlapping focused and full-suite counts are not additive.

- Worker RED: 3 failing assertions and 2 passing tests before gap wiring;
  typed-order API addition then produces the expected missing-API compile
  failure. Final worker GREEN: all 6 pass. Coverage includes unknown prefix,
  batched forward holes, continued PCM transfer, no invented drops/resets,
  stale-frontier preservation, same order across a changed epoch, reset before
  publication, microphone provenance and u64 boundary arithmetic.
- Telemetry RED: 4 passed/4 failed before implementation; GREEN: all 8 pass.
  Covers no-op zero, saturation, independent counts, old/missing/future schema
  and exact large-integer serialization.
- UI RED: 10 passed/3 failed before the row and schema expectation fix;
  GREEN: all 14 pass, including a subsequent microphone-scope regression.
  The red log also retains an intermediate E0063 fixture compile failure.
- Probe RED: the missing-slot quality assertion fails; the paired mode
  regression then fails too. Final full probe: all 261 pass, including the same
  raw gap continuing correctness but refusing strict mode without invented
  dropped frames.
- Initial full-app compile found an explicit telemetry literal missing the
  two new fields in `service/exports/runtime/diagnostics_tests.rs`. It now
  includes nonzero large counters, so existing observation-retention tests
  also carry them; the final full run passes. Failure log is preserved.

| Final command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 467 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 261 passed |
| `cargo test --locked -p moiren-app --lib --features gpui-test -- --test-threads=4` | 456 passed, 3 ignored |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 100 passed |
| `cargo test --locked -p moiren-app --target x86_64-pc-windows-msvc --test ui_acceptance --features gpui-test provider_diagnostics_render_last_observations_and_missing_evidence -- --test-threads=1` | 1 passed, 12 filtered out |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit 0 |
| `cargo check --locked -p moiren-app --all-targets --features gpui-test` | Exit 0 |
| `cargo fmt --all -- --check`; `git diff --check` | Exit 0 |

Independent read-only worker, telemetry and probe review found no remaining
Important/Critical issues in this bounded slice. No Windows driver validation,
Linux tests/CI, Miri, physical continuity or latency result is claimed.

## Remaining gates

This closes the missing playback-sequence-slot observability gap identified
by the preceding inventory audit, not S1-S3 as a whole. Real per-frame VAD
timestamp provenance, Process source sample age, truthful alternate-target
finite-window queue feasibility, actual lower native-period negotiation,
matched physical-route timing and sustained stable sub-10-ms output remain
OPEN/hardware-deferred. VM timing is not retried as a development gate.
