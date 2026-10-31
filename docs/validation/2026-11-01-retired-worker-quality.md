# Retired VAD worker quality gate

## Defect and repair

The supervised VAD collector checks live provider health and then performs
best-effort retirement. Its final retirement validator previously checked only
driver QUERY `silence_packets` and `dropped_packets`. Worker faults recorded
after the last live sample could therefore be present in the retained worker
snapshot without reaching the quality latch. The preceding
[live loss/silence gate repair](2026-11-01-live-provider-quality.md) does not
replace this tail check.

The control-side validator now also checks the last live and retired provider
worker observations. Both must have `diagnostic_provenance="Fresh"`, explicit
`error:null` and raw telemetry `schema_version=5`. It requires these twelve raw
unsigned counters without compatibility deserialization:

```text
dropped_packets              driver_dropped_packets
quota_dropped_packets        carrier_dropped_packets
stale_packets                silence_packets
silence_frames               completion_clock_regressions
sequence_gap_observations    missing_sequence_slots
epoch_resets                 recovery_events
```

Missing, malformed, saturated (`u64::MAX`) or regressed required counters are
hard evidence failures under both policies. Unsupported or absent schema and
stale/failed provider views are also hard failures. Reading raw JSON is
intentional: `VadPcmTelemetrySnapshot` has compatibility defaults and schema
normalization, which must not manufacture zero-fault evidence here. This is a
check of these required fields, not complete telemetry accounting.

For the first ten counters, any nonzero retired total is a quality fault, not
a baseline-subtracted delta. Strict policy rejects it; correctness collection
may continue operationally but retains quality failure. Playback is required
for both routes; the MIC route additionally requires the microphone provider.
The existing same-endpoint/session/export identity, final Removed and driver
QUERY checks remain in place. Raw observations and the original first failure
are retained unchanged. Counters can overlap and have different units; they
are not a distinct loss total or a latency estimate.

`epoch_resets` and `recovery_events` need a separate boundary: intentional
native Stop/admission closure can advance these totals. The validator keeps
their raw final totals and rejects a nonzero last-live total under strict
policy; it never clears an already latched live failure. Final-only lifecycle
increments are **unattributed**, not proven harmless. This patch neither
identifies their cause nor supplies complete interval quality.

The collector invokes this validator after `guard.finish()` when the recorder
was admitted. The diagnostic prefix now says `final provider quality` rather
than implying all failures came from QUERY. There is no production provider,
driver, ABI, RT, retirement-order, scheduling, waveform, buffer or default
change. The fixed `quality_complete=false` rule remains unchanged.

## Retention boundary review

The production worker sets its drained Release flag after worker completion
and overlapped-PCM cleanup. `VadSession::stop_and_join` cancels and joins that
worker. `EngineHost` shutdown joins its cleanup worker, and both collector
routes attempt host shutdown before endpoint retirement. The retirement loop
retains provider JSON and waits for Removed; provider retirement refuses a
session which has not drained. Existing cleanup tests cover the route ordering
and continued cleanup after failures.

These source relationships do **not** prove every retained worker snapshot is
the last atomic state. `Lifecycle::poll` snapshots telemetry before its Acquire
read of `drained` and removes drained sessions. A concurrent finishing worker
can therefore require closer publication/retention analysis; an earlier poll
may already have removed its session before host join. This patch does not
alter that production publication path or claim a concurrency repair. It
validates the raw snapshots actually supplied to it. A fresh driver QUERY is
not a guarantee of a coherent final worker snapshot. The synthetic regression
fixtures do not exercise worker publication or protocol-decoded identity.

The subsequent [drained-worker retention repair](2026-11-01-drained-worker-retention.md)
reproduces this precise stale-copy/removal interleaving with a real worker
thread and makes both control-side reapers Acquire drained before copying
telemetry. It closes that demonstrated race in software, not coherent live
publication or actual native quality. The review above records the boundary
at this document's original `275406c` delivery.

## Regression evidence

Repository base is `bc7cc62`. Host evidence root is
`D:\coding\moiren-evidence\20261101-retired-worker-quality`.
The frozen hypothesis pins eight source/plan inputs before the regressions
and implementation. Receipts contain exact argv, UTC times and exit codes;
the local date is November 1, 2026 (Asia/Shanghai).

Five compiled regressions first fail with assertions, exit101, against the
omitted worker check. After the repair the focused retirement suite passes10
tests. Additional cases remove each required counter individually and reject
fractional values or absent provider provenance/error fields. One subsequent
full-suite attempt fails compilation because a new test loop was placed in
the wrong scope (`before` unavailable). That test placement is corrected; its
failure logs remain, separate from the genuine omission RED run.

The final integrated checks pass on the corrected source:

```text
cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe
    635 passed; 0 failed; 0 ignored
cargo clippy --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --tests -- -D warnings
cargo fmt --all -- --check
git diff --check
cargo check --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --tests --target x86_64-unknown-linux-gnu
```

The Windows suite is hardware-free host execution. Linux verification is
cross-target compilation only, not test execution. No workspace-wide, remote
CI or native audio result follows. The tests cover each of the ten tail-fault
counters on both routes/policies, invalid/missing/regressed evidence, lifecycle
boundary handling, raw observation retention and first-failure preservation.
`red-receipt.json`, `green-focused-receipt.json`, `full-probe-receipt.json`
(the retained compilation failure), `full-probe-v2-receipt.json`, the lint/fmt/
Linux receipts and `final-checks.json` record these distinct outcomes. Final
source hashes are pinned before documentation and local commit.

## Remaining acceptance

This closes a demonstrated omission for available retired worker snapshots,
not the complete end-of-collection quality problem. Coherent live publication,
native final-retention verification, attribution of final-only lifecycle
changes and actual native tail-fault detection remain unverified. Historical
reports and their original refusals are not rewritten or accepted by these
software tests.

Complete changing-waveform/clock handling, continuous quality, matched
preceding-arm/rebase evidence, sustained lower-budget delivery, matched
VAD/Process comparison and calibrated physical acceptance remain open.
S1-S3 remain incomplete; no stable sub-10-ms result, device floor,
impossibility claim or lower-default promotion follows. No native replay,
remote/audio process, physical render or system-control/driver write is
performed. Delivery is a scoped local commit only, no push or subagents;
unrelated `gate.log` is preserved.
