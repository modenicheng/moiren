# E2E VAD minimum-period request

Date: 2026-10-29. Baseline: `66d3f4a`. Feature source: `1d3e691`.
The S1-S3 objective remains active.
This slice changes the actual E2E probe's external exclusive VAD source client.
No native audio, VM access, driver installation, or deployment was performed.

## Behavior and evidence

`e2e_latency_probe live-vad ... --vad-minimum-period` explicitly queries both
default and minimum device periods and selects the minimum. The switch is
valueless, single-use, and rejected for Process Loopback or other source modes
before artifact/native preparation. It applies to ordinary playback,
diagnostic soak, and supervised VAD collection.

Omitting it preserves the existing default-only native query and default
request, including the accepted 1,000..=1,000,000 hns range and absence of an
added floor. An invalid unused minimum cannot reject the default. With the
switch present, an invalid selected minimum refuses without falling back to the
default; an unused default does not become a new admission condition.

Opt-in manifest/source fields retain requested intent. After a successful
native query, optional `native.vad_playback.period_request` retains:

- `qualification = "unqualified-request-only"` and `minimum_period = true`;
- raw `default_hns` and `minimum_hns`;
- `initial_request_hns` and `final_request_hns`;
- `alignment_retry`.

Request evidence is seeded before the worker performs COM/identity work. Raw
query values survive a selected-value refusal, and the initial request is
published before Initialize. The existing single fresh-client alignment retry
records recovery intent before the fallible aligned-size query, and publishes
its final request before fresh activation/Initialize. If the size query fails
or returns invalid frames, `alignment_retry` remains true while the final
request stays at the last known initial request; no aligned period is invented.
The flag alone does not prove fresh activation or another Initialize occurred.
Existing retry buffer bounds and ceiling conversion remain unchanged; the
initial request range is not applied again to the aligned retry.

Both supervised and ordinary preparation retain a joined partial worker
outcome if preparation fails before Ready. Request evidence does not depend on
successful `PlaybackInfo`, preroll, Start, or a prepared graph. Unknown values
remain null. Native errors and cleanup failures are retained. With the switch
omitted, all added optional fields are absent. Shared Process source playback
continues to use its separate shared preparation path.

The existing `native_buffer_frames` remains an independently observed buffer
size. Provider packet observations and physical output observations are
unchanged. Requested minimum, aligned request, buffer capacity, packet cadence,
and route presentation latency are distinct; no actual-period field or stable
sub-10-ms result is synthesized.

## Verification

Logs are under ignored `target/s1-s3-20261028/`.

| Check | Result | Log |
| --- | --- | --- |
| New CLI tests before implementation | 2 expected failures: unknown switch | `e2e-vad-period-cli-red.log` |
| CLI tests after implementation | 2 passed | `e2e-vad-period-cli-green.log` |
| Portable helper tests before behavior implementation | 8 runtime failures at intentional skeleton bodies | `e2e-vad-period-helper-red.log` |
| Joined failure evidence before forwarding | Expected null versus 100000 failure | `e2e-vad-period-retention-red.log` |
| Alignment intent before a failed size query | Expected missing-intent failure before the fix | `e2e-vad-period-alignment-red.log` |
| Focused period tests, final | 15 passed, including 14 new tests and existing recovery-cap parsing | `e2e-vad-period-focused-final.log` |
| Complete Windows example suite, final | 337 passed | `e2e-vad-period-full-reviewed.log` |
| Normal strict example Clippy, final | Passed | `e2e-vad-period-clippy-reviewed.log` |
| Actual test-profile strict example Clippy, final | Passed | `e2e-vad-period-test-clippy-reviewed.log` |
| Linux normal example cross-check | Passed with 1 cfg-specific dead-code warning | `e2e-vad-period-linux.log` |
| Linux test-profile example cross-check | Passed with 4 cfg-specific dead-code warnings | `e2e-vad-period-linux-test.log` |

The initial test-profile strict Clippy attempt failed on two baseline marker
test lints: constant `chunks_exact_mut` and a manual range predicate. Separate
local commit `b62de1e` applies their equivalent idioms without changing test
assertions. The failed attempt remains in `e2e-vad-period-test-clippy.log`.
Root formatting and `git diff --check` pass. Linux checks are compilation,
not executed Linux tests or warning-free builds.

The first complete independent review identified missing alignment intent if
the aligned-size query failed or returned invalid frames. Its failing-then-
passing regression calls the actual alignment helper, checks that the native
query callback holds no publication lock, and checks joined failure evidence.
The success matrix covers frames 1, 48, 512, and the existing 9,600-frame upper
bound, both with and without minimum intent. Neither test performs COM calls.
Final complete independent re-review found no remaining Important/Critical
findings and retained the original finding as resolved. A separate final
native integration re-review also found no material issue. Review notes are
`e2e-vad-period-final-review.md` and
`e2e-vad-period-native-review.md`. No library/app/RT/Miri or remote CI rerun is
claimed for this example-only slice.

## Remaining gates

Actual device period queries, the requested period being accepted, resulting
buffer/packet shapes, native failure/cleanup behavior, sustained output, and
matched physical IO/Bus route latency require a suitable target. Portable
selection tests and source review cannot provide that evidence. The VM timing
qualification remains deferred by user direction.

Production queues, whole-wake admission, recovery limits, validation modes,
and the 0..3000 ms correctness progress grace remain unchanged. Grace is not
allowable audio latency. Stable output below 10 ms remains unqualified.

Source and documentation commits are kept local for a later batched push.
