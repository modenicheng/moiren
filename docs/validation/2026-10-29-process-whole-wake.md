# Whole-native-wake Process admission regression

2026-10-29 (Asia/Shanghai). Host-only regression slice, implementation receipt
`305591d`, pushed to `origin/feat/minimal-io-latency` and remote-SHA verified.
No production behavior/default/buffer change, native audio execution, VM,
deployment or driver build/install. Unrelated `gate.log` remains untouched.

## Audit correction

The prior remaining-work list mentioned whole-native-wake admission versus
segmented Process reads. Inspection finds the production protection already
present; this is a verification/documentation gap, not a missing functional
implementation:

- Backend preparation uses the negotiated native render target/capacity,
  converts to Engine rate with ceiling, and adds 64 frames plus two Engine
  blocks. Actual Process preparation revalidates against this complete demand.
- The actual master Pump sums every Engine block rendered for one native
  demand and publishes one aggregate consumer event. Partial writes neither
  rerender that demand nor publish extra consumption events.
- Measured reuse preserves both prepared and observed demand maxima, rebinds
  them upward between rates and validates the replacement's complete demand.
- ClockSource queue histories contain segmented ProcessSource reads. They
  remain replay evidence only, never converted into measured admission proof.
  Replay is still `qualified: false`.

Source anchors: `crates/moiren-windows-audio/src/engine_backend.rs`,
`crates/moiren-windows-audio/src/engine_backend/owner.rs`,
`crates/moiren-windows-audio/src/process_loopback/measured.rs`,
`crates/moiren-windows-audio/src/engine_backend/process_admission.rs`, and
`crates/moiren-windows-audio/src/process_loopback/replay.rs`.
Existing real preparation/refusal tests remain in
`engine_backend/measured_preparation_tests.rs` and `process_admission/tests.rs`.

## Actual software path exercised

The new isolated RT regression compiles the actual Capture bridge ->
ProcessSource -> Bus -> RingSink graph and runs it through the actual master
Pump with an injected NativeStream. Eight combinations cover f32/f64, 48/96 kHz
Engine rates and 64/512-frame blocks. The source scratch is 256 frames, native
demand 480 frames and partial acceptance 73 frames. Three demands complete
through 21 software Pump calls. These are synchronous modeled calls, not OS
wakes or a physical latency test.

The allocation/deallocation guard encloses actual Pump/Engine/source execution.
Stack-only timeline endpoints establish each independent native-demand delta.
After processing, each aggregate event must equal its corresponding delta and
the sum of its associated segmented source reads. Partial writes must create
exactly three demand events, native accepted frames must total 1440, queue
priming/underrun must remain zero, and every aggregate must fit the reconstructed
prepared bound. The bound is reconstructed from the inspected formula here;
actual backend preparation is covered by the separate existing tests above.

Only the following admission-arithmetic portion uses synthetic timestamps and
producer deliveries. It retains the actual Pump demand frame aggregates but
does not claim those synthetic clocks are native evidence. A 384-frame target
fits a 256-frame segment plus the fixture's 48-frame supply gap, yet refuses
the whole-wake requirement. A larger 4096 target preserves the full prepared
bound. Independent evidence with a deliberately small prepared estimate of
256 still preserves the observed whole-wake maximum and refuses the small
target. Neither candidate changes production defaults or becomes a preset.

## Verification

| Command | Final result |
| --- | --- |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml process_wake::whole_native_wake_survives_process_read_segmentation_in_both_precisions -- --exact --nocapture` | 1 passed, all 8 modeled combinations |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 135 passed, 0 failed |
| `cargo clippy --locked --manifest-path tools/rt-check/Cargo.toml --all-targets -- -D warnings` | Exit 0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets` | Exit 0; cross-check only, Windows-only ProcessSource test excluded |
| Root fmt, scoped RT rustfmt, whitespace and restored production-file diff | Exit 0 |

To prove the regression detects the intended error, a temporary mutation caps
the actual aggregate publication at 256 frames. The test fails with published
768 versus actual Engine 1472 frames. A `finally` restores the original source
byte-for-byte; `git diff --exit-code` confirms no production-file change before
the final full suite. The mutation is not committed. An initial exact filter
omits the module path and runs zero tests; the corrected full path runs one.
That zero-test attempt is not counted as validation.

Logs: `target/s1-s3-20261028/s3-whole-wake-*`. Independent read-only review finds
no remaining Important/Critical issue after strengthening per-wake association
and independently testing the observed maximum. No native timing, Linux
execution, remote CI or Miri result is claimed.

## Remaining gates

Whole-wake reserve protection is implemented and now exercised end-to-end in
the real software components; the earlier list must not imply it is absent.
Segmented ordered replay still cannot establish native wake grouping, sustained
feasibility, future-safe admission, signal continuity or source sample age.
These remain distinct from the complete-demand preparation contract.

The full S1-S3 goal stays active. Sample-age provenance, sustained feasibility,
actual lower native negotiation, matched physical routes and stable sub-10-ms
output remain open/hardware-deferred. Correctness/latency-sensitive policies
and configurable 0..3000 ms extra progress grace are unchanged. No VM timing
retry or production buffer enlargement is introduced.
