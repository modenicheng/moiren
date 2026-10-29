# Process host-ingress elapsed enforcement

Date: 2026-10-29. Baseline: `d3864eb`.
This advances S3; the full S1-S3 objective remains active.
Core/API and admission source: `02bd1f0`. Desktop/CLI and localization: `03e77a0`.

## Behavior and limits

An optional positive `ProcessBridgeOptions.max_ingress_elapsed_100ns` limits
host time since packet publication started. It is independent of FIFO capacity,
target fill, delivery assumptions, and source/sample age. A stalled consumer
can retain old input below high water; the new policy clears that input before
it can resume through the next read. The compatibility default remains `None`.
`Some(0)` is refused. Native preparation refuses an enabled policy without
its prepared host QPC clock.

Frames retain the packet's existing publication-start host observation without
changing native sample/device stamps. Producer clock high-water includes both
publication endpoints, so a later start behind a known completion has unknown
ingress provenance. Completion cannot retroactively revoke frames that another
thread already consumed.

Each enabled read acquires one finite ring prefix, observes the host clock once
after acquisition, and validates retained frames and actual interpolation
support. Missing, zero, future, or regressed observations do not prove freshness.
The consumer retains the highest known pre/acquisition/post observation,
including endpoints from a rejected bracket. There are three clock observations
per enabled nonempty read and two with the policy disabled, never per sample.

Invalid or over-limit support clears held frames and the remaining acquired
prefix in one recovery. This can also discard fresh frames in that prefix.
Publications after acquisition belong to a later read and are not chased or
discarded by that recovery. Repriming uses the existing 32-frame fade. Freshness
takes precedence when invalid support also changes generation; fresh or
disabled generation-only recovery retains its previous behavior.

`ingress_elapsed_discarded_frames` and `ingress_elapsed_recoveries` describe this
recovery separately from native sample-age, backlog, prime, overflow, underrun,
and reset-held loss. Queue conservation, atomic history, closed-writer evidence,
retained archive events, baseline replay reports, and replay inventory retain
the new loss category. Old queue JSON defaults it to zero; zero is omitted.

The actual desktop selector and monitor CLI expose independent intent:

- `TransportLatencySettings.process_max_ingress_elapsed_100ns` persists exact
  optional values and merges independently of Process/VAD queue edits.
- The selector offers Disabled and 10/20/50/100/250/1000 ms; arbitrary existing
  positive values remain exact. The control is available under Compatible
  input policy. Chinese and English copy distinguish host residence from
  unsupported source age.
- `monitor --max-ingress-elapsed-100ns N` accepts Compatible or declared input
  settings and refuses zero, malformed, duplicate, and missing values.
- Save while running changes requested settings; Restart Audio waits for
  matching active owner confirmation. Requested and active summaries include
  the policy in both diagnostics and the measured editor.

Current measured/replay admission does not reproduce native freshness clocks.
Enabled policies therefore receive explicit refusal in CLI, backend builder
orderings, native measured observations, retained final measurements,
evaluation/candidate/profile construction, and evaluated apply. They must not
silently become unlimited replacements. Diagnostic raw collection may retain
evidence even when final measured replacement is refused. The actual replay CLI
retains requested intent and a typed `ingress_elapsed_unsupported` reason.

Process source/sample age remains unknown, and its existing age-limit request
remains explicitly refused. Host residence is checked at admission; thread
preemption after that check and later hardware presentation are outside this
bound. This is not a physical latency guarantee or a waveform continuity pass.
Whole-native-drain and whole-wake admission remain required.

## Runtime regression evidence

Logs are under ignored `target/s1-s3-20261028/` in this worktree.
The final read-only reviews inspected source and retained logs; reviewers did
not independently execute native or portable tests.

| Check | Result | Log |
| --- | --- | --- |
| Initial core behavior RED | 5 expected runtime failures, 1 passed | `ingress-elapsed-core-red.log` |
| Queue/guard/replay RED | 10 expected runtime failures, 1 passed | `ingress-elapsed-guards-red.log` |
| Core expanded before review | 9 passed | `ingress-elapsed-core-expanded-final.log` |
| Review clock/mixed-generation RED | 5 expected runtime failures, 10 passed | `ingress-elapsed-review-red.log` |
| Final core integration | 15 passed | `ingress-elapsed-review-green.log` |
| Complete Windows audio crate | 829 passed, 3 ignored, 31 result suites | `ingress-elapsed-audio-reviewed.log` |
| Complete E2E example suite | 338 passed | `ingress-elapsed-probe-reviewed.log` |
| Complete RT harness | 182 passed | `ingress-elapsed-rt-reviewed.log` |
| Strict audio all-target Clippy | Passed | `ingress-elapsed-audio-clippy-reviewed.log` |
| Strict example test-profile Clippy | Passed | `ingress-elapsed-probe-test-clippy-reviewed.log` |
| Strict RT all-target Clippy | Passed | `ingress-elapsed-rt-clippy.log` |
| Linux audio all-target cross-check | Compiled with 3 cfg-specific dead-code warnings | `ingress-elapsed-audio-linux.log` |
| Actual desktop selector Save/Restart clicks | 2 passed | `ingress-elapsed-app-click-final-green.log` |
| Desktop app library with gpui-test | 526 passed, 3 ignored | `ingress-elapsed-app-gpui-lib-final.log` |
| Full app with gpui-test, integration and doc tests after localization | 589 passed, 4 ignored, 13 result suites | `ingress-elapsed-app-full-final.log` |
| Strict app all-target Clippy | Passed | `ingress-elapsed-app-clippy-normal-final.log` |
| Strict app all-target gpui-test Clippy | Passed | `ingress-elapsed-app-clippy-gpui-final.log` |
| Root formatting | Passed | `ingress-elapsed-root-fmt-final.log` |
| RT workspace formatting | Passed | `ingress-elapsed-rt-fmt.log` |
| Exported core commit workspace all-target check | Compiled with 1 pre-existing app test-helper warning, resolved by the next app commit | `ingress-elapsed-core-staged-check-final.log` |

The RT regression uses the actual shadow-compiled bridge for 1,000
publication/read/stale-flush/reprime cycles at 44.1/48/96 kHz inside allocation
and deallocation guards. It does not measure real device deadlines.
Linux evidence is cross-compilation, not executed Linux tests.

Two material findings were fixed after independent review. First, the initial
implementation forgot post-read/post-publication clock endpoints and high
pre-read endpoints from rejected brackets. Four deterministic varying-clock
tests now fail before the fix and pass after it. Second, a generation mismatch
could bypass full-prefix freshness recovery. Its regression checks three valid
outputs, five ingress discards, zero reset-held loss, empty prefix inventory,
one recovery, and the subsequent patterned PCM fade. A fresh/disabled control
keeps legacy generation-only recovery unchanged. Final core re-review finds
no remaining material issue within that scope.

The first expanded fixture incorrectly expected six discards instead of five;
its arithmetic was corrected and is not product-failure evidence. Earlier app
Clippy caught an unwired summary helper, and a desktop click test caught missing
diagnostic observation IDs. Both integration gaps were fixed before final
passes. Root formatting initially caught the newly added native-preparation
test layout; scoped rustfmt corrected it. Failed attempts remain in the logs.
The explicit-locale regression first failed with untranslated English; editing
only YAML initially reused Cargo's compiled locale bundle. The final full-app
run rebuilt the bundle and includes the passing exact-integer/placeholder
regression, increasing the app library total to 527 passed.
The first proposed commit split did not compile because the new typed fields
and replay error also required the app draft merge and probe replay consumer.
Those dependencies moved into the core/API commit, and its exported staged
tree passed the full workspace all-target check. This was a commit-boundary
correction, not an additional product regression.

Independent review notes: `ingress-elapsed-core-independent-review.md`,
`ingress-elapsed-app-independent-review.md`,
`ingress-elapsed-guards-final-review.md`, and the queue report in the same
ignored evidence directory. Final reviews cover core freshness, loss
conservation/export, measured refusal boundaries, app behavior, and localization.

## Delivery and remaining gates

Source is preserved in two coherent local commits. Batch remote updates at
meaningful verified checkpoints per the user's 2026-10-29 instruction; this
slice performs no push. Unrelated `gate.log` remains untouched.

No VM access, native audio run, driver installation/deployment, remote CI, or
Miri execution occurs in this slice. Production queue defaults and validation
grace remain unchanged. The diagnostic 0..3000 ms correctness grace is not an
audio latency allowance or production queue setting.

Actual QPC/device behavior, measured-versus-declared delivery under load,
source-age provenance, sustained future-safe admission, actual lower native
period acceptance, matched physical routes, and stable real IO/Bus output
below 10 ms remain open or target-dependent. Passing host tests does not close
those gates; VM timing qualification remains deferred.
