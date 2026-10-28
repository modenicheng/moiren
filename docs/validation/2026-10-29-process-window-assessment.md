# Finite Process replay window assessment

2026-10-29 (Asia/Shanghai). Host-only S1/S3 continuation. No native audio
execution, VM/deployment, driver build/install or production buffer changes.
This improves candidate evaluation, not sustained or sub-10-ms qualification.

## Atomic receipts

Each implementation commit was pushed immediately and its full SHA compared
with `git ls-remote origin refs/heads/feat/minimal-io-latency`.

| Commit | Responsibility |
| --- | --- |
| `5a0a76c8a84fa905f11a7f21d9e81e4414f2e1d5` | Exact candidate inventory and typed finite-window assessment; minimal exhaustive CLI error mapping |
| `ff0dd5c31cdee3d5e5b16eb53baceb393d3474c9` | Opt-in actual replay CLI policy/gate, strict parsing, help and regression tests |

Unrelated untracked `gate.log` remains untouched. The implementation plan is
`docs/superpowers/plans/2026-10-29-process-window-assessment.md`.

## Inventory and assessment contracts

Replay still executes the actual bridge with synthetic PCM and a fresh empty
queue. Existing 256-event candidate read history is collected every 256 reads
plus the final partial chunk, offline. No new realtime storage, allocations,
default tuning or audio capacity is introduced. Every retained sequence and
retirement transition is checked. Conservation uses native input frames:

```text
written = retired input + terminal pending
consumed input = retired input - prime/backlog/age/reset discards
```

Output-rate frames are not substituted for retired native input. Reset-held
losses, previously absent from the candidate report, are explicit. Global
pending extrema include initial zero, post-publication peaks and post-read
troughs; post-start extrema begin at the first output-bearing read. These are
event-boundary samples, not continuous extrema or sample age. Existing
read-only fill extrema remain available separately.

`ObservedProcessTrace::replay_window` validates explicit thresholds, runs the
candidate once and assesses that exact result. No public API promotes an
externally edited report into an assessment. Results distinguish:

- `observed_violation`: an explicit initial allowance is exceeded, or later
  unserved demand, overflow, prime/backlog/age/reset discard or reset is seen.
- `inconclusive`: no violation, but insufficient output frames, output-bearing
  reads or conservative output exposure. Never-started output cannot pass.
- `no_violation_in_window`: none of the above in this finite window, not
  future-safe admission, waveform playback or physical timing qualification.

Only complete zero-output requests before the first output-bearing read use
the initial unserved allowance. Any unserved suffix of that first partial
output read is post-start loss. Initial prime trimming includes the first
output-bearing read; later prime trimming violates. The legacy
`startup_unserved_frames` field is retained but does not excuse that tail.
Violations take precedence over insufficient exposure.

Exposure uses original trace first/last output-read envelopes, not replay CPU
time. At least two output-bearing reads are required. Lower span is
`last.before - first.after`; upper span is `last.after - first.before`.
Producer-only trailing supply is reported without extending output exposure.
No inventory slope or future extrapolation is inferred.

## Optional CLI gate

The bounded `replay-process` command accepts `--window-policy FILE`, for example
this explicit experiment policy:

```json
{
  "schema_version": 1,
  "policy": {
    "max_initial_unserved_frames": 1024,
    "max_initial_prime_discarded_frames": 64,
    "min_output_frames": 280000,
    "min_output_reads": 5900,
    "min_output_span_100ns": 59000000
  }
}
```

Initial allowances use consumer/output-rate frames and native input frames
respectively. These are fixture thresholds, not production defaults or
hardware qualification criteria. Every field is required; minimum frames/span
must be positive and minimum reads at least two. Unknown/duplicate fields,
wrong schemas and malformed/missing policy files are invalid input. Exact
`u64` values remain integers, not floating point.

Without the flag, existing sweep semantics are unchanged: exit 0 means sweep
completion even with poor-quality or refused candidates. With the flag,
exit 0 requires every candidate to replay with `no_violation_in_window`;
refused, violating or inconclusive candidates return 1. Invalid input/IO
returns 2. Raw reports/refusals remain on gate rejection. Top-level
`status: completed` still means sweep completion; `window_accepted` and exit
status carry the optional gate result.

Policy shares the explicit per-file 16 MiB input cap. The 16-candidate and
separate 1 MiB output caps remain. Serialization is capped before opening a
new destination; existing files cannot be overwritten. Later actual IO write
failure can still leave partial output, not an atomic-write guarantee.
Imported histories remain `caller_supplied` and `qualified: false`, including
a passing gate.

## Fresh host verification

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib process_loopback::replay` | 34 passed |
| `cargo test --locked -p moiren-windows-audio --lib` | 543 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 298 passed, including 24 replay CLI tests |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 464 passed, 3 ignored |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | Final 142 passed, including shadow offline replay tests; not 142 separate allocation tests |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test packet_publication --test queue_inventory --test packet_service` | 31/8/12/6 passed |
| Strict all-target audio Clippy with `vad-probe`, app Clippy with `gpui-test`, RT harness Clippy | Exit 0 |
| RT all-target Linux cross-check | Exit 0, warning-free; not Linux execution |
| Root/scoped RT fmt and diff whitespace checks | Exit 0 |
| Actual probe build with `--target x86_64-pc-windows-msvc --features vad-probe` | Exit 0; exercised offline |

Actual probe SHA-256:
`BAC58A0A3AB6991CB28732CF92C6B18DE8DB873BA70D25CAAA93FE978FA63D49`.
The actual binary uses the existing synthetic schema2 6000-packet/read fixture
without native discovery. Candidates independently pass the explicit policy:

| Target frames | Output frames | Initial unserved | Initial prime discard | Lower output span, 100 ns | Terminal native frames |
| --- | --- | --- | --- | --- | --- |
| 256 | 287760 | 240 | 32 | 59939999 | 223 |
| 512 | 287520 | 480 | 16 | 59889999 | 479 |
| 1024 | 286992 | 1008 | 32 | 59779999 | 991 |

Raw reports match ungated replay exactly. Zero startup-unserved allowance
returns 1 with `observed_violation`; requiring ten seconds of output exposure
returns 1 with `inconclusive`. Mixed accepted/refused candidates return 1
when gated and 0 when ungated. Wrong generation returns 1; malformed policy,
duplicate flag, input/output cap and existing destination return 2. Pre-open
cap refusal creates no output. Existing input/evidence and destination hashes
stay unchanged; creation time `9007199254741115` stays exact.

## Red evidence and review

- Inventory/assessment tests first fail on missing APIs, then focused tests
  pass. Nine new CLI tests first fail, then all 24 pass.
- Temporarily disabling post-start-unserved violation makes the first partial
  output regression fail on missing `PostStartUnserved`. The script initially
  expects a different assertion message and itself exits 1; its `finally`
  restores original source bytes. The unchanged condition is verified and
  all 34 focused tests pass after restoration. No mutant remains.
- A fixture initially assumes an exact legacy fill maximum despite a later
  read. Corrected fixtures prove the real pre-trim 20-frame peak exceeds the
  read-only maximum; production behavior is not altered to fit the test.
- First RT run reports an unused report reexport in the shadow module. A
  local shadow-only allow matches its dead-code policy. Final strict Clippy
  and Linux checks are warning-free; production APIs are not widened.
- A partial-index compatibility blob initially has PowerShell-added trailing
  whitespace; cached diff checks refuse it before commit. Explicit UTF-8/LF
  bytes correct the index without altering working CLI changes.
- Initial multi-file documentation patches miss exact anchors and apply no
  changes; smaller patches correct the writes. Wrong-path inspection calls
  are not counted as validation evidence.
- Independent read-only core and CLI reviews find no Important/Critical
  findings; reviewers do not independently run tests.

Host logs/receipts: `target/s1-s3-20261028/s3-window-*` and
`target/s1-s3-20261028/window-cli-binary/`.

## Remaining gates and VM policy

Existing correctness mode records timing/quality faults without early abort,
with configurable 0..3000 ms extra progress grace (default 3000).
Latency-sensitive mode fails fast with no extra grace. Identity, schema,
resource, native IO, storage and cleanup failures remain hard errors. This
does not enlarge production buffers or reinterpret three seconds of jitter
as acceptable audio latency.

Finite candidate assessment is usable. Source-sample age provenance,
future-safe sustained admission, automatic desktop profile selection, actual
lower-period negotiation and matched physical VAD/Process end-to-end stable
sub-10-ms output remain open/deferred. Whole-native-wake production protection
is already implemented and separately verified; segmented replay is not
promoted to that evidence. Archive caps, missed polls and overlapping clock
envelopes can still prevent replay. No VM timing gate, native listening test,
remote CI or Miri run is claimed. Full S1-S3 stays active; unavailable hardware
does not stop independent software work.
