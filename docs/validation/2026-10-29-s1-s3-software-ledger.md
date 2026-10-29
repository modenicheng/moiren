# S1-S3 software status and evidence boundaries

Date: 2026-10-29. Worktree: `minimal-io-latency`.
The baseline reconciliation is supplemented by the linked probe repair,
VAD software-route, common physical-render-request and physical IO receipts.
**S1-S3 remain incomplete; stable output below 10 ms is not established.**

## Basis and interpretation

Requirements are in
[`../vad-low-latency-goals-2026-10-28.md`](../vad-low-latency-goals-2026-10-28.md),
S1-S3. Reconciliation uses the three completion audits under
`target/s1-s3-20261028/{s1,s2,s3}-completion-audit-20261029.md`, the historical
[`2026-10-28-s1-s3.md`](2026-10-28-s1-s3.md), and the linked slice receipts below.
The audits describe baseline `b17929e`. The original reconciliation used no
Git operation to independently verify that baseline or the then-current index;
later slice receipts record their own source revisions and delivery checks.

Older chronological statements that desktop measured selection, retained
evaluation/Apply, or the E2E VAD minimum-period switch are missing are
superseded by their later receipts. Earlier test counts, failed native runs,
package hashes and push receipts remain historical evidence at their recorded
revisions; they are not fresh validation of work now in progress.

"Implemented" below means the bounded software behavior and recorded host
regressions exist. It does not mean native acceptance, a successful acquired
waveform, a calibrated physical delay distribution, or an overall S1-S3 PASS.
S1 remains partial because native acceptance and acquired measurement
evidence are missing. Physical-input orchestration and common physical-render
controls now have separate software receipts below, without native acceptance.
The exact virtual IO/Bus
collector now has the separate bounded software receipt below. S2's bounded
software mechanisms are present under their
declared/admitted conditions; native/lower-setting qualification remains open.
S3 remains partial because live observed-window application admission,
checkpointed long windows, freshness composition and source-time provenance
remain open. The bounded live-prefix core/native API and audited probe repair have
separate bounded software receipt below.

## Completed bounded software slices

| Area | Implemented / recorded software evidence | Boundary of the result |
| --- | --- | --- |
| S1 analysis | Packet-aware changing-marker correlation, bounded parsing, invalidation around loss/reset/silence/unknown clocks, optional rate search, segmented relative-delay trends and known-delay/drift regressions. See [rate-aware analysis](2026-10-29-rate-aware-marker-analysis.md) and [segmented trends](2026-10-29-segmented-marker-trends.md). | Synthetic/model recovery and delay slopes are not physical calibration, native epoch proof or an acquired common reference. |
| S1 common physical render request | [Common mode/period forwarding and exact-observation refusal](2026-10-29-common-physical-render-request.md), source `32b537d`, covers ordinary/owned Process and VAD physical output; final example suite 442 passed. Requests and immutable pre-gate acceptance/refusal snapshots survive later observation changes. | Omission preserves defaults; explicit Shared compatibility/conversion or unknown/mismatched period refuses. Native acceptance, matched A/B settings and actual delay remain unqualified. |
| S1 observations | Bounded packet/queue/wake/padding/reset/loss evidence, VAD sequence diagnostics, and separate [completion service-clock summaries](2026-10-29-vad-completion-clock.md). | Driver transfer-service and host submission-service stamps are not source sample time. Unknown age, hardware provenance and unavailable closure remain unknown. |
| S1 virtual IO/Bus collector | The actual [VAD playback -> Bus -> microphone collector](2026-10-29-vad-software-route.md) now selects two exact owned IDs on one control root, uses the production virtual-realtime graph, records the selected microphone and retires both exports after recorder release. | Host route/identity/health/lifecycle regressions are not native acceptance, acquired PCM, an independent common reference or physical latency qualification. |
| S1 physical IO/Bus collector | The actual [physical Input -> Bus -> Output collector](2026-10-29-physical-io-probe.md), source `f83f3a5`, uses production EngineHost/ProfessionalBackend, independent bounded recording, separate startup/steady supervision and pinned post-join evidence; final probe suite 471 passed. | Post-activation admission and portable lifecycle/health regressions do not establish endpoint acceptance, complete capture quality, acquired changing PCM or calibrated physical delay. |
| S2 operating budget | Independent target/capacity; packet, Engine transfer, FIR, whole wake and bounded recovery admission; playback prime/servo and microphone occupancy/phase/reprime. Requested/admitted values and later actual packet-shape validation are present. See [reprime budget](2026-10-29-microphone-reprime-budget.md) and [app recovery-cap integration](2026-10-29-app-vad-recovery-cap.md). | Conditional on declared delivery/wake bounds and later native-shape admission. Model packet/block/phase tests do not select a lower production setting or prove real scheduling/waveform continuity. |
| S2 short packets | The [short-timer progress fix](2026-10-29-vad-short-timer-progress.md) removes the 48/49-frame repeated zero-progress case, bounds debt by mapped history, and retains one-packet/three-span/one-transfer callback work. | Timely synthetic redue and patterned-copy tests do not prove Windows/provider replenishment at that cadence. Overload still counts/quiesces/rebases. |
| E2E VAD source period | The actual [E2E minimum-period option](2026-10-29-e2e-vad-minimum-period.md), source receipt `1d3e691`, supports `live-vad --vad-minimum-period`, preserves omission/default behavior, refuses invalid selection without fallback, and retains alignment/partial-failure evidence. | The 15 focused passing period tests establish host selection/evidence behavior, not actual device acceptance or packet cadence. This option is for the external VAD source, not physical render mode/period. |
| S3 app/API | Validated independent target/capacity/jitter through API, declared and two-phase measured monitor CLI, actual desktop measured selection, prepared owner replacement and truthful requested/active confirmation. See [monitor CLI](2026-10-29-process-monitor-cli.md) and [desktop measured selection](2026-10-29-desktop-measured-process.md). | Desktop/service regressions are distinct from CLI coverage. Experimental 1024/512/256-frame candidates are not qualified presets. Source/sample-age requests are explicitly refused. |
| S3 retained evaluation | Pinned exact-prefix collection, bounded ordered replay/inventory, finite-window assessment, conservative final whole-drain/whole-wake measurement and opaque retained-native candidate with explicit desktop Apply. See [retained evaluation/Apply](2026-10-29-retained-process-apply.md). | This software workflow is present, not a missing UI button. Imported JSON remains caller-supplied; finite historical approval does not authenticate future service or physical latency. |
| S3 live-prefix assessment | The [bounded live Process prefix core and pinned native API](2026-10-29-live-process-prefix.md), source `5df48d0`, seal and replay a sequence-one prefix through the production queue while actual writers remain open. Reports preserve explicit policy, origin, actual closure, refusal and observed loss. | Diagnostic reports are not Apply tokens. App/CLI admission and checkpointed continuation beyond 8192 events per writer remain open; no future service or physical latency guarantee follows. |
| S3 production wake reserve | The production backend/Pump admits and publishes complete native-wake demand independently of segmented source reads. See [whole-wake regression](2026-10-29-process-whole-wake.md). | That production receipt does not certify the two separate E2E probe preparation callers; their audit finding is tracked below. |
| S3 host residence | Optional positive [host-ingress elapsed enforcement](2026-10-29-process-ingress-elapsed.md), finite-prefix recovery, separate loss accounting, exact app persistence/selector/CLI forwarding and requested/active state. `None` preserves compatibility. | This is time since publication began, not source age or hardware presentation. Measured/evaluated/replay combinations currently refuse enabled policy explicitly rather than clearing it. |

## Exact remaining software integration

### S1 route and experiment controls

1. **Exact VAD playback -> Bus -> VAD microphone collector - bounded software
   integration implemented.** The [route receipt](2026-10-29-vad-software-route.md)
   supersedes the baseline missing-route finding. The actual E2E harness now
   assembles this separate required route without Gain or a dummy physical
   master, resolves the exact owned recorder on the same root, retains paired
   admission/health/cleanup evidence and releases its native recorder before
   export removal. Hardware-free injected-failure/identity/startup tests do not
   prove native acceptance, acquired waveform or calibrated route delay; those
   gates remain open for a separately authorized suitable target.
2. **Common explicit physical-render mode/period software gap closed.** The
   [new receipt](2026-10-29-common-physical-render-request.md) records one
   validated request through ordinary Process, owned Process and VAD physical
   output. It preserves omitted defaults, requested versus actual evidence,
   exact-observation refusal and an immutable VAD pre-gate decision snapshot.
   Queue target, capacity and Engine block size remain distinct from native
   period; `--vad-minimum-period` controls the source only. Actual target
   acceptance and matched A/B conditions remain open, not a new native PASS.
3. **Physical-input-through-graph orchestration software gap closed.** The
   [physical IO receipt](2026-10-29-physical-io-probe.md) supersedes the baseline
   missing-route finding. `live-physical` builds the exact production
   Input(Follower) -> Bus -> Output(Master) route with independent recording,
   separate direction requests, bounded startup/full steady duration and
   pinned native/professional evidence after host join. This goes beyond
   standalone `record-measurement`. Actual endpoint acceptance, changing PCM,
   common reference and calibrated physical measurements remain open.

A matched A/B must also retain source application mode/rate/buffer, the same
physical output device/mode/period, graph, clock roles and load, and disclose
the VAD routing change. The audit records Shared Process source versus
Exclusive VAD source; sharing output ID and nominal rate alone does not make
those historical runs a fully matched comparison.

### S3 probe demand, observed windows and policy composition

- **Actual E2E probe whole-wake preparation - software repair complete:** both
  ordinary and owned Process callers now prepare gated render first and admit
  capture against its actual prepared queue target, not one Engine segment.
  Full-demand archives, refusal-before-capture, parked-owner cancellation,
  partial-report retention and panic-report error semantics have hardware-free
  regressions. The [repair receipt](2026-10-29-probe-whole-wake-admission.md)
  records 367 passing example tests and 831 passing audio tests / 3 ignored.
  These close the audited software defect, not actual native acceptance or
  waveform qualification. The simpler probe does not copy the production
  Pump's distinct prefetch/rounding reserve.
- **Observed-window demand/delivery density admission - core implemented,
  application integration remains:** the [live-prefix receipt](2026-10-29-live-process-prefix.md)
  records actual ordered production-queue replay with preserved held/SRC/servo
  state, explicit exposure and feasible/inconclusive/violation outcomes for a
  sealed sequence-one prefix. Its regression detects repeated demand that
  exhausts supply even though each individual wake fits. The pinned native API
  retains its original source/generation; editable archives remain diagnostic.
  Live measured app/CLI admission still checks per-wake envelopes. It needs
  explicit window policy and opaque evidence binding, with exact source,
  settings, rate and demand rechecks before owner replacement. Existing stopped
  retained evaluation/Apply remains a separate implemented workflow. The new
  bounded core refuses capped/incomplete evidence. Continuation beyond 8192
  events per writer still needs checkpoint state rather than an invented
  empty/full queue at each chunk boundary.
- **Freshness composition:** measured/evaluated/replay paths honestly refuse
  enabled host-ingress elapsed intent. They do not yet combine the independent
  features. Current queue histories retain the outside read clock endpoints,
  not the middle post-acquisition freshness-decision clock and acquired-prefix
  provenance. Version and retain the required evidence and reproduce live
  discard/prime/reset decisions before lifting guards; preserve the exact
  policy and distinct ingress-expiry losses and refuse old/ambiguous traces.
- **Source/sample-age provenance:** positionless Process publication observations
  do not establish the source application's sample timestamp. A raw stamp
  integrity diagnostic may reject zero/error/regression/discontinuity, but
  cannot enable physical position correlation or manufacture source age.
  Existing sample-age intent remains explicitly refused.

These are concrete bounded integration/capability gaps, not a requirement to
prove all future Windows scheduling. A runtime budget violation or loss must
still be reported; rejecting a universal guarantee does not weaken an actual
admission invariant or qualification gate.

## Missing native, waveform and physical evidence

- **Native acceptance:** actual selected/aligned periods, accepted buffers,
  packet shapes, delivery/gap/phase under load and worker/driver service on the
  declared target remain unverified for new lower settings. A request is not
  negotiation; a model-admitted shape is not an observed native shape.
- **Acquired waveform and reference:** each exact route still needs retained
  changing PCM, packet continuity/loss/reset evidence, actual endpoints and
  known phase-to-recorder mapping where available. The same waveform on two
  output channels is not an independent pre-route reference: zero interchannel
  offset can coexist with a large whole-route delay. Submission indices and
  nominal-rate subtraction cannot replace a capture-aligned reference.
- **Calibrated physical measurement:** a documented common-reference setup,
  uncertainty, independent observation count, p50/p95/p99/max and startup versus
  steady-state reporting are still required. The separate VAD software route
  must never be promoted to acoustic/physical E2E evidence.
- **Historical failures:** the archived Process collection is incomplete with
  cleanup complete; the corrected-ID VAD collection and cleanup are incomplete.
  Both failed recorder continuity and neither yields a valid latency
  distribution. Preserve their faults/artifacts; do not assume an environmental
  cause, erase them or declare a path winner from later host tests.
- **Sustained empirical qualification:** S5's repeated-duration baseline/load/
  fault-injection and real non-silent waveform requirements remain separate.
  No lower preset/default, lossless native output or stable sub-10-ms result
  follows from a finite passing software suite. The requirements explicitly do
  not promise a hard Windows scheduling guarantee.

The VM remains a functional/failure target, not a calibrated physical reference;
VM timing/stability qualification is deferred by user direction. Later native
work requires separately authorized target operations and a suitable declared
measurement setup. Do not install the test driver on the development host to
bypass that boundary or retry VM timing qualification here.

## Verification and delivery policy

Recorded receipts, not rerun by this reconciliation: the short-timer slice has
10 driver host suites plus analyzed WDK/package/INF evidence; the E2E period
slice has 15 focused and 337 full example tests. The ingress slice records
829 audio passed / 3 ignored, 338 probe passed, 182 RT passed and 589 full app
passed / 4 ignored. These belong to their recorded revisions. RT totals
include shadow/unit tests, not 182 separate allocation-only checks; compilation
cross-checks are not executed Linux or native tests. No such result certifies
the newer probe repair by themselves. Its separate fresh receipt records the
final suite and review; earlier counts are not silently relabeled as current.

Correctness mode keeps timing/quality diagnostics with configurable 0..3000 ms
extra progress grace; latency-sensitive mode remains fail-fast. Grace is not
permitted audio latency or a production buffer change. Identity, schema,
resource, native IO, storage and cleanup failures remain hard errors.

Keep each coherent source/documentation slice in a local atomic commit and
combine remote updates at meaningful verified integration checkpoints; verify
the remote SHA at those batch checkpoints, not after every small slice.
Older immediate-push receipts are history, not the current delivery policy.
The baseline docs reconciliation did not rerun tests; the linked probe repair
and route receipts record their own fresh Cargo checks and local delivery.
The physical IO receipt separately records 471 final probe tests, 831 audio
tests / 3 ignored, strict Clippy and Linux compile-only checks, including the
repaired test-profile portability failure; these are not native results.
The live-prefix receipt separately records 842 audio passed / 3 ignored,
471 probe passed and 193 RT harness passed, strict Windows Clippy and Linux
compile-only checks with the disclosed cfg warnings and repaired RT compile
failure. None performs native audio, VM or driver operations or pushes. Unrelated
`gate.log` remains untouched.
