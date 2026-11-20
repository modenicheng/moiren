# Control-polled Process prefixes and bounded replay

2026-10-29 (Asia/Shanghai). Host-only continuation of S1/S3; no native audio
execution, VM, deployment, driver build/install or production buffer tuning.
The full S1-S3 objective remains active, not marked hardware-qualified.

## Atomic receipts

All implementation commits were pushed to `origin/feat/minimal-io-latency`
and compared with `git ls-remote` before proceeding. Unrelated `gate.log`
remains untracked and untouched.

| Commit | Responsibility |
| --- | --- |
| `9a13f7f` | Control-only prefix collection, schema2, bounded full-prefix validation |
| `78b0633` | Actual offline replay CLI imports schema1 and schema2 |
| `8d97154` | Both actual Process collectors poll before activation and through collection |
| `aa74c39` | Explicit full-work boundary and six-second target/capacity regression |

## Behavior and limits

Realtime packet/read history storage remains256 events per writer, with no
audio-side allocation, logging, retries or larger audio capacity. A separate
control-thread collector merges exact sequence-one prefixes from actual
snapshots, validating overlap and latching missed-event/conflict/event-limit
refusals. Contention is one unsuccessful snapshot attempt, not stale success.
Writer closure and final counters are still collected after a refusal; no
archive refusal requests audio stop. Once invalid, the archive cannot restart
at a convenient later suffix or invent empty initial inventory.

Schema2 keeps at most8192 events per writer. Full-prefix validation runs the
canonical writers and verifies each256-event chunk before native storage can
overwrite it, plus final counters. Both writer closures are required for
replay, but closure is not successful transport cleanup. Single-event work
remains65536 frames; archive offered-plus-requested work is capped at16777216
frames. Schema1 retains its256-event and1048576-frame limits.

Strict import uses direct bounded visitors, not an untagged enum that first
buffers unbounded event arrays. Schema2 requires an explicit nullable
`collection_refusal` field; schema1 rejects it. Unknown fields, unsupported
schemas, oversized histories and malformed counts remain refusals. Imported
identity is caller-supplied, never promoted to retained-native provenance.

Both actual collectors initialize before activation and poll on their control
passes and after joins/source destruction. The ordinary finite path now polls
even without an observations journal or controlled load, retaining natural
completion and requesting stop only at its bounded deadline. Archive output is
streaming-capped at16 MiB before a new file is opened. `replay-process` accepts
schema1/2 with1..16777216 input bytes per file, but its output remains separately
capped at1 MiB;16 explicit candidates maximum. IO faults remain hard failures.
Pre-open cap refusal creates no file; a later IO write failure may leave a
partial file and is not claimed atomic.

Polling is best-effort, not a scheduling guarantee. A long scheduler pause,
blocking controlled-load shutdown, or event-limit exhaustion can still make
the trace incomplete. That produces diagnostic export and replay refusal,
not a claim of completeness merely because final writer closure is true.

## Host-only evidence

| Command | Fresh result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` |530 passed,2 ignored |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` |289 passed, including15 CLI tests and11 collector/export tests |
| `cargo test --locked -p moiren-app --lib --features gpui-test` |456 passed,3 ignored |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` |128 passed, including27 shadow replay/archive tests; not128 independent allocation tests |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test packet_publication --test queue_inventory` |31/8/12 passed |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` |Exit0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` |Exit0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets` |Final exit0, warning-free; cross-check, not Linux execution |
| `cargo build --locked -p moiren-windows-audio --example e2e_latency_probe --target x86_64-pc-windows-msvc --features vad-probe` |Exit0; actual executable exercised offline |
| Root fmt, scoped RT rustfmt, whitespace checks |Exit0 |

A real capture-bridge software regression records6000 packet/read events at
48 frames/1 ms, a six-second modeled window, while native histories still hold
only256 each. Targets256/512/1024 and independent capacities2048/4096 replay
with no candidate drop, underrun or post-startup unserved request. This is
synthetic queue mechanics, not signal continuity or real-time scheduling.
An exact16777216-frame validation boundary passes; the next event exceeds
the work cap and refuses, despite remaining under the event cap.

The rebuilt actual probe executable also processes a synthetic schema2
6000-event fixture. Baseline288000 dropped supply frames remain separate from
candidate results. Targets256/512/1024 produce startup unserved240/480/1008
frames, no later unserved demand, no candidate drop/underrun and output counts
287760/287520/286992. Exact creation time9007199254741115 is preserved. Exit0
means completed sweep, not quality qualification. Wrong generation returns1;
existing output returns2 with unchanged SHA-256; amplified output returns2
without creating the destination. All reports remain CallerSupplied and
qualifiedfalse.

Recorded red/corrected attempts are retained rather than counted as passes:

- New archive/collector APIs first fail compilation, then focused suites pass.
- CLI expanded tests first report4 passed/11 failed, then15 pass.
- Serializer cap test truly fails when an entire oversized vector is built;
  the streaming-capped implementation stops serialization early and passes.
- The first work-limit fixture has an ambiguous Rust integer type; its explicit
  usize correction compiles and validates both cap edges.
- Early Linux RT checks show unused shadow reexports. Matching actual public
  test imports removes warnings; final cross-check is clean.
- First manual binary candidate declares a48-frame gap, but its positive
  publication-envelope duration requires49 under conservative ceil arithmetic.
  The sweep correctly retains candidate refusals. Explicitly declaring49 in
  the fixture yields the successful sweep; no automatic fallback/code weakening.
- One PowerShell log-inspection call uses unsupported brace expansion and is
  immediately rerun with explicit paths; it is not validation evidence.

Logs: `target/s1-s3-20261028/s3-prefix-*`; actual binary fixtures and final
receipt: `target/s1-s3-20261028/prefix-cli-binary/verified-final.txt`.
Independent read-only review finds no Important/Critical issues in the library
or reviewed collector/CLI diff; reviewer did not independently rebuild tests
or separately revalidate finalized collector commit8d97154. Parent verification
uses the full final suites reported here. No remote CI/Miri run is claimed.

## Remaining gates

This replaces the short native-suffix-only limitation with bounded complete
prefix replay. It does not provide indefinitely sustained history, arbitrary
suffix initial-state reconstruction, overlap-order inference, future-safe
production admission or an automatic desktop profile selector.8192 events at
1 ms cadence is only about8.2 seconds per writer; larger demands may hit the
work limit sooner. Genuine cross-writer overlapping envelopes are still
refused, not ordered by guesswork.

Sample-age provenance, whole-native-wake admission versus segmented read
demand, actual lower-period negotiation, matched physical routes and stable
sub-10-ms real output remain open/deferred. The user's existing correctness
mode still records timing/quality faults and continues with configurable
0..3000 ms additional progress grace; latency-sensitive mode fails fast.
Hard resource/IO/cleanup faults are not reclassified as jitter. No production
audio buffer was enlarged to hide VM behavior, and no VM timing gate is retried.
