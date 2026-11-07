# Streaming Rename Waits Retain Errors From Both Route Exports

Date: 2026-11-05, Asia/Shanghai. Source base: `ae60e9b`.
This repairs the existing sticky-failure contract of the acceptance harness.
It is not a new rename mode or a native driver/audio qualification result.

## Source-Supported False Acceptance

The streaming coordinator's readiness-only wait discarded non-ready snapshots.
It also returned a ready snapshot without retaining its explicit errors. The
initial Apply, replacement provider/name readiness, and replacement Apply
waits could therefore forget an observed service, graph or provider failure
if a later QUERY/Engine confirmation cleared the published error.

The provider/name predicate is particularly direct: resource presence and the
requested provider name can satisfy it even while an explicit error exists.
Its returned snapshot was discarded before a subsequent Apply.

The first local correction covered only the selected export. Independent
review identified a second omission: both route exports participate in Pair
readiness, but a peer QUERY failure can survive only in that export's detached
diagnostics after Engine confirmation clears the global graph error. Recovery
could then allow a selected-export-only observer to accept the Case.

These are source-supported paths with deterministic portable reproductions,
not a claim that a new native failure was acquired.

## Final Observation Contract

The actual Windows loop now calls a portable function that owns projection of
both configured route IDs from the same snapshot before readiness evaluation.
The three waits carry explicit stages and minimum consumed-request IDs:

| Stage | Request floor |
| --- | --- |
| `old_apply` | Initial Apply request |
| `replacement_present` | Rename edit request |
| `replacement_apply` | Replacement Apply request |

Pre-request snapshots cannot satisfy readiness or attribute an error to this
request. Current-request service, graph and either export's provider errors
are retained before readiness, including errors on an otherwise ready
snapshot. Duplicate observations do not duplicate identical failures.

Request consumption establishes when the error was observed, not necessarily
which operation originally caused its text. Clean pending/removing states
remain ordinary pending states. The updated Case is published before a ready
snapshot returns; the original absolute phase deadline is still checked after
publication.

Every configured export remains represented by `ExportService::statuses`
through removal and recreation, including unavailable/absent resources.
Missing or duplicate configured IDs are structural inconsistencies, not an
ordinary lifecycle transient.

Existing exact Windows name, direct Removed, Start/PCM, admission generation,
freshness, owner-release and final cleanup acceptance remain mandatory. This
paired-route observation is not independent sibling endpoint isolation.

## Failure-First Verification

Evidence root:
`D:\coding\moiren-evidence\20261105-s4-wait-errors`.

1. The mechanically extracted original readiness-only processing produces
   139 Windows passes and three assertion failures: forgetting an error after
   recovery, ignoring an error on a ready snapshot, and evaluating readiness
   for a pre-request snapshot.
2. The selected-export correction passes its checks but remains superseded.
   An isolated extraction preserving selected-only projection produces five
   passing and two failing Linux wait tests. It demonstrably forgets the peer
   QUERY failure and projects only one endpoint before readiness.
3. The final correction collects both route exports. Seven wait regressions
   cover all three stages, both provider-error positions, ready/non-ready
   failures, stale requests, deduplication and clean pending states.

The original RED/selected-only logs, source archives and manifests remain
separate. The final-source four-file manifest is in `route-peers`; its checks
finish before source mutation or commit. Earlier selected-only passes do not
certify the final patch.

| Final-source check | Result |
| --- | --- |
| Windows explicit example | 145 passed, zero failed |
| Linux Rust1.99.1 explicit example | 124 passed, zero failed |
| Windows workspace all-target strict Clippy, both opt-in features | Exit0 |
| Linux Rust1.99.1 strict example Clippy | Exit0 |
| Windows workspace all-target check, both opt-in features | Exit0 |
| Windows non-test example build | Exit0 |
| Workspace fmt and diff whitespace checks | Exit0 |
| Workspace/headless with both opt-in features | 2625 passed, 10 ignored, zero failed across73 harnesses |

Explicit example totals are separate from the workspace totals. All four
final source length/SHA pins match after verification, and the original
worktree's29 preserved draft/cache pins remain unchanged. Independent final
read-only review finds no actionable blocker after the peer correction.

### Published Source And Optimized Artifact

The scoped correction is committed and normally pushed as
`8761d07f33488aed5950b45fd39b1d01a7dda6ed`. Local/remote hashes match and
PR26 stays Draft/open; its prior body is preserved verbatim with a new
checkpoint appended. The automatically triggered exact-head run
`41915822264` is observed by one retained watcher, not manually dispatched
or restarted. Its initial live state is historical; the closure below records
the terminal result.

A subsequent optimized example build starts from that clean source and
finishes with exit0. The [release record](2026-11-05-release-probe-readiness.md#readiness-wait-error-repair-artifact)
pins its source/tree, 7,819,264-byte immutable copy and SHA-256. It includes
the both-route observation correction. It is not executed or deployed; the
earlier pre-client-only binary remains historical, not the current shared
Cargo output. These later documentation changes do not alter tested source
and were held locally throughout the live exact-head run.

### Exact-Head CI Closure

Run `41915822264` completes all20 jobs successfully on exact `8761d07`,
including Windows/Ubuntu tests and strict Clippy, driver host, every Miri
suite and aggregate. GitHub's last completion is
`2026-11-04T22:01:19Z` (`2026-11-05T06:01:19+08:00`); the original watcher
finishes at06:01:24 host time with exit0. These timestamps remain separate.
The full final API result, watcher result and closure receipt are archived
in the same evidence directory. Nonfatal cache warnings do not change the
actual successful job conclusions.

This closes software CI for the both-route correction, not native or physical
acceptance. No CI is dispatched merely to record its own completion.

## Prior Published CI And Remaining Acceptance

The existing `ae60e9b` run `41907994546` completes all20 jobs successfully.
Its original watcher exits0 at05:06:07 host time; GitHub's last aggregate
completion is2026-11-04T21:06:15Z, recorded separately. The full API result and watcher
receipt are retained. This certifies that prior published checkpoint, not the
new wait correction, which requires its own matching-head CI.

No native clients, host UI/audio, driver installation or remote acquisition
are started by this correction. The proposed A -> B -> A mode is separate and
unimplemented. Full S0-S5 still requires matched/full-waveform and calibrated
evidence, native budgets, retained desktop acceptance, the complete naming and
lifecycle matrix, and sustained non-silent/load/fault qualification.
