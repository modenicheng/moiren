# Desktop Closed Process Window Diagnostics - 2026-10-29

Implementation: `6c332672c86f13e8981d917387204d038872a264`.
Plan: `f00095b5710d71c0c134fd92661bb5173b81f144`.
Both are pushed to `feat/minimal-io-latency` and independently compared with
the full `git ls-remote` branch SHA. Unrelated untracked `gate.log` is preserved.

## Actual desktop workflow

Settings > Input buffers > Evaluate closed Process prefix opens a secondary
diagnostic page. Capacity, target and jitter remain the three independent
measured-input draft values. The page shows the exact retained predecessor
app/document/graph/native/Process/node identity and five explicit thresholds:
initial output-unserved allowance, initial input-prime-discard allowance,
minimum output frames, output reads and conservative original-QPC output span.
These are diagnostic policy thresholds, not production defaults or VM grace.

Evaluation uses the actual immutable service-retained closed trace, on one
bounded background worker. It sends no audio request, restarts nothing, never
copies an old profile and never authorizes Apply. Changing the candidate or
policy invalidates displayed results and pending work without prematurely
releasing the worker slot. Completion, successful reports and candidate errors
are fenced on exact retained Arc, candidate and policy. Equal-looking replacement
evidence is still a different predecessor. An old completion cannot finish a
new worker. Ordinary rendering never replays or clones full archives.

Results distinguish candidate preparation refusal, observed violation,
insufficient exposure and no violation in this window. They display actual
provenance, declared timing assumptions, complete prepared wake/rate, exposure,
violation causes and insufficient thresholds. Every result remains unqualified.
English and Simplified Chinese catalog entries follow the actual shipped
language set, not the earlier summary's incorrect seven-language description.

## Diagnostic assumptions, not admission

The candidate evaluator derives a declared publication bound from maximum
**offered** packet frames, including dropped tails, and the outer consecutive
packet envelope `current.after - previous.before`. It rounds the maximum gap
up to native 48000-Hz frames with checked clock subtraction and integer
conversion. A singleton packet prefix explicitly refuses missing gap evidence;
it does not invent a period. Capacity is bounded before candidate allocation.

The original consumer rate and complete prepared native wake demand remain
unchanged, including a regression at 96000 Hz / 1024 frames. Segmented reads
cannot shrink that preparation requirement. Delivery is marked `Declared`,
not `Empirical`; native trace origin and declared synthetic assumptions remain
independent. Test fixtures use actual software bridge histories and remain
`CallerSupplied`. Production uses the already-private native-retained trace;
neither editable snapshots nor imported artifacts gain native provenance here.

Packet publication envelopes do not recover native callback grouping, future
gaps, startup/terminal silence bounds, sample age or waveform correctness.
Successful diagnostic replay is deliberately **not** a measured profile.
Existing live measured Apply remains independent: current native whole-wake
admission, owner replacement and exact activation confirmation are unchanged.
The integrated production evaluated-apply workflow remains open rather than
promoting predecessor diagnostic evidence into current admission.

## Fresh verification

All commands run on the development host using the shared Cargo target directory;
none runs native audio, VM, deployment or driver operations.

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 495 passed, 3 ignored |
| `cargo test --locked -p moiren-app --lib process_window --features gpui-test` after mutation restoration | 11 passed |
| `cargo test --locked -p moiren-app --test cli --test transport_settings --test ui_acceptance --features gpui-test` | 2 + 6 + 13 passed |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit 0 |
| `cargo fmt --all -- --check`, `git diff --check` | Exit 0 |

The 11 focused checks cover actual bridge replay/options/provenance, offered
rather than accepted supply, original rate/wake, envelope rounding, bounded
capacity, singleton refusal, distinct output outcomes, policy/predecessor/
candidate/job fences, error fencing, two actual Settings click paths and Chinese
translation. The stopped-session click fixture has no live native observations;
the missing-prefix path cannot dispatch audio or evaluate a fabricated trace.
Full application checks also retain the existing live measured Apply tests.

Two independent read-only reviews find no remaining Important/Critical finding
after the failure-feedback fence fix. Reviewers inspect sources, not rerun cargo.

## Failed and corrected attempts

- Initial service/state tests fail on missing evaluator/modules: compile RED,
  not a claimed behavioral failure.
- The first UI test uses an incorrect `Option<bool>` disabled assertion; fixed
  before the meaningful missing-open-button RED. The widget does not expose a
  disabled snapshot at that lookup, so the final test clicks the unavailable
  action and checks visible refusal plus zero audio requests instead.
- Initial UI implementation reports `Not qualified`; a case-sensitive test
  expects lowercase and is corrected. A new visibility warning is fixed with
  narrower field visibility; final strict Clippy is clean.
- The offered-supply fixture initially sets target equal to capacity and fails
  fixture preparation. Its explicit target is corrected to 128 < 256. This is
  not attributed to candidate evaluation.
- Chinese action tests fail on untranslated fallback before catalog entries.
  Final language verification passes.
- Review detects failure text surviving predecessor replacement while successful
  reports are correctly fenced. A scoped failed-result job fixes it. A deliberate
  mutation removes only that error fence: the exact stale-error assertion fails
  normally. `finally` restores original production bytes; all 11 focused tests
  then pass. No mutant remains. Full-library results are from the same restored
  source bytes; the mutation changes no intended implementation.

Logs: `target/s1-s3-20261028/s3-desktop-window-*`. Final `app-lib`,
`integrations`, `clippy` and `restored-green` logs support the counts above;
earlier failed attempts are retained and are not counted as successful evidence.
No new remote CI, Miri, audio/probe/RT harness suite, native screenshot or physical
measurement run is claimed for this app-only slice.

## Remaining gates

S1-S3 remains active: production evaluated Apply, source-age provenance,
future-safe sustained admission, actual lower native periods and matched
physical stable sub-10-ms qualification remain open/deferred. VM timing/music
stability is not retried. Correctness mode continues timing/quality diagnostics
with configurable 0..3000 ms extra progress grace; latency-sensitive mode stays
fail-fast. Hard identity/schema/resource/native IO/storage/cleanup failures
remain hard. No production buffer/default is enlarged to hide VM scheduling.
