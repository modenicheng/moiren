# Common physical render mode and period request

Date: 2026-10-29 (Asia/Shanghai). Source: `32b537d66d085437b84f2e1e186afd3b9516cef0`.
Scope: the E2E probe's ordinary Process Loopback, owned Process collection and
VAD physical-output paths. This is a software capability/refusal receipt, not
native device acceptance or a latency qualification.

## Implemented controls

Both physical-output probe commands accept:

```text
--render-mode shared|exclusive
--render-period-frames 64|128|256|512|1024|2048|4096
```

Either flag may be supplied independently. Omission preserves the existing
Shared/default-period behavior and each caller's sample-format/rate/clock-role
policy. The Process owner still requests Float32 at 48 kHz and requires actual
48 kHz stereo; the VAD physical sink retains its existing Native-format and
Master-clock configuration. These controls do not change the external VAD
source: `--vad-minimum-period` is still a separate source-client request.

The common portable request applies only mode and period to DeviceOptions.
Both Process callers use the same RenderOptions constructor; the actual VAD
project helper configures its one physical Master sink. The virtual
playback -> Bus -> microphone route is unchanged and refuses explicit physical
render controls. No Gain, extra graph node, different queue target or new
production default is introduced.

Duplicates, invalid values, unsupported backend period presets and Exclusive
target-queue combinations fail before artifact creation or native preparation.
Exclusive requires full-capacity rendering; capacity, Engine block size, render
queue target and native period remain distinct concepts.

## Acceptance and evidence

- An explicit period requires actual nonzero native timing with the exact
  requested period. Capacity or requested metadata alone is insufficient.
- Shared additionally requires observed Client3 initialization, exact requested
  and observed periods, and no format conversion, compatibility fallback,
  Client3 failure, period-selection failure or failed actual-period query.
  Production WASAPI compatibility behavior is not changed; the probe refuses
  its result instead of silently calling it exact acceptance.
- Exclusive additionally requires that actual buffer capacity equals the
  requested period and no Shared negotiation record is presented. Unsupported
  or aligned-to-different explicit requests are not retried as successful
  settings. Existing native errors remain visible; an open failure does not
  manufacture a negotiated record that the backend never returned.
- Process retains request/timing/shared-period evidence before refusal, before
  readiness, capture preparation, attachment or Start. Exclusive has no Shared
  padding observer; absence remains None/null rather than fabricated zero.
- VAD validates after the host reports Running but before active-revision gate
  admission/source activation. It requires current running owner-generation
  evidence and exactly the configured physical MasterOutput node. Missing,
  ambiguous, wrong-node/kind, stale or unknown timing observations refuse.
  This does not claim the physical native owner was never started: EngineHost
  already owns that native lifecycle when the VAD gates are checked.
- `native.engine.physical_render_admission` retains an immutable pre-gate
  host/native snapshot, expected output node, request, actual validation verdict
  and nullable failure. Later health/final observations may advance or become
  null without erasing that decision. Acceptance means the physical render
  request passed validation, not that subsequent gates opened or PCM succeeded.

The live report and manifest retain `physical_render_request` for physical
routes even on preparation failure. Its omitted values serialize as null,
distinct from an explicit Shared choice. The microphone route omits the field.
Existing negotiated evidence stays separate; none of these request fields is
an acquired waveform, independent reference, sample-age timestamp or measured
whole-route latency. All dry runs remain unqualified.

## Fresh verification

Ignored logs and frozen source hashes are under `target/s1-s3-20261028/`.

| Check | Result / log |
| --- | --- |
| Parser behavioral RED | Missing flags/request artifact: 2 passed / 2 failed, `common-render-parser-red.log` |
| Helper behavioral RED | No-op apply/validation/observations: 4 passed / 5 failed, `common-render-helper-red.log` |
| Actual caller/preparation behavioral RED | 9 passed / 7 failed, `common-render-wiring-red.log` |
| Admission snapshot retention RED | 2 runtime failures, `common-render-admission-retention-red.log`; status-specific failures retained separately by the wiring report |
| Focused final physical-render suite | 18 passed, `common-render-admission-retention-green.log`; legacy virtual-schema regression separately passed |
| Complete Windows probe suite, final source | 442 passed, `common-render-full-final.log` |
| Audio library/integration/doc regression | 831 passed / 3 ignored, 31 result summaries, `common-render-audio.log` |
| Strict audio all-target Clippy | Passed, `common-render-clippy-final-all.log` |
| Strict actual example test-profile Clippy | Passed, `common-render-clippy-final-tests.log` |
| Workspace fmt / diff | Passed, `common-render-fmt-final.log` and pre-commit diff checks |
| Linux normal / actual example test-profile compilation | Passed, `common-render-linux-final-{normal,tests}.log`; 1 normal / 4 test cfg-specific existing dead-code warnings |

The Linux checks compiled the example with and without its test cfg; they did
not execute tests or native audio on Linux. Earlier receipts used a different
combined test-selection command and their warning counts are not relabeled.
No Miri, app suite or remote CI execution is claimed for this example-only
slice. The audio count is portable/host regression coverage, not 831 independent
allocation tests or any hardware acceptance.

Independent parser/helper review found no concrete remaining defect. Native
wiring review found admission evidence could be overwritten; the fix had
genuine RED -> GREEN tests for both accepted/refused decisions followed by
changed and null observations. Re-review found no remaining concrete scoped
finding. Review logs are not native qualification.

## Remaining gates and delivery

This closes S1's common physical render request forwarding/refusal software gap.
It does not establish matched native settings: both routes still need the same
actual output device/mode/period plus declared source-client mode/rate/buffer,
format, clock roles and load. Actual lower-setting acceptance and aligned
native shapes need a separately authorized suitable target.

S1 physical-input-through-graph orchestration, acquired changing PCM and an
independent calibrated physical reference remain open. S3 observed-window
supply/demand density admission, freshness/provenance composition and unknown
source sample age remain open. S2 native/lower-setting qualification and stable
sub-10-ms real output are not established by this receipt.

No native audio, VM access, driver installation/deployment, deletion of guest
files, push or remote CI trigger occurred. Source and this receipt/plan/ledger
are separate local atomic commits. Keep remote updates batched at meaningful
verified integration checkpoints. Unrelated `gate.log` remains untouched.
