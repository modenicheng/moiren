# VAD playback -> Bus -> microphone software collector

Date: 2026-10-29. Baseline: `080712f`. Source: `0d7dd81`.
The S1-S3 objective remains active. This receipt closes the bounded software
orchestration gap for the exact S1 virtual route; it does not establish native
acceptance, an acquired waveform, stable output, or sub-10-ms latency.
No native audio, VM, driver installation, deployment, or push was performed.

## Explicit route and compatibility

The actual `e2e_latency_probe live-vad` command now accepts a distinct owned
microphone export instead of physical `--output`:

```text
live-vad --stable-endpoint-id PLAY_ID --name PLAY_NAME
  --vad-microphone-id MIC_ID --vad-microphone-name MIC_NAME
  --source-schedule SOURCE_JSON --vad-collection COLLECTION_JSON
  --artifact-dir NEW_DIR --seconds 20
  --validation-mode correctness --jitter-tolerance-ms 3000
```

This is command syntax, not a target run. Both IDs must be exact distinct
32-hex Engine identities. Names follow CREATE's UTF-16/control-character
validation and are not selectors. The route requires an owned changing source,
bounded collection and Internal evidence boundary. Physical output/render-only
requests, imported delay/calibration, Process mode and diagnostic soak refuse.
Even an explicit empty `--output` or a default-valued render-only flag refuses;
duplicates cannot erase physical intent.

Collection schema 1 still requires an explicit recorder ID. Schema 2 permits
omission only for the owned microphone route, for example:

```json
{
  "schema_version": 2,
  "ring_frames": 1024,
  "max_pcm_bytes": 1048576,
  "max_metadata_bytes": 100000,
  "max_packets": 1000,
  "preroll_ms": 100,
  "postroll_ms": 200
}
```

These are the bounded dry-run regression's recorder/storage limits, not
recommended production latency settings. If supplied, `input_endpoint_id` must
exactly equal the discovered microphone MMDevice ID. PCM/metadata/packet limits
and the source/roll/Start/stop budget remain validated before artifacts.

Physical selection remains the default. Existing compatible buffering remains
unchanged when input budget flags are omitted. Explicit capacity/jitter/packet/
gap flags now require `--input-target-frames` on every route: previously those
orphan requests silently disappeared in Compatible mode.

## Real graph, identity and lifecycle

- Extracted the previous inline Windows native module into `live_native.rs`;
  its mechanical extraction compiled before behavior changes. `vad_route.rs`
  builds the actual stereo Source -> Bus -> Sink with default unity sends,
  no Gain, hidden physical output or separate PCM bypass. The production
  ProfessionalBackend/EngineHost supplies its existing VirtualRealtime owner.
  Both virtual boundaries are Followers; the physical branch retains Master.
- One selected DeviceInterface supplies both the provider control session and
  exact root for resolution. Both exports are created before microphone
  discovery/recorder preparation. Resolution requires active capture direction,
  exact raw hardware/stable suffix/root and a unique result; friendly names and
  defaults never authorize routing. Only owned identity strings escape COM.
- Initial MMDevice publication may lag ACX readiness. An absolute 20-second
  publication budget is anchored before export preparation. Only typed
  zero-match absence retries, with capped waits; ambiguity, identity or IO
  errors abort. Deadline checks before/after synchronous queries reject late
  success, but do not preempt a blocked native call. The selected recorder ID
  and full identity are rechecked immediately after native preparation; that
  later check never waits for or adopts replacement identity.
- The external source requests Exclusive 48k stereo; the existing recorder
  requests Shared Float32 48k and retains its negotiated rate/channels. These
  are deliberately distinct clients, not matched period/cadence evidence.
- Both gates use one confirmed Engine revision and one shared admission ticket,
  each with its own exact prepared stamp/shape. Actual post-open snapshots are
  retained/validated. Current ABI Open preserves GateEpoch; Close/reset advances
  it. No synthetic epoch increment is inferred from an open request.
- Recorder Start precedes source activation/preroll. Initial confirmation polls
  only exact-stamp Paused observations until Running within the original
  activation-relative `min(5s + progress grace, total ceiling)` deadline. There
  is no rebaseline, reopened changed generation or new startup interval. A
  stream reacquisition, gate/reset/shape/owner change is a hard refusal, even
  if its gate remains open or correctness mode permits quality faults.
- Running/final health requires both virtual streams and their own boundary
  probes, matching owner/revision and both provider identities/telemetry. It
  neither fabricates a physical master nor waives the physical branch's master
  progress checks. Missing evidence, resets, nonfinite or malformed state stay
  hard errors; correctness separately retains timing/underrun diagnostics.
  A separate current microphone `stream_running` observation prevents a legal
  same-stamp Pause from passing either the newer startup health QUERY or later
  health checks; successful historical native Start is not current Running.
- Partial CREATE ownership, second CREATE/gate failure, and unwinding CREATE
  panic retain all introduced/uncertain exports, never preexisting siblings.
  Cleanup requests source and recorder stop before joins, closes both gates,
  joins the graph, then finishes/releases the recorder native client before
  owned PDO removal. Final observation/close/retire attempt each sibling despite
  an error or unwinding panic. Separate provider/retirement receipts survive.

No production audio-thread, driver, app default, queue tuning or public ABI
change is included in this example-only slice.

## Evidence meaning

Manifest/report retain both requested identities, explicit Internal route,
clock intent and client-mode requests. Actual native collection additionally
retains resolved/rechecked microphone identity, prepared recorder ID, both
provider observations/cleanup results and final joined-owner counters. Requested
intent in dry-run is not an observed route. Physical render queue metadata is
omitted in the microphone manifest and null/not-applicable in collection.

All results stay unqualified. The duplicated stereo waveform is not an
independent pre-route reference. Source submission and nominal frame arithmetic
cannot determine whole-route presentation delay, and no relative-zero-channel
result is a sub-10-ms measurement. Unknown sample age/provenance remains unknown.
The recorder's generic library `Physical` source label does not turn its exact
virtual microphone into a physical measurement endpoint.

## Verification

Logs are retained under ignored `target/s1-s3-20261028/`.

| Check | Fresh result / log |
| --- | --- |
| Parser behavior before implementation | Genuine missing-physical-output refusal: `vad-route-parser-behavior-red.log` |
| Portable route helper RED -> GREEN | 5 passed / 6 runtime failures -> 11 passed: `vad-software-route-helper-isolated-{red,green}.log` |
| Parser review regressions RED -> GREEN | 38 passed / 2 failed -> 40 passed: `vad-route-parser-review-red.log`, `vad-route-parser-review-final-green.log` |
| Lifecycle RED -> GREEN | 10 passed / 4 runtime failures, separate CREATE-panic failure -> 22 passed: `vad-native-lifecycle-{red,green}.log`, `vad-native-create-panic-red.log` |
| Initial microphone publication RED -> GREEN | 1 passed / 5 failed -> 24 scoped resolver tests passed: `s1-vad-microphone-wait-{red,green}.log` |
| Same-stamp native Pause RED -> GREEN | 22 passed / 2 failed -> 24 native helper tests passed: `vad-native-same-stamp-pause-{red,green}.log`; ongoing microphone health tested under both policies |
| Complete Windows example suite | 423 passed: `vad-route-full-frozen.log`; same frozen source hashes retained in `vad-route-frozen-source-hashes.txt` |
| Audio library/integration/doc tests | 831 passed / 3 ignored, 31 result summaries: `vad-route-audio-regression.log` |
| Strict audio all-target Clippy | Passed: `vad-route-clippy-frozen-all-targets.log` |
| Strict actual example test-profile Clippy | Passed: `vad-route-clippy-frozen-tests.log` |
| Workspace fmt / diff checks | Passed: `vad-route-fmt-frozen.log`, `vad-route-diff-accepted.log`; repeated immediately before commit |
| Linux normal/test-profile cross-check | Passed: `vad-route-linux-accepted-{normal,tests}.log`; existing cfg-specific warnings: 1 normal / 3 combined test-profile |

Transient integration compiler failures are retained but are not behavioral RED
or GREEN. The first parent full run caught the intentionally failing CREATE
panic regression (419 passed / 1 failed); a 420-test GREEN preceded the final
Pause review repair. The later complete 423-test GREEN is the acceptance result.
Linux cross-checks compile only; no Linux/native run,
Miri, downstream app suite or remote CI rerun is claimed for this example-only
change. Audio regression counts describe their own execution, not new native
coverage or 831 independent RT allocation tests.

Independent identity/parser review closed explicit empty/erased output and
orphan input-budget findings with genuine failing-then-passing regressions.
Independent lifecycle review identified unbounded initial stamp adoption and a
same-stamp Pause hole; final exact-stamp/current-Running code and regressions
resolve both. Final read-only re-review reports no remaining concrete finding
or blocker within the lifecycle/collection/resolver scope. Reviewer inspected
the 423-pass and strict Clippy logs but did not execute native or other tests.
The identity review note is `vad-software-route-identity-review.md`; the final
lifecycle verdict was returned in the task's review handoff. Earlier source
audits and intermediate 420-test results are not final native qualification.

## Remaining gates and delivery

S1 still needs common explicit physical-render mode/period plumbing, the
supported physical-input route, actual native route acceptance/acquired PCM and
the calibrated common-reference physical experiment. S3 observed-window density,
freshness/replay composition and source-time provenance remain separate work.
S2's bounded budgets do not select or qualify a lower production preset.

VM timing qualification stays deferred. Correctness progress grace remains
0..3000 ms; it is neither allowable audio latency nor an enlarged production
buffer. Stable sub-10-ms output remains unqualified. No driver is installed on
the development host and no VM scratch is added by this slice.

Source commit `0d7dd81` and the accompanying documentation commit stay local for
a later verified batch push. Unrelated `gate.log` is excluded and untouched.
