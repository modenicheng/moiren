# Live VAD provider loss/silence gate

## Defect and repair

The latency-sensitive VAD collector can observe provider/driver silence or
driver loss yet continue until a later retirement check. Its live
`Health::require_present()` rejects nonzero `quality_faults`, but
`live_native.rs::endpoint_health()` omits two relevant observation sources:

- Worker `silence_packets` and `silence_frames` are absent from its aggregate.
- The current driver QUERY's `dropped_packets` and `silence_packets` are not
  included at all. Worker dropped/completed counters do not replace these
  separate driver boundary observations.

For example, an otherwise admitted MIC snapshot with driver silence1 and
worker fault counters0 produces live `quality_faults=0` before this repair.
The strict health check therefore does not reject that observation. The
existing final [retirement QUERY check](2026-10-31-native-first-mic-retirement.md)
can still detect the final driver counter; that does not supply the missing
live fail-fast behavior. This finding does not establish a cause for any
historical native fault or justify a replay of an acquisition.

The probe now folds both worker silence fields and both raw driver QUERY
counters into the control-side saturating nonzero fault gate. Playback and
MIC use the same endpoint helper. Strict policy rejects the faulty health
observation; correctness policy retains raw fault evidence and can continue
operational collection. Provider silence metrics describe encoder fallback
frames, not zero-valued source content or marker gaps. Their semantics and
the driver counters remain unchanged.

The aggregate already mixes packet, frame, reset and other event units and
can include overlapping observations. It is only a nonzero fault predicate,
not a count of distinct lost packets/frames or a latency measure. Each raw
typed field remains in the report without subtraction, normalization or
deduplication. Saturation stays nonzero, and unavailable evidence remains
subject to the existing readiness/quality-known checks.

This is an S1/Stage2 probe reporting/admission repair. There is no production
provider, driver, ABI, scheduling, RT, buffer, default, source waveform or
retirement-order change. `quality_passed`, `quality_complete` and `qualified`
definitions are unchanged. In particular, the collector continues to write
`quality_complete=false`; the [two selected QPC labels](2026-10-31-capture-qpc-provenance.md)
do not provide grounds to remove that boundary.

## Regression evidence

Repository base is e85196e. Host evidence root is
`D:\coding\moiren-evidence\20261031-live-provider-quality`; its inherited date
prefix does not replace the UTC timestamps retained in each receipt.
The frozen hypothesis pins seven existing source/plan inputs before tests or
implementation. No remote/native acquisition, audio owner or physical render
is started; no host/target driver, system-control or default-role write occurs.

Two new typed endpoint regressions are run before the repair. Both fail with
assertions, exit101: provider silence produces zero fault count, and driver
QUERY saturation is omitted. These are compiled/executed regressions, not a
build failure or a rejection-only stub. After the minimal aggregate change:

- Each worker silence field and each driver QUERY loss/silence field is
  independently nonzero in both Playback and MIC fixtures. The real
  `EndpointSnapshot::decode` and `endpoint_health` path is exercised, followed
  by the strict Health gate. Raw observations remain identical after checks.
- Independent QUERY/worker faults saturate at `u64::MAX`, never wrap to zero.
  Existing aggregate/readiness/unknown-identity tests remain intact.
- A policy composition regression covers both endpoints, all four fields and
  both policies. Strict validation returns an error and retains first failure;
  correctness returns operational success while retaining quality failure and
  both raw provider observations. Incomplete boundary quality stays explicit.
  This policy test supplies an explicit Health fault value; the separate typed
  endpoint regressions prove the production aggregation that creates it.

The initial focused endpoint filter passes14 tests (it also selects existing
endpoint-named tests); focused correctness-health passes4. The new policy
composition test passes separately. The final integrated checks pass:

```text
cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe
    630 passed; 0 failed; 0 ignored
cargo clippy --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --tests -- -D warnings
cargo fmt --all -- --check
git diff --check
cargo check --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --tests --target x86_64-unknown-linux-gnu
```

The Windows suite is hardware-free host execution. Linux verification is
cross-target compilation only, not Linux test execution. No remote CI result,
full-workspace test result or native validation is claimed by these checks.
`red-receipt.json`, focused/policy receipts, `final-checks.json` and
`linux-compile-receipt.json` retain exact argv, times and exits. Final source
hashes are pinned in the check receipt before this documentation is committed.

## Remaining acceptance

The software regression closes a demonstrated omission in live fault
admission. Its timely behavior against actual target silence/loss still needs
native verification; historical acquired files and their refusals remain
unchanged. The final retired-QUERY repair also remains natively unverified.
Complete changing-waveform/clock handling, continuous quality, matched
preceding-arm/rebase evidence, sustained lower-budget delivery, matched
VAD/Process comparison and calibrated physical acceptance remain open.
S1-S3 are incomplete; no stable sub-10-ms, device floor, impossibility or
lower-default promotion follows. Delivery is a scoped local commit, no push;
unrelated `gate.log` is preserved.
