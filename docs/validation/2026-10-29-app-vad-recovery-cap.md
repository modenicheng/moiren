# Application And Probe VAD Recovery Cap

Date: 2026-10-29, Asia/Shanghai. Worktree: `minimal-io-latency`.
Parent: `3266fdf13f365173a837708c4d044f06150ddd49`.

## Delivered Software

The S2 native recovery-cap API existed, but both actual desktop transport
conversion and the VAD probe adapter always supplied the omitted/default cap.
They now forward independent explicit intent:

- `TransportLatencySettings::vad_recovery_cap_periods: Option<u64>` persists
  independently of Compatible/Expert input buffering. Omission remains omitted
  on serialization and preserves the legacy eight-period ceiling. Portable
  validation refuses values outside 1..=8; no clamping or fallback is added.
- Both production Windows F32/F64 factory paths already consume
  `virtual_options()`, which now forwards the cap unchanged. Source inspection
  confirms this wiring; injected factories exercise the service lifecycle.
  No production device was opened by these tests.
- Settings > Input buffers > VAD offers Compatible (8) and explicit 1..8.
  Independent draft rebase preserves concurrent Process/VAD buffer edits and
  refuses same-cap conflicts. Requested and active summaries remain distinct.
- Save changes desired intent, not the active owner. Stopped Save stays Idle.
  Explicit Restart Audio waits for confirmed active settings before persisting
  the replacement; rejected activation retains the draft and old preferences.
- Probe `live-vad --vad-recovery-cap-periods 1..8` rejects duplicates, malformed
  values and Process routes. Backend preparation, collection reporting and
  VAD manifest use the same conversion. The manifest records requested options,
  including default/explicit distinction; it is not native admission evidence.
  Both root and `live-vad --help` explain the option and limitations.

The cap limits virtual-clock catch-up, not normal input residency, packet
size or physical wake demand. Smaller caps may discard more overdue periods.
They are not a latency guarantee. A 128-frame target with a 32-frame Engine
block is model-admitted at cap1 in focused fixtures, while cap8, a physical
256-frame wake, or a larger native packet is still refused independently.
This is preparation-model evidence, not lower native period qualification.

No production buffer default, clock recovery implementation, whole-wake
admission, audio-thread operation or test-validation policy changes.

## Atomic Code Commits

Each was pushed immediately and its full local SHA compared with
`git ls-remote origin refs/heads/feat/minimal-io-latency`:

- `9d337f9a16a34078cdeae0601ff0a33b0bef604e`: persisted cap, admission, independent
  merge, service/preference regressions and scoped plan.
- `35d94f9dc20437c2719a77cc866c504b5290b690`: actual settings choices, summaries,
  translations, stopped Save, held confirmation and rejection regressions.
- `00d77b270a8701ca3a41d77617b392e9c29c610b`: explicit probe option, shared adapter,
  requested manifest evidence, parsing/budget/dry-run tests and both help paths.

## Verification

Host-side commands use
`CARGO_TARGET_DIR=D:\coding\moiren\target`. Logs are retained under this
worktree's `target/s1-s3-20261028/`, not staged into source control.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --locked -p moiren-app --features gpui-test --lib` | 511 passed, 3 ignored | `app-vad-cap-final-app.log` |
| `cargo test --locked -p moiren-app --features gpui-test --test transport_settings --test ui_acceptance` | 8 preferences + 13 UI acceptance passed | `app-vad-cap-final-integrations.log` |
| `cargo test --locked -p moiren-windows-audio --lib` | 551 passed, 2 ignored | `app-vad-cap-audio.log` |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe` | 301 passed | `probe-vad-cap-final.log` |
| `cargo clippy --locked -p moiren-app --features gpui-test --all-targets -- -D warnings` | passed | `app-vad-cap-final-clippy.log` |
| `cargo clippy --locked -p moiren-windows-audio --all-targets -- -D warnings` | passed | `probe-vad-cap-final-clippy.log` |
| `cargo fmt --all -- --check` | passed | direct command output |
| scoped `rustfmt --edition 2024 --check` on the three edited probe modules | passed | direct command output |
| `git diff --check` | passed | direct command output |
| `cargo run --locked -p moiren-windows-audio --example e2e_latency_probe -- live-vad --help` | option and safety caveats present; no native IO | `probe-vad-cap-help.log` |

RED-to-GREEN evidence:

- Preference tests first fail on unknown `vad_recovery_cap_periods`; after the
  field and range validation they round-trip all eight caps and reject invalid
  ranges/types. Existing old-preference exact JSON assertion still passes.
- Actual click test first fails because `vad-recovery-cap-1` does not exist;
  implemented UI passes the click, Save-without-restart, held confirmation and
  new active snapshot checks. Independent merge tests were added after the
  merge implementation and are not claimed as behavioral RED evidence.
- Chinese translation first returns English fallback; the catalog fixes it.
- Probe tests first fail on unknown flag and missing requested manifest object;
  final tests admit explicit options and retain them without native claims.
- One dry-manifest assertion was corrected to the existing tagged input-policy
  schema (`{"policy":"compatible"}`), not an untagged string. The original
  missing-manifest failure remains valid RED evidence.

Two independent read-only reviewers find no Important/Critical issue in the
service/model/UI/probe scopes. The probe reviewer identifies the separate short
help omission; it is corrected and the actual help command verifies the text.
Reviewers run no cargo or native operations.

## Remaining S1-S3 Work And Boundaries

This closes the bounded application/probe S2 integration gap, not full S1-S3.
Bounded drift-aware offline marker analysis remains a useful independent S1
software task. Source-age provenance, smaller actual native period negotiation,
sustained future-safe operation and matched stable physical sub-10-ms output
are still open; none is inferred from software budgets or dry runs.

VM music/timing-dependent qualification stays deferred at the user's direction.
Correctness continues timing/quality diagnostics with 0..3000 ms extra progress
grace; latency-sensitive mode remains fail-fast. That grace is not audio latency
and does not enlarge production buffers. Identity/schema/resource/native IO/
storage/cleanup faults remain hard failures. No VM access, driver install,
deployment, live audio, remote CI or Miri is performed in this slice.

Unrelated `gate.log` is preserved and excluded from every commit.
