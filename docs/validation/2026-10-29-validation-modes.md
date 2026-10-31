# Separate correctness collection from latency acceptance

## Priority change

On 2026-10-29 the user reported that ordinary music also breaks up in the VM
and asked to defer VM-dependent latency/stability acceptance until a physical
target is available. Continue portable development, graph/budget admission,
corruption-aware analysis, and lifecycle correctness independently. Do not keep
retrying the VM latency gate or enlarge production audio buffers to hide it.

This changes the test policy, not the sub-10-ms physical goal or the canonical
driver verdict. The retained baseline failures are diagnostics, not proof of a
particular root cause. Physical A/B timing and sustained output remain deferred.
The original IO/Bus goal remains in
[`vad-low-latency-goals-2026-10-28.md`](../vad-low-latency-goals-2026-10-28.md).

## Explicit test intent

The supervised `live-vad --vad-collection FILE` and owned
`live-process-loopback --process-collection FILE` probe routes, plus the
separate `live-physical --recorder-config FILE` route, support:

| Mode | Timing/continuity faults | Progress deadlines | Meaning |
| --- | --- | --- | --- |
| `latency-sensitive` (default) | Fail fast, retain first failure and final counters | Existing deadlines, no extra grace | Strict diagnostic acceptance; not automatically physical qualification |
| `correctness` | Record underrun/drop/timeout/jitter observations and continue collection | Configurable additional `0..3000` ms, default 3000 | Exercise the complete source/graph/recorder/cleanup path despite scheduling noise |

Example additions to an existing supervised collection command:

```text
--validation-mode correctness --jitter-tolerance-ms 3000
```

For strict behavior:

```text
--validation-mode latency-sensitive
```

The tolerance is control-side progress grace, not a requested WASAPI period,
queue target, recovery-cap multiplier, or claim that three-second audio stalls
are acceptable in production. Recorded soft counters do not terminate a
correctness collection, even after a quality threshold is exceeded. A source
that never completes still reaches its finite progress/total deadline; grace
never renews the original `--seconds` ceiling. Synchronous native calls and
joins remain subject to their existing ownership/cleanup behavior; this is not
an OS watchdog that can interrupt a blocked kernel call.

Existing clock anchors are preserved: owned Process uses its initial setup
anchor, while VAD uses the recorder activation request after preparation.
VAD's `--seconds` is therefore an activation-relative collection ceiling, not
a wall-clock cap that includes driver discovery, CREATE/preparation and joins.
The new physical route instead uses a single startup window anchored before
recorder preparation and treats `--seconds` as the full steady duration.
Its fixed overall ceiling is startup allowance plus steady duration; it does
not reset startup deadlines between recorder and graph owners. See the
[physical IO receipt](2026-10-29-physical-io-probe.md) for that command's
separate observation and retirement contract.

Identity, generation/revision/shape changes, missing or malformed mandatory
evidence, nonfinite samples, unexpected owner termination, native API failures,
invalid source accounting, storage caps and disk errors remain hard failures.
Correctness does not suppress errors by matching arbitrary error text.

## Result and automation contract

- Report and manifest retain the selected validation policy.
- Collection completion and cleanup completion are operational results.
  `quality_passed` is separate and may be false on a completed correctness run.
- First soft observations, raw cumulative final counters and hard failures are
  retained separately; repeated polls use bounded diagnostic storage.
- Correctness exit 0 requires complete collection, complete cleanup and no hard
  failure. It does not mean uninterrupted audio or a low-latency pass. Dry runs,
  incomplete collections and hard failures remain nonzero.
- Strict live behavior preserves the existing unqualified/nonzero convention.
  Every live report still says `qualified: false`; correctness never emits a
  promotable latency qualification report.

Hardware-dependent acceptance is intentionally not rerun for this change.
Software regressions must exercise the same injected soft fault in both modes,
hard-fault refusal, final/cleanup reconciliation, configurable grace and total
deadline bounds. Their results belong in the S1-S3 execution ledger.

## Playback sequence diagnostics continuation

`df6ea7e` adds independent playback native forward-gap observations and
missing-sequence-slot counters. The actual health and final quality paths count
missing slots once as delivery quality, not as proven dropped/silent frames.
The same injected gap continues correctness collection with failed quality
and is rejected by latency-sensitive validation. First-packet prefix remains
unknown; a new reset epoch starts a fresh accepted frontier. Microphone host
submission sequences are not native packet loss. Full software evidence:
[`2026-10-29-vad-sequence-observation.md`](2026-10-29-vad-sequence-observation.md).
