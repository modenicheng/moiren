# Desktop measured Process budget selection

2026-10-29 (Asia/Shanghai). Host-only S3 application continuation. No native
audio execution, VM/deployment, driver build/install or buffer-default change.
This makes the existing conservative measured replacement operation available
in the actual desktop; it does not add automatic tuning or hardware admission.

## Atomic receipt

`68e6b549cff02e5d9e46027b6b3fb57ab612f430` implements the desktop selection,
transaction state, translations and regressions. It was pushed immediately;
the full local SHA matched `git ls-remote origin
refs/heads/feat/minimal-io-latency`. Unrelated untracked `gate.log` is untouched.
Plan: `docs/superpowers/plans/2026-10-29-desktop-measured-process.md`.

## User-visible behavior

Settings / Input Buffers now contains a measured Process subsection above the
existing declared expert estimates:

- Select a running Process graph source and independently choose ring
  capacity, normal target and jitter reserve. These values are a session-local
  draft, not new saved production defaults or a serialized trusted measurement.
- Apply explicitly requests owner replacement using the existing
  `ApplyMeasuredProcessBridge` service operation, once. It does not issue an
  extra `ApplyGraph`, resize a live ring or silently fall back to Compatible.
- Preflight requires matching running graph/settings and source/session/native
  observation identities. The service still owns authoritative revalidation
  and both precision factories. Missing/stale/unsafe evidence gives a visible
  refusal without stopping the currently running owner.
- Unsaved global/device settings or another transaction block measured Apply.
  The separate declared packet/gap drafts are preserved. While a measured
  transaction is pending, repeated Apply is disabled even after consumption.
- Requested and active summaries remain distinct. No active configuration is
  explicitly unknown, not Compatible. Mailbox consumption is not activation.
  Success needs the captured request, new selection ID and new application
  session, matching active graph/settings and running native observation.
  Backend-local numeric native generations may repeat after a new app session.
- Refusal, stop, changed settings/graph and unrelated confirmation retain the
  editable draft and show failure. Unchanged old graph errors do not reject a
  fresh accepted selection. A success message clears after stop/replacement.

The 30-second UI deadline wakes without another snapshot/render event. It is
presentation-only expiry, keyed by RequestId, not Stop or cancellation. The
warning explicitly says audio may still change later; an old deadline cannot
expire a newer request. This is separate from correctness jitter tolerance.

## Fresh verification

All commands use the shared host Cargo target and the scoped worktree. Logs
are retained in `target/s1-s3-20261028/s3-desktop-measured-*`.

| Command | Final result |
| --- | --- |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 476 passed, 3 ignored |
| `cargo test --locked -p moiren-app --test cli --test transport_settings --test ui_acceptance --features gpui-test` | CLI 2 and settings 6 passed; initial UI failure corrected below |
| `cargo test --locked -p moiren-app --test ui_acceptance --features gpui-test` | Final 13 passed |
| `cargo test --locked -p moiren-app --lib --features gpui-test measured_ui_tests` | Final 6 passed |
| `cargo test --locked -p moiren-windows-audio --lib` | 543 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 298 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 142 passed; not 142 separate allocation tests |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets` | Exit 0, warning-free; not Linux execution |
| App all-target check and strict all-target Clippy with `gpui-test` | Exit 0 |
| Root fmt and diff whitespace checks, including staged diff | Exit 0 |

Twelve new app tests comprise five state/preflight regressions, six real GPUI
click tests and one explicit-locale translation regression. Click tests use a
held software service fixture: they verify exact dispatch, duplicate guards,
requested/active projection, refusal and deadline wake-up, not actual native
replacement. Existing managed-service tests in the full app run separately
exercise orchestration and both precision factories.

## Failed attempts and review

- Missing-module/API compilation establishes the initial test-first RED, not a
  behavioral failure claim. State regressions then expose stale-error rejection
  and success surviving Stop; corrected behavior passes.
- Independent review finds two Important issues: missing active settings were
  displayed as Compatible, and a deadline relying on observation/render could
  wait indefinitely. Two actual GPUI regressions fail first, then pass with
  explicit missing-config presentation and a background wake-up. The callback
  notifies even when current observation completes the request first.
- The original four click tests pass when first compiled; no behavioral RED
  is claimed for them. Final independent read-only review finds no remaining
  Important/Critical issue within this UI/state scope; reviewer does not run
  an independent test suite.
- A glob GPUI import initially recursively expands the test macro; explicit
  imports fix compilation. Stateful Div erasure, private test construction and
  visibility/unused-import warnings are corrected before final strict checks.
- The Chinese translation regression fails first. YAML-only changes initially
  leave the embedded locale stale in the incremental build. Rebuilding after
  touching its source consumer makes the regression pass. No build-system
  locale-dependency repair is claimed.
- The added subsection moves legacy expert controls below the viewport. The
  existing acceptance test initially fails on an invisible Process policy.
  A first scroll correction still clicks a partially clipped VAD button; the
  original draft assertion fails. Requiring the whole button inside the scroll
  viewport corrects the interaction. All original assertions remain, including
  retained unsaved drafts and no service mutation before Save. Final 13 pass;
  production behavior is not changed to weaken this test.

## Remaining gates

This action uses existing delivery and whole-native-wake measured budgets,
not offline JSON or finite-window replay approval. Integrated desktop pinned
prefix collection, lifecycle-finalized replay assessment and evaluated
candidate application remain separate software work. Native history archives
can still refuse missed polls, capped/incomplete prefixes or ambiguous order.

Sample-age provenance, future-safe sustained admission, actual lower native
periods and matched physical stable sub-10-ms output remain open/deferred. No
remote CI, Miri, real-device listening or VM timing/stability pass is claimed.

The user's validation policy is unchanged: correctness records quality/timing
faults and continues with configurable 0..3000 ms extra progress grace,
default 3000; latency-sensitive mode fails fast. Identity/schema/resource,
native IO, storage and cleanup faults stay hard. Three seconds is not an audio
latency target and no production buffer is enlarged to conceal VM scheduling.
Full S1-S3 remains active; unavailable hardware does not block this UI work.
