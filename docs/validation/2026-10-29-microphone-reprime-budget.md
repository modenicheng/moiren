# Microphone reset readiness follows the prepared Engine budget

Host-only software work on 2026-10-29. No native audio device, VM, deployment,
driver build/install, physical latency or continuity qualification is involved.
The full S1-S3 objective remains active; this closes one S2 recovery defect.
Implementation commits `3579367` (preparation) and `e28ef71` (actual worker and
compiled graph regressions) were each pushed and verified against remote SHA.

## Defect and behavior

The old reset-only microphone pump used two packets of readiness and a wait
of 2.5 native periods. At 48 kHz, packet48 waits only 2.5 ms while an Engine256
publication can be 5.333 ms away. Merely extending the deadline is insufficient:
the first real SRC block can satisfy two packets, then run out before the next
Engine publication. The steady native phase and first-ever startup are separate
from this reset recovery path.

`PreparedVirtualBudget::microphone_reprime` binds the actual negotiated packet
and actual carrier capacity to immutable preparation evidence. Its ideal
readiness is the maximum of:

- The explicit admitted content target, if configured.
- Two actual native packets.
- The complete admitted native Engine burst, one packet, FIR lookahead and
  explicit wake/jitter reserve.

Compatible storage can limit that threshold; `capacity_limited` reports this
instead of resizing storage or claiming ideal recovery. Explicit Target refuses
an impossible threshold. Spare capacity alone does not change a reachable
threshold. The method recomputes public budget evidence from requested intent
and complete Engine demand, rejecting edited/underreported transfer, FIR, queue
or recovery fields and mismatched previously negotiated packets.

The finite maximum wait is the larger of the legacy 2.5-packet floor and a
nominal refill time plus a complete Engine wake and declared jitter. Refill
covers required inventory plus FIR at a conservative 998/1000 production
floor; positive microphone SRC correction slows native publication. Arithmetic
is checked and rounded upward. For explicit packet48, Engine256, cap1,
target369, capacity1024 and jitter32, readiness is369 frames and maximum wait
is143711 ticks of100 ns (14.3711 ms). This is a **reset-only nominal recovery
budget**, not operating latency, sample age or a physical scheduling guarantee.

The actual provider passes its prepared budget into the microphone worker and
rebinds recovery policy after every actual packet negotiation, before native
owner creation. A completed two-way reset fence arms one checked deadline.
Worker polls do not renew it; sleeps remain at most1 ms. Reaching readiness
releases immediately. Exhausting the deadline falls back to honestly counted
silence/ordinary driver drops rather than waiting forever or declaring success.
Cancellation and a new epoch clear the hold. Initial startup remains immediate;
queue targets, capacities, steady submission phase and validation modes stay
unchanged. Native packet admission still does not prove actual lower periods.

## Regression evidence

Logs are under the worktree's `target/s1-s3-20261028/s2-reprime-*.log`, with
`CARGO_TARGET_DIR=D:\coding\moiren\target`. Counts overlap and are not additive.

- Pure-policy RED: the original threshold misses the complete Engine wake;
  three assertions fail. Final `virtual_budget` suite has21 passing tests,
  including450 packet/block/rate/capacity combinations, unchanged intent,
  Compatible capacity limitation, invalid geometry, large integer arithmetic,
  no allocation/deallocation and public-evidence refusal.
- Worker API RED first records the expected missing API compile failure. A
  pass-through constructor then reproduces the behavioral failure: a WRITE
  occurs before the first Engine256 publication. Final focused worker suite
  has5 passing tests including exact-deadline fallback, no deadline renewal,
  unchanged first startup and cancellation/epoch refusal.
- The compiled Source -> Bus -> real `VirtualMicrophoneSink` SRC tests perform
  the full producer/consumer epoch handshake and discard old content. No fresh
  test-side prefill is used. f32/f64 each exercise36 combinations of Compatible
  versus Target, first-publication phase, native completion phase and
  graph-before/after-worker ordering for80 ms. They require continuing output,
  zero short-read silence telemetry and no carrier overflow. FIR startup zeros
  are not mislabeled as missing input.
- Keeping the new deadline but temporarily restoring two-packet readiness makes
  both compiled precision tests fail at10.5 ms with31 counted silent frames.
  The file is restored and the focused5-test suite passes. This isolates the
  readiness fix from the longer timeout.
- Independent review identifies public DTO underreporting. A new regression
  fails on a zeroed native-transfer field, then passes with complete evidence
  revalidation, including accepted consistent negotiation and refused mismatch.

Final local verification after the evidence fix:

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --test virtual_budget` | 21 passed |
| `cargo test --locked -p moiren-windows-audio --lib` | 474 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 261 passed |
| `cargo test --locked -p moiren-app --lib --features gpui-test -- --test-threads=4` | 456 passed, 3 ignored |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 100 passed |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit 0 |
| `cargo check --locked -p moiren-app --all-targets --features gpui-test` | Exit 0 |
| `cargo fmt --all -- --check`; `git diff --check` | Exit 0 |

The RT harness is unchanged by the public-evidence follow-up and was run in
this slice. Independent read-only review confirms the underreporting Important
is closed and finds no remaining Important/Critical in this bounded change.
No Linux/CI/Miri or native operational result is claimed. Unrelated `gate.log`
is excluded from all commits.

## Remaining gates

This is a software recovery regression result, not sustained real playback or
sub-10-ms qualification. Actual packet negotiation, matched physical routes,
VAD sample timestamp provenance and hardware stability remain deferred.
Process finite-window alternate-target queue feasibility and sample age stay
open: consumer retirement includes discards/censored demand, servo behavior is
target-dependent, and packet availability within a native drain is not known.
Endpoint inventory alone cannot prove a smaller target safe. No synthetic
initial fill or counterfactual replay claim is introduced.
