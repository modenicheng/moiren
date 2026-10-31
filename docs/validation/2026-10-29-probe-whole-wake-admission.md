# E2E Process whole-wake preparation and retained failure evidence

Date: 2026-10-29. This receipt covers host software changes, not native audio.
**S1-S3 remain incomplete; stable IO/Bus output below 10 ms is not established.**

## Defect and correction

The ordinary `live-process-loopback` and owned Process collection paths prepared capture
using `engine_block_frames` before discovering the native render geometry. The
renderer splits one native wake into Engine-sized reads; those small reads do
not reduce the startup/whole-wake consumption burst. A target could therefore
pass segment-only preflight while lacking reserve for the actual native wake.

Both callers now prepare the gated renderer first. Its owner publishes the
actual `NativeStreamTiming` and `PreparedRenderQueue`. Before preparing capture,
the shared control-side admission helper validates that geometry and binds
`ProcessBridgeOptions::for_consumer(48000, queue.target_frames)`. The probe
renderer is fixed-48k without the production Pump's separate SRC/prefetch
rounding, so the actual queue target is its complete consumption bound. The
production backend's different overhead is not copied into the probe.

Input target/capacity/jitter, independently declared or observed delivery,
source-age intent and host-ingress elapsed policy remain unchanged. Render
period never becomes Process delivery cadence. Invalid geometry or insufficient
reserve refuses before capture or activation, without inflating a target.

`native.process_admission` separately retains the preliminary segment request,
observed native geometry, admitted complete-wake options and admission/capture
failure stage. These are preparation observations, not successful streaming or
physical latency. The manifest's preliminary `process_bridge` is not actual
native admission. Trace archive demand retains the admitted complete wake while
individual source reads retain their real segmented demand. Export remains
unqualified and does not certify cleanup, sustained service or sample age.

## Ownership and failures

- Admission/capture refusal disconnects and retires a parked render owner;
  joining it while attachment is still connected would block.
- Prepared-render cancellation attempts both stop and join and retains an
  available report even when stop fails. Both errors remain visible.
- Owned capture startup abort broadcasts peer cancellation before retiring
  render. A panicking peer hook still retires render and retains panic evidence.
- Ordinary graph construction, attachment, gate activation, load startup and
  collection Result exits all reach explicit stop/joins, partial report
  projection, trace export and journal finish. Serialization/persistence errors
  are combined rather than suppressing subsequent retirement/evidence attempts.
- Review found a pre-existing legacy capture-join hole: an after-ready Process
  panic could carry a partial report, but `join()` discarded it when returning
  `WorkerPanicked`. New `join_retaining_report()` exposes both independently;
  legacy `join()` remains an error. Both probe callers use the retaining API.
  An unrelated panic produces no fabricated report, and retained reports never
  convert a panic into success. Ordinary artifact-write failure does not erase
  either the report or the panic error.

The native owner work here is control-side preparation/retirement. No production
queue defaults, validation grace, DSP or per-sample RT work changed. Direct
post-capture native build/activation/load failure injection is not executed;
the Result continuation was source-reviewed, while preparation/abort/retention
regressions use actual hardware-free owner seams.

## Executed evidence

Logs live under ignored `target/s1-s3-20261028/` in this worktree. Reviewers
read source and retained logs; they did not independently rerun Cargo/native.

| Check | Result | Log |
| --- | --- | --- |
| Segment-only helper mutation | 8 expected runtime failures | `probe-whole-wake-helper-red.log` |
| Admission-before-capture seam mutation | 9 passed, 2 expected failures | `probe-whole-wake-helper-seam-red.log` |
| Missing preparation evidence mutation | 11 passed, 4 expected failures | `probe-whole-wake-helper-evidence-red.log` |
| Final helper/evidence tests | 15 passed | `probe-whole-wake-helper-evidence-green.log` |
| Ordinary caller segment-only mutation | 3 expected runtime failures | `probe-whole-wake-ordinary-segment-red.log` |
| Graph parked-render cleanup RED | 4 passed, 1 expected failure | `process-graph-cleanup-red.log` |
| Final actual Graph tests | 6 passed | `process-graph-admission-green.log` |
| Render stop/report retention RED/GREEN | 2 expected failures, then 7 passed | `s3-probe-cancel-retention-{red,green}.log` |
| Legacy capture join delegation RED/GREEN | 1 expected failure, then 21 capture tests passed | `s3-capture-join-retention-{red,green}.log` |
| Ordinary panic-report loss mutation | 1 passed, 1 expected failure | `probe-capture-join-caller-red.log` |
| Complete Windows audio crate | 831 passed, 3 ignored, 31 suites | `probe-whole-wake-audio-retaining-final.log` |
| Complete E2E probe example | 367 passed | `probe-whole-wake-all-retaining-final.log` |
| Strict audio all-target Clippy | Passed | `probe-whole-wake-audio-clippy-retaining-final.log` |
| Strict example test-profile Clippy | Passed | `probe-whole-wake-test-clippy-retaining-final.log` |
| Root fmt and whitespace diff | Passed | `probe-whole-wake-fmt-retaining-final.log` |
| Linux example normal/test cross-check | Compiled; 1/4 existing cfg dead-code warnings | `probe-whole-wake-linux-{normal,test}-retaining-final.log` |

The full suite includes all five ordinary caller regressions and the retained
library panic-report regressions. A compiled Input -> Bus -> Output regression
renders native512 as eight64-frame reads while archived prepared demand remains
512. Its clock is intentionally unavailable; replay refuses rather than
inventing native timestamps. The exact-bound fixture admits546 frames
(native512 + interpolation2 + declared gap32) and refuses545. These numbers are
test geometry, not recommended settings or measured delay.

Some early integration builds caught transient API/visibility and serialization
bound mismatches; those were corrected. One early segmentation fixture wrongly
expected replay success without QPC; its expectation was corrected and is not
counted as a product fix. Behavioral RED rows above are runtime assertions, not
compiler failures. Scoped mutation scripts restored the correct source before
the final suite. Independent review: `probe-whole-wake-independent-review.md`;
helper/Graph reports are in the same ignored evidence directory.

## Delivery and remaining work

Keep source/API and documentation changes in coherent local commits. This slice
does not push, merge, deploy, install a driver, access the VM or execute native
audio. It does not trigger remote CI or run Miri, the RT harness or the app suite;
earlier totals remain historical. Unrelated `gate.log` is untouched.

Local source receipts: `a2a275a` retains capture join reports independently of
errors; `2b09537` implements complete-wake probe preparation and both callers.
Documentation is preserved in a separate local checkpoint. Remote tracking was
not refreshed by these commits; no remote head or CI change is claimed.

The exact VAD playback -> Bus -> VAD microphone collection route, common explicit
physical render mode/period selection, physical-input graph collection, live
observed-window demand/delivery admission, freshness/replay composition and
source-time provenance remain separate software/evidence gaps. Actual native
acceptance, waveform continuity with an independent reference, calibrated
physical delay and sustained low-latency qualification remain open. See
[the S1-S3 ledger](2026-10-29-s1-s3-software-ledger.md). VM timing qualification
remains deferred; diagnostic grace up to3000ms is not an audio buffer allowance.
