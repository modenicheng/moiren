# Live Process supply and demand prefix assessment

Date: 2026-10-29. This receipt covers a bounded live-prefix core and pinned
native collector API. **It does not complete S3 application admission or
establish stable real audio below 10 ms.**

## New behavior

`ProcessTraceArchive::replay_live_window` assesses a candidate while original
history writers remain open. `NativeProcessTraceCollector::replay_live_window`
polls its already pinned source and calls that same implementation. Both take
explicit `ProcessBridgeOptions` and `ReplayWindowPolicy`.

This supplements the existing per-wake measured reserve. A regression uses
actual software bridge histories whose individual demand requests fit the
candidate reserve, while dense repeated reads exhaust available supply.
The finite-window replay reports that loss rather than inferring sustained
balance from the single largest wake.

The result contains:

- Requested candidate options and policy, original Process identity and owner
  generation, and caller-supplied versus retained-native origin.
- `no_violation_in_window`, `observed_violation` or `inconclusive`. Loss takes
  precedence over insufficient exposure; explicit output-frame/read/span
  thresholds still apply.
- Original writer closure flags, retained versus replayed event counts,
  independent packet/read sealing horizons and the common cutoff.
- The actual candidate replay and inventory assessment when evidence permits,
  otherwise a bounded refusal plus the original collection refusal cause.
- `qualified: false`. No writer teardown, signal continuity, source age,
  future scheduling or physical latency is inferred.

A public archive always retains `CallerSupplied` origin. The native collector
uses its private original source association even after registry replacement;
exporting or editing its archive cannot preserve authenticated native origin.
These are diagnostic reports, not opaque Apply authorization tokens.

## How the window is bounded

Both packet and queue histories must be complete from sequence one, within
8192 events per writer and the existing per-event/frame-work bounds. Full
writer payloads, counters, generations and clock envelopes are validated
canonically before selecting a prefix. Missed, conflicting, saturated or
capped collection stays inconclusive.

The cutoff is the minimum of the latest retained packet and read envelope
starts. Only complete events ending strictly before that cutoff are assessed.
Each sole writer has already completed a retained sentinel beyond the earlier
common prefix. Unsealed or in-flight tails cannot supply or consume within
the reported window. Exact cross-writer order and acquired publication
frontiers are validated again on the selected prefix; overlap and timestamp
ties still refuse.

The candidate uses the production `capture_bridge` mechanics from the known
sequence-one empty start. Its queue, held interpolation state, fractional
phase, prime/servo behavior and reset/loss accounting persist through that
entire replay. The implementation does not invent an occupancy at an arbitrary
overlap boundary or substitute nominal-rate frame arithmetic for actual queue
consumption.

Each assessment replays one finite sequence-one prefix off RT. It does not
claim checkpointed continuation beyond the archive cap. A capped run cannot
restart a fresh empty candidate chunk and call it a continuous window.

## Compatibility and refusal boundaries

Existing closed constructors and `ordered_trace()` still require actual
finished writers. The temporary canonical validation writer's destruction
does not become native closure evidence. Open-prefix reports cannot enter the
existing stopped retained-evaluation/Apply path.

Existing measured per-wake selection, declared defaults, production queue
capacities and RT instrumentation are unchanged. The native wrapper refuses
an original owner with host-ingress elapsed enforcement, including when the
candidate omits that field. A candidate requesting that enforcement receives
an inconclusive report retaining its exact intent and refusal. Sample-age
intent and incompatible rate/demand remain refused by existing preparation
and replay checks.

The standalone RT shadow harness compiles the same new source through explicit
module paths and report-type reexports. Its checks do not replace actual
Windows/native tests.

## Verification and delivery

Source: `5df48d03168ba3aed344038d6d2f782babfe0a8c`
(`feat(audio): assess sealed live Process supply demand prefixes`).
Independent watermark/source and compatibility/provenance reviews by Dewey
and Newton found no concrete remaining scoped defect.

Fresh checks on this source, under `target/s1-s3-20261028/`:

| Check | Result | Log |
| --- | --- | --- |
| Focused core and pinned native-wrapper regressions | 11 passed | `live-prefix-focused.log` |
| Full audio library, integration and doc tests | 842 passed, 3 ignored, 31 summaries | `live-prefix-audio.log` |
| Full probe regression | 471 passed | `live-prefix-probe.log` |
| RT shadow harness | 193 passed | `live-prefix-rt.log` |
| Strict Windows audio all-target Clippy | Passed | `live-prefix-clippy-final.log` |
| Strict actual probe test-profile Clippy | Passed | `live-prefix-clippy-probe-tests.log` |
| Workspace formatting and scoped diff checks | Passed | `live-prefix-fmt.log` and pre-commit diff check |
| Linux audio all-target compilation | Passed with warnings described below | `live-prefix-linux-all.log` |
| Linux RT all-target compilation | Passed, warning-free | `live-prefix-rt-linux.log` |

Genuine RED evidence is retained in `live-prefix-red.log` (1 passed, 4 failed),
`live-prefix-native-red.log` (2 failed) and `live-prefix-refusal-red.log`
(1 failed). The failures preceded their implementations. The first RT
attempt in `live-prefix-rt-first.log` failed to compile because the shadow
module needed explicit source paths and its own report-type reexport.
Those were repaired before the final 193-test and Linux RT checks.

The Linux audio library-test compile has two existing unused test helpers
(`with_publication_gap_100ns`, `with_retained_owner_origin`) and the new
cfg-only unused `replay_retained_live_window`; the normal probe has the
existing unused `progress_grace`. Linux compilation is not executed Linux
tests or native audio validation. RT totals include shadow unit tests;
they do not mean 193 independent allocation-only checks.

No native audio, VM, driver installation/deployment or remote push is involved.
Unrelated `gate.log` remains untouched. S3 still needs explicit application/CLI
policy selection, source/settings/proof binding before replacement admission,
and checkpointed long windows. Freshness-decision/acquired-prefix provenance
and physical measurement remain separate open gates in the
[S1-S3 ledger](2026-10-29-s1-s3-software-ledger.md).
