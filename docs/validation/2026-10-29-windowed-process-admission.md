# Explicit live Process window application admission

Date: 2026-10-29. Sources:

- `88853ab`: opaque native measured-window preparation and candidate.
- `75481a8`: managed application service and explicit monitor CLI admission.

This closes the bounded service/CLI integration of live-prefix evaluation. It
does not complete S3 or establish stable native output, lower default buffers,
source age, future supply/demand balance or physical latency below10ms.

## Behavior and trust boundary

The separate application request `ApplyWindowedProcessBridge { request, policy }`
requires a Running, settled original Process owner. Existing per-wake measured
selection and stopped retained evaluation/Apply remain separate and unchanged.
All five finite-window thresholds are explicit; none has a guessed default.

The service uses its own pinned `SessionTrace`, not UI observation DTOs or
imported JSON. The native collector prepares an opaque frozen archive with
complete-drain delivery and whole-wake consumption evidence from that same
source. Before/after evidence equality and cumulative packet/frame coverage
refuse racing or partially completed drains/wakes. Publication brackets widen
the measured gap conservatively; packet segments cannot replace a complete
drain or whole native wake. Original ingress-elapsed intent remains refused.

The immutable preparation contains no native audio resources. A reserved
completion credit and the bounded Prepare lane run actual bridge replay off
the service and RT threads while the original collection continues polling.
Only a retained-native `NoViolationInWindow` result with actual prefix/replay
can issue the opaque native candidate. Editable reports have no issuance API.

The managed service binds that result to:

- Session/native generations, document, graph and host revisions.
- Exact Process PID plus creation time, node and executing host selector.
- Audio, transport and latency-profile settings; exact options and policy.
- The original private collection instance and selection identity.

A later mutating request invalidates an in-flight evaluation. Stale completions,
worker/completion backpressure, missing evidence, violations and insufficient
exposure do not retire the original owner or publish a new measured selection.
Before retirement, the binding and executing predecessor are rechecked. Before
opening the replacement, captured/current settings and the joined predecessor
generation/revision/clean cleanup receipt are checked again. Both precisions
configure the production evaluated bridge, which preserves the exact replayed
rate/whole-wake demand envelope and refuses growth even if the target could fit.

The requested/active public summary includes the explicit policy, sealed cutoff
and replayed packet/read counts only after successful service admission. The
private proof retains the exact full report. No public summary constructs proof.

## Explicit CLI mode

In addition to the existing `--measure-seconds` and target/capacity/jitter tuple,
all five are required together:

```text
--window-max-initial-unserved-frames N
--window-max-initial-prime-discarded-frames N
--window-min-output-frames N
--window-min-output-reads N
--window-min-output-span-100ns N
```

Frames/span must be positive for the minimum exposure fields; output reads
must be at least2. Initial allowances may be0. Exact u64 values are preserved;
large minimum exposure can be inconclusive, not silently clamped or accepted.
The actual replacement must confirm the exact requested window policy, not
merely the same queue tuple. JSON preserves requested policy even on failure;
`qualified` remains false. Omitting all five preserves per-wake measured mode.

## Bounded limitations

The replay archive remains sequence-one and capped at8192 packet/read events
per writer. **Native replacement admission has a stricter current limit:** its
complete-drain and whole-wake histories are256 events each. Either history
overwriting refuses, even if the larger archive remains complete. A1second
baseline at a sufficiently short native cadence may already exceed this limit.
The CLI reports refusal; it does not use only the surviving suffix or call
baseline elapsed time replay exposure. Continuous retained measurement and
checkpointed replay remain open work, not a reason to relax quality gates.

The desktop settings controls are not newly wired to this live-window request;
the existing desktop per-wake and stopped retained Apply flows remain. Service
API and monitor CLI are the actual new entry points in this slice. Freshness
composition, checkpointed long windows and hardware qualification remain open.

## Verification

Logs are under `target/s1-s3-20261028/` on the build host.

| Check | Final result | Log |
| --- | --- | --- |
| Service window regressions |9 passed within final full app suite; initial7 focused passed | `window-app-final.log`, `window-service-focused-final.log` |
| CLI window regressions |14 passed;41 CLI regression tests passed before lint-only fixes; final app covers them | `window-cli-green.log`, `window-cli-regression.log`, `window-app-final.log` |
| Full app with gpui-test |612 passed,4 ignored,13 summaries; test threads4 | `window-app-final.log` |
| Full audio tests |851 passed,3 ignored,31 summaries | `window-audio-full.log` |
| Actual E2E probe regression |471 passed | `window-probe-final.log` |
| RT shadow harness |201 passed | `window-rt-final.log` |
| Strict app all-target Clippy with gpui-test |Passed | `window-app-clippy-final.log` |
| Strict audio all-target Clippy |Passed | `window-audio-clippy.log` |
| Strict actual probe test-profile Clippy |Passed | `window-probe-clippy.log` |
| Workspace fmt and scoped diff checks |Passed | `window-fmt-final.log` and pre-commit diff checks |
| Linux audio all-target compile |Passed with disclosed warnings | `window-linux-audio.log` |
| Linux RT all-target compile |Passed, warning-free | `window-linux-rt.log` |

Native collector tests use real software bridge histories/private registry and
opaque production APIs, not native audio execution. The service lifecycle
fixture preserves `CallerSupplied` report labels and uses a private cfg(test)
already-issued-proof seam; it cannot authenticate native IO or exist in a
production build. A separate caller-origin regression refuses production-style
issuance. RT counts include shadow unit tests, not201 allocation-only checks.

The Linux audio library-test compile retains the existing cfg-only unused
`with_publication_gap_100ns`, `with_retained_owner_origin` and
`replay_retained_live_window`; the normal probe has unused `progress_grace`.
Linux checks compile only. No Linux execution or app Linux build is claimed.

Earlier failures are retained, not relabeled:

- `window-service-red.log`: missing policy/request/collection interfaces before
  implementation. `window-service-first.log` has compiler/type/visibility fixes.
- `window-service-focused.log`:6 passed/1 failed due the test's omitted dispatch
  of its deferred cleanup; fixed fixture dispatch, then7 focused passed. Two
  additional origin/policy regressions pass in the final full app suite.
- `window-cli-red.log`:3 passed/11 failed before CLI implementation.
- `window-native-red.log`:1 passed/5 failed before native preparation/evaluation.
- `window-rt-full.log`: the new production-backend test did not compile inside
  the source-shadow harness. It moved to the production admission test module;
  assertions remain and the final full audio/RT checks pass.
- `window-app-clippy.log`: large Command/Completion variants and constant
  chunks lint; fixed with scoped boxing and array chunks, not lint suppression.
- `window-fmt.log`: a formatting diff before final workspace formatting.
- `window-app-full.log`:549 passed/1 failed/3 ignored due an existing UI
  confirmation timeout. Exact isolated rerun passes in `window-app-ui-rerun.log`;
  final full run with four test threads passes. No timeout threshold changed.

Newton independently reviewed service transition fences and native evidence
preparation/provenance; no Important/Critical finding remained. Local atomic
commits preserve unrelated `gate.log`. No native audio, VM, driver installation
or deployment is performed. Remote delivery is recorded separately only after
a verified batch push.
