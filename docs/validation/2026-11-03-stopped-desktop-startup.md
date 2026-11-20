# Stopped desktop fixture and owned-input refusal

Date: 2026-11-03 (Asia/Shanghai). Branch: `feat/minimal-io-latency`.
Baseline: `fd3bbe4`. No production audio, driver, default or timing change.

## Root cause and bounded repair

The [preceding UI-only attempt](2026-11-02-joined-receipt-native-acceptance.md#stopped-ui-comparison-refused-before-measurement)
unexpectedly opened default audio because its fresh preferences disabled project
restoration. In `service/mod.rs::start_service`, no restored project selects
`with_automatic_startup`; catalog completion then configures/starts defaults.
Zero gain does not avoid native device open. This is an evidence-runner error,
not a newly discovered production startup regression.

`scripts/desktop-stopped-startup.ps1` supplies a filesystem-only, fixed fixture:
v2 English preferences, restoration enabled, exact absolute owned project path,
null session, empty candidate graph and no endpoint exports. It uses CreateNew,
bounded64KiB reads, fixed fields, explicit readback/hashes and ancestor reparse
refusal. It never sets the caller's APPDATA, starts a process, enumerates devices
or calls the service. Its `valid=true` describes the documents only;
`runtime_state_verified=false` and `qualified=false` remain explicit. It is not
a general production-project validator or protection against hostile concurrent
filesystem replacement.

A genuine missing-helper assertion RED precedes implementation. Nine PS5 cases
pass, then independent review requests a junction-ancestor regression. That
regression fails before repair and passes afterward, verifying refusal before
either document is created outside the owned path and refusal of redirected
config validation. A separate PS7 execution exposes its JSON integer type
difference; retaining integral Int32/Int64 rather than only Int32 fixes that
compatibility issue. Final ten cases pass separately on PS5.1 and PS7. The
earlier failed PS7 result remains retained. No remote package is silently updated
after acquisition: the final compatible repository helper differs from the
PS5-only helper frozen in the acquired package.

The Rust project regression loads the same JSON shape through actual
`StartupDocuments`, verifies a restored project, Idle/undesired context, empty
candidate topology/exports and no active/pending revision. Its false-restore
case verifies the absence of a restored project. This is a portable startup
contract test, not an execution of Windows `start_service`, native polling or
device closure. Existing automatic-startup behavior is separately covered by
`automatic_startup_uses_catalog_defaults_once_and_stop_stays_authoritative`.

## Verification

An isolated `git archive fd3bbe4` plus exactly three reviewed overlays excludes
all27preexisting dirty/cache files. Provenance and hashes are in:

```text
D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-stopped-startup-20261103-0020
```

Fresh checks on that source:

- Focused stopped-fixture Rust regression:1passed.
- `cargo test --locked -p moiren-app --lib project::`:36passed.
- Existing automatic-startup regression:1passed.
- Full default app library:556passed/2ignored/0failed.
- `cargo clippy --locked -p moiren-app --lib --tests -- -D warnings`:passed.
- `cargo fmt --all -- --check`:passed.
- Stopped-startup helper:10cases each on PS5.1/PS7, filesystem-only.

An initial Rust check fails because a `rustfmt --emit stdout` file header was
mistakenly included when writing its output. The header is removed, ordinary
in-place rustfmt is used, and the corrected exact source passes the listed
checks. The failed compiler output remains archived; it is not regression RED
or a product defect. This test/runner slice does not claim a new full-workspace,
gpui-feature or native audio-quality pass.

## Actual remote control

Remote root: `D:\MoirenTest\20261103-stopped-startup-0020`.
Task: `Moiren-StoppedUi-20261103-0020`. Lifecycle: one bounded task, exact held
app identity, worker150s/task240s, Limited interactive session1. The unchanged
34,150,912-byte app is copied within the target; SHA-256 remains:

```text
67025240CDDA7CFC6C277928FE20897806D4BB8D498D21B7E99F7003071AE974
```

Its source provenance remains the earlier clean8d080f5archive plus the three
reviewed joined-cleanup diagnostic overlays, subsequently committed in ceaf61d;
it is not relabelled as an unmodified current-HEAD build.

The evidence-only alternative input helper uses physical UIA button geometry,
per-monitor-v2 caller DPI and exact visible root HWND/PID/title checks. It rejects
obscured/different-child points, changed window/client geometry, held modifiers
or existing capture. A single-use plan queues at most one DOWN/UP pair; posting
is not consumption or UI effect. It does not use global cursor movement,
foreground activation or SendInput. Receiver-side mouse capture remains a
possible consequence, so input is not globally side-effect-free. Source review
and six helper checks cover compilation/pure coordinate packing/static guards;
none executes user32 on the development host.

The task launches once at00:31:12 +08:00. The restored empty project hashes match
before and after app launch. Native UIA reports Start present/disabled and no
Stop; the final drained log contains no audio-host/native-stream opening or
audio-owner start. This is consistent with the intended stopped startup on this
run, not a measured runtime phase distribution.

The first Settings plan then refuses:

```text
Point is obscured or belongs to a different/child HWND
```

No plan is posted and `actions=[]`. The evidence does not identify the covering
window/child or prove a coordinate defect. No user window is moved, hidden or
closed to bypass the guard. Settings/Close/normal Quit success and usefulness of
this input transport for the400/900ms audio window remain unestablished. The
worker performs bounded forced cleanup of only its held app, exit-1; worker/
supervisor/task finish1. No repeat, changed deadline, relaxed hit-test guard,
larger audio buffer, Start/Stop acquisition or unchanged soak follows.

Thirteen compact files are individually length/SHA-256 verified after a9,891-byte
download. Final00:34:55audit confirms task Ready/result1, no app/source/analysis
owners, unchanged protected OBS/QQMusic identities, defaults, render volume/mute
and running driver/SYS hash. The driver is never installed on the development
host. Main-workspace concurrent edits remain untouched; only exact low-latency
slice paths are staged locally, with no push or memory update.

## Remaining boundary

The stopped startup guard closes an actual runner precondition defect, but the
unobscured native input control still refuses. No UIA two-second root cause or
successful closed-prefix Stop/join/Evaluate/Apply receipt is acquired. Lower
native quality, matched route/clock evidence and calibrated physical/sustained
qualification remain open. First-packet work stays deferred. These honest
boundaries do not turn repeated UI automation experiments into a prerequisite
for independent product work, and they do not complete the original S1-S3 goal.
