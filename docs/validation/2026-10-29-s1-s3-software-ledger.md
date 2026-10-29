# S1-S3 software status and evidence boundaries

Date: 2026-10-29. Worktree: `minimal-io-latency`.
The baseline reconciliation is supplemented by the linked probe repair,
VAD software-route, common physical-render-request and physical IO receipts.
**S1-S3 remain incomplete; stable output below 10 ms is not established.**

Current remote build policy: [retain reusable caches](2026-10-29-remote-build-cache-policy.md).
Do not clean after each build, deployment or archive; inspect disk pressure
and reclaim only justified obsolete artifacts. Earlier cleanup receipts below
record history, not a recurring requirement or a completion gate.

## Physical-target follow-up

The [physical deployment receipt](2026-10-29-physical-target-deployment.md)
records authorized work on `LAPTOP-NP49RG6H` / `192.168.12.97`, separate from
the deferred VM. It supersedes baseline statements that no native route has
been acquired: exact VAD software-route recordings and cleanup now complete,
including explicit 2048/1024 operating targets with independent capacity.
They still fail waveform/provider quality and are not physical latency results.
Those early physical-render runs used muted Realtek. The later
[DAWN follow-up](2026-10-30-dawn-native-process.md) has a user-authorized,
explicitly pinned headphone output; no other speaker state is changed.
An independent calibrated common reference remains unavailable; no path
winner or smaller default is selected.

The DAWN receipt now supersedes the original native-confirmation blocker:
merged-main app3ac8484 passes68 measured/UI release tests and a real production
desktop Process Apply/restart with exact requested/active UIA confirmation,
generation2 cleanup and normal Stop/Quit. This closes that baseline S3 desktop
integration gap, not measured delivery feasibility at smaller targets. Its
diagnostic4096-frame target is not a low-latency preset. A separate30-second
VAD -> Bus -> DAWN diagnostic completes submission and cleanup with zero
reported in-run provider loss/recovery and playback xruns/short reads. It has
no acquired PCM or calibrated reference and remains explicitly unqualified.
Original native failures are preserved, not relabeled as successful runs.

The [native packet-period / Engine-block matrix](2026-10-29-vad-period-block-matrix.md)
adds observed144/480-frame VAD playback packets with128/256-frame Engine
blocks at explicit target1024/capacity4800. Three cases complete operation
and cleanup; all four retain recorder quality failures. The fourth also
retains microphone drop/silence and a premature probe failure: execution is
Running with a null optional timeline, but health calls it stopped. Its
original collection/cleanup failures are preserved separately from successful
underlying native retirement. This software-route matrix is not calibrated
physical delay, lower-default qualification or S1-S3 completion.

The [progress-health repair and retest](2026-10-29-vad-engine-progress-health.md)
separates confirmed execution from optional timeline data and bounds renewed
identity-matched progress without fabricating fresh observations. Final509
probe tests and strict Clippy/fmt pass; the unchanged audio suite passes899 /
3 ignored. Only the repaired probe is redeployed. A single480-packet/128-block
native retest completes collection/cleanup and the240,000-frame schedule, but
retains the recorder's first-packet discontinuity and qualityfalse. No transient
null timeline recurs in that run; deterministic RED/GREEN owns that branch's
regression proof. Driver/app remain unchanged and ~0.85 GiB reusable build
outputs are retained, not cleaned after archiving.

The [retained-waveform analysis](2026-10-29-vad-retained-waveform-analysis.md)
runs the unchanged analyzer on that completed recording with explicitly chosen
diagnostic confidence settings, not a previously qualified production gate.
All three marker windows refuse confidence, and 487 later packet-QPC intervals
violate the existing diagnostic policy despite complete stored coverage and
adjacent device positions. Neither establishes waveform corruption, measured
PCM loss or physical delay. Fresh read-only catalog again confirms muted/volume0
Realtek output; common physical reference/calibration remains unavailable.
This adds waveform evidence without rebuilding, repeating the buffer matrix,
reinstalling the driver, cleaning caches or claiming S1-S3 complete.

The [2026-10-30 native desktop controls receipt](2026-10-30-native-desktop-controls.md)
adds real UI Automation Stop, settings navigation, targeted scrolling and
normal Quit on deployed app `e195bfe` in the physical target's console session.
Native owner cleanup/reaping and final process exit 0 are verified. Hidden
startup and off-viewport invocation mistakes remain retained harness failures;
the final run scrolls Quit fully into view. The black hidden-window image is
not visual evidence. No Process budget Apply, native selection confirmation,
waveform or latency pass is inferred. All eight terminal helper tasks are
removed after archival; Cargo caches, deployed binaries and driver are retained.

Three bounded software repairs follow those native observations:

- `792f99e` accepts the Engine's legitimate initial resource generation 0 in
  the physical collector, pins that exact value, and still refuses subsequent
  identity/resource/plan changes. Focused physical regressions: 39 passed.
- `a64739d`, completed by review repair `45a084e`, retains raw playback totals,
  an explicitly completed startup snapshot, checked steady deltas and stopped
  final conservation. The source publishes completion of the original prime;
  a multi-callback initial mute is not frozen at its first publication, and
  an interrupted initial prime followed by recovery cannot satisfy completion.
  A contended one-attempt live read remains unavailable, with prior evidence
  retained as historical; stopped final evidence is still mandatory. No fixed
  32-frame allowance, recovery/short-read/xrun erasure or invented classification
  is used. The final integrated probe suite passes 499 tests on the target.
- `2ad3df3` separates Process packet processing safety from an untrustworthy
  pre-first-event `GetBufferSize` result. Raw query evidence remains; Process
  capacity/period remain unknown. Actual packets are bounded before borrowing,
  with the existing 8-MiB limit, complete lease release and retained dual faults.
  Physical native-capacity admission is unchanged. CaptureReport is schema 11;
  the final library suite passes 607 / 2 ignored. This is not observed native
  delivery admission or a buffer-latency claim.

The latter decision is corroborated by Chromium's documented Windows Process
Loopback pre-first-event query bug in
`media/audio/win/audio_low_latency_input_win.cc` at upstream revision
`623c32009e03672597ac3967474a7f8e8a87b1ce` (crbug.com/411452039). The patch does
not start a source ahead of the activation gate to obtain a capacity.
Native retesting and integrated test counts belong to their own later
receipts; the focused passes do not relabel prior failed runs as successful.

The real native desktop idle renderer/catalog capture also completes with
26 screenshots, using isolated preferences and no audio Start. It does not
prove desktop live-window Apply, actual desktop audio or physical latency.
Suitable integrated tests/builds now move to a separate target worktree;
the clean remote main remains untouched. Target/temp/download caches stay on
D:, one Cargo target is reused, space is checked between phases, and only idle
verified Cargo outputs are cleaned. Audio acquisition never overlaps heavy
builds. The first explicit remote clean reclaims 1.1 GiB without deleting
driver rollback or measurement evidence.

Final native repair retesting at source `45a084e` now completes collection
and cleanup for physical IO, both VAD software targets and owned Process
Compatible. The real app Compatible monitor also exits 0; measured targets
1024/512/256 refuse the actual observed budget rather than expanding it.
The physical route and independent recordings still fail quality. VAD has
explicitly completed initial prime and zero classified steady deltas, but
recorder discontinuities remain; Process artifact position gaps with repeated
zero device position remain unresolved evidence semantics. None is promoted
to a stable preset, physical latency distribution or path comparison.

The final remote release is deployed with verified hashes and all 173 evidence
files are archived and host-verified. An idle explicit shared-target
`cargo clean` then removes 12,234 files / 6.1 GiB; final target bytes are 0.
Deployed binaries, driver and rollback, source worktrees and evidence remain.
The linked receipt records the failed FXC auto-discovery, single-path task
override, successful build retry, exact native outcomes and post-clean checks.

The subsequent [Process artifact applicability repair](2026-10-29-process-artifact-position.md),
source `5382369`, supersedes only the unresolved position-gap semantics above.
CaptureReport schema 12 / artifact metadata 2 explicitly mark Process position
checks inapplicable while retaining actual timestamp/native/overflow faults.
Fresh native Process4 captures confirm repeated position 0 no longer creates
534 synthetic gaps: Compatible and explicit 2048 complete collection/cleanup
with zero artifact gaps and bridge underruns/drops/resets. Both independent
recordings still retain one discontinuity and fail quality. Explicit 1536/1024
refuse the complete native-wake budget before capture; their missing/not-started
receipts still report cleanup false. Neither refusal is a successful run.
The separate probe deployment and all 128 evidence files are host-verified;
the application was still at `45a084e` at that Process4 receipt. The later
`e195bfe` software verification does not itself update that deployed copy;
release, deployment, smoke, archival and cleanup have separate receipts.
The earlier post-clean zero-target receipt is historical: the shared target is
reused for this next bounded build/test phase. Subsequent cleanup receipts
record what occurred, but the current retention policy no longer requires
another idle clean after copies or archival. No new physical latency claim follows.

The later [desktop integration/deployment receipt](2026-10-29-desktop-live-process-window.md)
records clean source `e195bfe`, all630 app tests /4 ignored, strict Clippy and
committed7-test confirmation, followed by the successful release build and
hash-verified app replacement. Fresh Compatible monitor completes run/cleanup;
explicit live-window target2048 refuses the actual complete-wake budget with
cleanup complete and no fallback. All121 final evidence files are host-verified.
The subsequent idle shared-target clean removes12,280 files /6.1GiB and leaves
target bytes0, D free272,451,121,152 bytes; final deployed hashes/driver remain
verified and no test/build process is left running. That cache-discard
decision is historical and superseded by the current retention policy.

The [native follow-up](2026-10-29-native-process-followup.md) isolates the
recorder's first-packet raw discontinuity in three standalone baselines and
retains exact Shared/Exclusive128 preparation refusals without bypassing them.
Actual app per-wake and explicitly selected live-window4096 requests now
activate in a new native-backed session and complete bounded run/cleanup.
Later gap-envelope violations remain visible; the finite replay is not a
future-service or physical-quality guarantee. This supersedes only the earlier
absence of successful native window activation, not the original2048 refusal,
desktop-click acceptance, recorder quality or missing calibrated reference.
All127 files in the two new archives are host-verified. No build or cache clean
occurs in this follow-up; default, volume and mute settings remain unchanged.

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
S1 remains partial because the acquired native recordings do not establish
clean waveform continuity, a common pre-route reference or calibrated delay.
Selected native routes now have acceptance or refusal receipts; physical-input
orchestration and common physical-render controls have separate receipts below
without qualifying new lower settings.
The exact virtual IO/Bus
collector now has the separate bounded software receipt below. S2's bounded
software mechanisms are present under their
declared/admitted conditions; native/lower-setting qualification remains open.
S3's original bounded configuration/replacement paths exist in the actual
desktop and CLI. The separately selected [desktop live-window controls](2026-10-29-desktop-live-process-window.md)
are now implemented in `e195bfe`, with fresh full app/Clippy verification and
independent review and verified deployment; the later explicit4096 diagnostic
has native CLI/service activation evidence, while2048 still refuses and native
desktop-click acceptance remains unproved. Bounded
live-window service/CLI
admission, the live-prefix core/native API and audited probe repair have
separate software receipts below. Checkpointed long windows, combining
freshness with replay, and authentic source-time provenance are unsupported
extensions, not additional prerequisites silently added to the original S3
scope. Their refusal boundaries remain binding and must stay visible.

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
| S3 live-prefix assessment | The [bounded live Process prefix core and pinned native API](2026-10-29-live-process-prefix.md), source `5df48d0`, seal and replay a sequence-one prefix through the production queue while actual writers remain open. Reports preserve explicit policy, origin, actual closure, refusal and observed loss. | Diagnostic reports are not Apply tokens. The separately linked service/CLI slice adds opaque native admission; checkpointed continuation beyond8192 events remains open. No future service or physical latency guarantee follows. |
| S3 live-window application admission | The [explicit service/CLI workflow](2026-10-29-windowed-process-admission.md), sources `88853ab`/`75481a8`, freezes opaque native measured evidence, replays on the bounded Prepare worker and binds successful policy/options to owner replacement with source/settings/predecessor fences. The [desktop opt-in page](2026-10-29-desktop-live-process-window.md), source `e195bfe`, requires all five policy choices and exact asynchronous confirmation. Fresh full app suite 630 passed /4 ignored; strict Clippy passed. | Existing per-wake and stopped Apply remain separate. Either 256-event measured history overwriting refuses even below the 8192-event archive cap. Longer checkpointed windows and unsupported policy combinations are explicit limits, not hidden fallback or extra baseline scope. No native/physical qualification follows. |
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

### S3 probe demand and bounded capability limits

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
- **Observed-window demand/delivery density admission - bounded core and
  service/CLI implemented:** the [live-prefix receipt](2026-10-29-live-process-prefix.md)
  records actual ordered production-queue replay with preserved held/SRC/servo
  state, explicit exposure and feasible/inconclusive/violation outcomes for a
  sealed sequence-one prefix. Its regression detects repeated demand that
  exhausts supply even though each individual wake fits. The pinned native API
  retains its original source/generation; editable archives remain diagnostic.
  The [service/CLI receipt](2026-10-29-windowed-process-admission.md) adds a
  separate opt-in request with all five explicit policy fields, opaque native
  preparation, background replay and exact source/settings/rate/whole-wake
  demand binding before replacement. Joined predecessor and captured settings
  are rechecked before opening; later mutating control requests invalidate
  in-flight work. `RefreshCatalog` is deliberately non-cancelling, and its
  later cumulative receipt is neither cancellation nor activation.
  Per-wake measured selection and stopped retained evaluation/Apply remain
  separate workflows. The [desktop live-window controls](2026-10-29-desktop-live-process-window.md)
  now dispatch that exact existing request after explicit opt-in and all five
  policy choices, preserving original-source and requested/active guards.
  Measurement is limited by256 complete-drain/whole-wake history events;
  overwrite refuses instead of using a suffix. The packet/read archive is
  capped at8192 events per writer. Longer continuously checkpointed windows
  are an unimplemented extension, not promised support or a new baseline S3
  completion gate. Any future extension must carry actual checkpoint state,
  not invent an empty/full queue at each chunk boundary.
- **Freshness composition:** measured/evaluated/replay paths honestly refuse
  enabled host-ingress elapsed intent. They do not yet combine the independent
  features. Current queue histories retain the outside read clock endpoints,
  not the middle post-acquisition freshness-decision clock and acquired-prefix
  provenance. These combinations are outside current supported capability;
  the original S3 does not require every independent option's combination.
  Any future support must version/retain the required evidence and reproduce
  live discard/prime/reset decisions before lifting guards, preserving exact
  policy and distinct ingress-expiry losses and refusing old/ambiguous traces.
- **Source/sample-age provenance:** positionless Process publication observations
  do not establish the source application's sample timestamp. A raw stamp
  integrity diagnostic may reject zero/error/regression/discontinuity, but
  cannot enable physical position correlation or manufacture source age.
  Existing sample-age intent remains explicitly refused.

The original S1-S3 baseline does not require unbounded history, all policy
combinations or unavailable source clocks. Honest refusal is an implemented
boundary, not a reason to manufacture evidence or continually expand this
task. A runtime budget violation or loss must still be reported; none of
these scope distinctions weakens actual admission or physical quality gates.

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

The bounded live-window integration separately records612 app passed /4 ignored,
851 audio passed /3 ignored,471 probe passed and201 RT harness passed, strict
app/audio Clippy and Linux audio/RT compile checks. Its receipt preserves the
initial unrelated UI timeout and passing isolated/full reruns without threshold
changes, the repaired RT shadow compile failure and Clippy fixes. Batch remote
delivery is recorded separately; individual source slices do not each push.
