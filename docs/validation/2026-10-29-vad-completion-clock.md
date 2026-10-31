# VAD completion service clocks, not sample age

2026-10-29 (Asia/Shanghai). Host-only S1 diagnostic continuation. No native
audio execution, VM, deployment, driver build/install or production buffer
change. The full S1-S3 objective remains active.

## Atomic receipts

| Commit | Responsibility |
| --- | --- |
| `592a4e4` | Fixed completion clock summaries, actual worker wiring and RT regression |
| `f6ac8c4` | App diagnostics, exact integer presentation, legacy unknowns and translations |

Each commit was pushed to `origin/feat/minimal-io-latency` and its full SHA
compared with `git ls-remote`. Unrelated `gate.log` remains untracked/untouched.

## Timestamp provenance

The existing ABI stamps have different origins in the two directions:

- Playback READ carries the driver's transfer callback QPC. Event mode passes
  callback `now`; timer mode uses the callback `now` at logical packet completion.
  It excludes callback entry/State-lock delay before that stamp. The interval
  ends when the owner collects the completion, before order/decode handling.
- Microphone WRITE completion echoes the cached host submission QPC, including
  intentionally dropped WRITE completion. Its interval includes host copy/
  submission work. It is not the driver's transfer timestamp.

Source anchors: `driver/vad/src/audio/Streams.cpp`,
`driver/vad/src/audio/TimerPcm.h`, `driver/vad/src/Pcm.cpp`,
`crates/moiren-windows-audio/src/vad/pcm.rs`, and
`crates/moiren-windows-audio/src/vad/provider/pcm_worker.rs`.
These sources were inspected, not changed or rebuilt.

Neither stamp identifies the first source sample. No FIFO oldest-frame age,
physical latency, source continuity or successful-delivery count is inferred.
Stale playback and dropped microphone completions still have service intervals;
their existing loss/order diagnostics remain separate. No timing gate is added.

## Implementation

Preparation obtains QPC frequency once. Each collected completion reads raw
system QPC once, then subtracts raw ticks before checked u128 conversion. The
minimum is rounded down and maximum up to 100 ns units. These bounds cover
conversion rounding only, not full clock/ordering uncertainty. Missing/zero
clock values, future stamps and conversion overflow have separate counters;
none becomes a false zero-delay observation.

A crate-private recorder requires one completion writer per session and uses
only fixed atomic loads/stores with saturating counters. No heap work, locks,
retry loops, logging or frequency query is introduced in the recording path.
The actual unique PCM worker satisfies this single-writer contract. Readers
receive independent nontransactional live summaries; final evidence requires
the worker to have joined. Publication is not a multi-field snapshot barrier.

VAD telemetry schema advances from 3 to 4 with independent playback-transfer
and microphone-submission summaries. Legacy absent fields default to explicit
unknown provenance, not fresh evidence. App rows preserve every u64 digit and
raw 100 ns min/max, show unknown for incomplete/invalid summaries and state the
scope limitation. The fields remain diagnostic, never export gate authority.

## Host-only evidence

| Command | Final result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 536 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --test packet_service` | 6 passed; 1 allocation/deallocation guard, 5 shadow unit tests |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test packet_publication --test queue_inventory` | 31/8/12 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 134 passed; includes fixed recording/snapshot allocation guard |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 289 passed |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 464 passed, 3 ignored |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit 0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets` | Exit 0, warning-free cross-check; not Linux execution |
| Root fmt, scoped RT rustfmt and whitespace checks | Exit 0 |

Tests cover large exact raw ticks, subtraction-before-rounding, zero delta,
missing/zero/future clocks, arithmetic limits, counter saturation, separate
directions and legacy defaults. The actual injected PCM worker regression
first observes 0 service completions where 4 are required, then passes after
wiring. Stale playback and deliberately dropped WRITE coverage confirms that
observation does not imply successful delivery. App tests verify service
pass-through, exact values above 2^53, unknown/reversed/incomplete summaries,
English/Chinese presentation and lack of gate authority.

Retained failed/corrected attempts are not counted as passes:

- Core/telemetry API tests first fail compilation before implementation.
- Actual worker test fails at expected 4 versus actual 0, proving missing wiring.
- An initial allocator test refers to a nonexistent crate allocation helper;
  it is moved to the existing integration allocator scaffold.
- The shadow unit-test module initially resolves the wrong relative path;
  an explicit source-relative test path corrects it.
- Final fmt initially finds module ordering only; ordering is corrected and
  both root/scoped formatting checks pass before commit.
- Two inspection commands use incorrect log/glob paths; corrected explicit
  worktree-relative paths read the retained logs. They are not test evidence.

Logs: `target/s1-s3-20261028/s1-vad-service-*`. Read-only provenance/core
reviews report no remaining Important/Critical findings after narrowing the
recorder API to the crate. Parent independently verifies the final suites.
No remote CI, Linux execution, Miri or native-device success is claimed.

## Remaining gates

This closes a discarded-stamp/service-observability gap, not source/sample-age
propagation, whole-native-wake admission, sustained queue feasibility or the
full S1-S3 objective. Actual lower native negotiation, matched physical routes
and stable sub-10-ms output remain hardware-deferred. Driver ABI and buffers
are unchanged.

Correctness mode continues recording timing/quality faults with configurable
0..3000 ms extra progress grace, default 3000. Latency-sensitive mode retains
fail-fast behavior. Hard resource/IO/cleanup errors stay hard failures. No VM
timing retry or production buffer enlargement is used to manufacture success.
