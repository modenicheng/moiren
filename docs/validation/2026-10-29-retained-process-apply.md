# Retained Process Evaluation And Explicit Apply - 2026-10-29

Plan: `14c5d230d244b7af9de2bcda71126388e08fea0c`.
Audio: `507c496071018a3f76e75586f826ed9dedf595eb`.
Service: `d6c63fab018ab16840281d0ea6bc0584f7a9a469`.
Desktop: `d408d0de18f81cc56f69dfa8cfa632163837a7a8`.
Each atomic commit was promptly pushed to `feat/minimal-io-latency` and its
complete SHA compared with `git ls-remote`. Unrelated `gate.log` is preserved.

## Actual workflow and trust boundaries

Settings > Input buffers > Evaluate closed Process prefix now performs two
separate operations on its existing single bounded background worker:

1. Declared offered-packet/outer-publication-envelope diagnostic replay.
2. Conservative final measured whole-drain/whole-wake admission, followed by
   replay of those exact admitted options under the selected window policy.

Evaluation still sends no audio request. Only a retained-native trace with
valid final measured evidence and `NoViolationInWindow` can issue the opaque,
non-Deserialize application candidate. Missing measured evidence leaves
diagnostic replay available. Caller-supplied/edited/imported artifacts cannot
issue production candidates; mutable reports cannot modify issued candidates.

The collector retains original observation-only delivery and consumer peers,
not audio resources or unique writers. Final measurement is collected after
host join and existing origin/cleanup checks, without reacquiring a replaceable
registry. Complete prepared and observed whole-wake demand remain authoritative;
segmented queue reads cannot shrink them. The final publication outer gap can
only widen the typed measured reserve, with upward frame rounding. It does not
replace whole native drains with individual packet sizes.

The desktop separately displays conservative empirical drain/gap/wake/rate,
the five actual replay-policy thresholds, refusal, and explicit Apply. Apply
requires stopped Idle, the exact retained predecessor Arc and matching document,
graph, source PID/creation, settings and clean cleanup. Pending work and cleanup
also refuse it. The service rechecks before opening, including deferred attempts.
The token travels inside the measured profile so a superseding ApplyGraph
cannot erase these checks. Both frozen ApplySettings and current settings must
match the evaluated predecessor, including after a mutate-back sequence.

Native preparation re-admits the exact options and refuses a negotiated rate
change or whole-wake growth beyond the evaluated envelope, even if the buffer
target could fit the new demand. Cleanup/completion credits are reserved before
Idle becomes Starting, allowing backpressure retries without stranding a token.
Activation confirmation requires the exact new selection, graph, settings and
new app owner with running native observations. Mailbox consumption alone and
an Idle resource wait do not confirm activation. The existing 30-second UI
deadline remains presentation-only and cannot cancel an in-flight audio request.

Existing live measured Apply remains a separate path. English and Simplified
Chinese cover the new actions and boundaries. Neither workflow claims future
delivery safety, waveform continuity, source sample age or physical latency.

## Fresh software verification

All checks run on the development host, with the shared host Cargo target.
No VM connection, audio-device open, driver build/install or deployment occurs.

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 551 passed, 2 ignored |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 505 passed, 3 ignored |
| `cargo test --locked -p moiren-app --test cli --test transport_settings --test ui_acceptance --features gpui-test` | 2 + 6 + 13 passed |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 298 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` with `CARGO_INCREMENTAL=0` | 176 passed |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit 0 |
| `cargo clippy --locked --manifest-path tools/rt-check/Cargo.toml --all-targets -- -D warnings` | Exit 0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets` | Exit 0, no warnings |
| Root and RT-harness `cargo fmt --all -- --check`; `git diff --check` | Exit 0 |

The RT harness total includes shadow unit tests, not 176 allocation-guarded RT
checks. Actual allocation assertions remain around the existing realtime paths.
It now shadow-compiles actual private bridge and measured-admission modules
instead of widening production visibility for standalone test compilation.
Explicit module paths refer to the same source files as before. Its exhaustive
bridge-error conversion exists only in the harness.

Native-observation tests use software bridge storage in the audio crate's
private owner seam: final identity survives registry replacement and late
delivery/demand; missing whole-wake evidence refuses admission; positive exact
empirical replay preserves retained-owner provenance and `qualified=false`.
The positive fixture has distinct publication-before/after, packet delivery and
drain timestamps and verifies upward rounding from 96 to 97 gap frames.
This proves the provenance boundary mechanically, not actual native IO.

Application tests use explicitly test-only software candidates whose reports
remain CallerSupplied. Production issuance rejects that same origin. Tests
exercise managed stop/reopen in both precisions, exact activation, stale
origin/settings/refusal, credit exhaustion/retry, superseding ApplyGraph,
captured-settings mutate-back, positive candidate invalidation and real Settings
clicks. ObservedViolation and Inconclusive assessments cannot issue a token.
There is deliberately no public native-provenance fixture constructor.

## Failed and corrected attempts

- Initial native/backend tests fail to compile before their new APIs exist.
  These are compile REDs, not claimed behavioral regression evidence.
- A permissive software factory is required to detect a lost candidate fence;
  the initial generic factory refused profiles independently. With the fixture
  corrected, superseding ApplyGraph genuinely opens a second owner before the
  profile retains its token; the final regression refuses that stale open.
- Exhausting either cleanup or completion credits initially leaves Starting
  instead of Idle. Reservation before intent transition fixes the actual
  behavioral RED; releasing credits then permits one new activation.
- The Apply click initially times out because its visible-but-clipped button's
  center is outside the scroll viewport. Bounds assertions identify this, and
  tests now scroll the actual Settings surface before pointer dispatch. A
  fixture with historical hardcoded graph identity subsequently fails real
  confirmation; it now records the current session/graph/settings. A stopped
  held-request fixture also incorrectly reports drained and parks forever;
  it now remains active until confirmation/exit. No production timeout is widened.
- Added admission text also clips the earlier diagnostic rows in full UI runs.
  The shared test helper reveals requested rows by real scrolling, including
  returning to policy/evaluation controls. All nine measured UI tests pass.
- New Chinese actions first fail with untranslated English fallback. Catalog
  entries fix the actual translation assertion; full app tests pass.
- Review finds frozen ApplySettings can differ while current settings mutate
  back to the evaluated origin. The new regression genuinely opens a second
  owner before the frozen-settings check. Final tests refuse precision,
  transport and latency-profile mismatches. The precision fixture is chosen
  opposite its actual default rather than assuming F32.
- Review finds nonzero publication brackets need 97 frames while point-gap
  admission supplies 96. The corrected native fixture fails `UnsupportedShape`
  before monotonic outer-envelope widening; exact empirical replay then passes.
- Standalone RT compilation first mixes dependency/private shadow types.
  The attempted incomplete shadow also hits missing child paths and a rustc
  incremental ICE. Actual private modules, matching local exports, explicit
  child paths and harness-only error mapping correct compilation without
  exposing private APIs. A non-incremental full run passes 176 tests; ordinary
  strict Clippy and Linux checks also pass. No cache deletion is required.

Two independent read-only reviewers close the frozen-settings and publication-
envelope findings and identify no remaining Important/Critical issue in those
fixes. The additional harness wiring has a separate clean read-only review.
Reviewers do not execute cargo or native/VM operations. Earlier failed logs are
retained and never counted as passing evidence. Logs are under
`target/s1-s3-20261028/s3-retained-*`.

## Remaining qualification

The desktop evaluate-and-explicit-apply software gap is closed. Full S1-S3 is
still active: source-age provenance, sustained/future-safe service, actual lower
native period negotiation and stable matched physical sub-10-ms output remain
open/deferred. No remote CI, Miri, live screenshot or physical PASS is claimed.

VM music/timing qualification remains deferred at the user's direction.
Correctness mode continues timing/quality diagnostics with configurable
0..3000 ms extra progress grace; latency-sensitive mode remains fail-fast.
This grace is not allowed audio latency and never enlarges production buffers.
Identity/schema/resource/native IO/storage/cleanup faults remain hard failures.
