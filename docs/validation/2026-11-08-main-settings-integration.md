# Main integration: settings and low-latency controls

Date: 2026-11-08, Asia/Shanghai. Integration commit `b9d219b` merges
`c8ab5e4` from main into the existing minimal-latency branch based on
`da77880`. It does not merge the feature into main or establish native
acceptance. The original dirty `minimal-io-latency` worktree is not edited.

## Preserved behavior

- Main's full-workspace settings, centered cards, value menus, overlay
  scrollbars, immediate interface preferences, per-node/strip faults and
  graceful terminal quit remain present.
- The feature's Input buffers section retains exact native Shared periods,
  exact expert input intent, measured Process admission, render budgets,
  freshness checks and requested/active transaction confirmation.
- The diagnostic cards retain Process/native timing accessibility nodes,
  provider/Windows endpoint state, requested/active budgets and explicit
  physical-latency-unqualified status. No nominal queue is relabeled latency.
- Audio save no longer captures or rewrites immediate interface preferences.
  A regression clicks a density change, then saves the pre-existing audio
  draft and verifies the new density remains persisted while audio stays Idle.

## Regression evidence and repairs

The first merged workspace run reports868passed,7failed,3ignored in the app
library. Focused investigation reproduces the viewport extending below the
real window: the scrollbar wrapper was not a column flex container. Giving
it a constrained column layout restores ordinary scroll access without
changing test budgets or removing visibility assertions. The focused window
suite then passes245tests with1ignored.

Additional tests first fail against long recovery-cap option clipping, stale
device-conflict/pending cache state and an old period-input callback changing
a newly replaced editor. Repairs wrap the long choice groups, key only the
device state actually rendered by each settings section, retain nested input
notifications and refuse callbacks from replaced input entities. Ordinary
text keys reach the focused platform input handler; Escape still peels the
value menu/page and the workspace remains unmounted behind settings.

New test fixtures initialize their own audio settings and do not assume the
developer's persisted period is blank. Pending-state cache tests require a
repaint rather than exactly one render, since focus/disabled-state changes
can produce an additional nested notification.

## Verification

Evidence root:
`D:\coding\moiren-evidence\20261108-094031-pipeline-next`.

- `cargo test --locked --workspace --features moiren-app/gpui-test,moiren-app/vad-integration-probe`:
  2,651passed,0failed,10ignored across73result groups; exit0.
- `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test,moiren-app/vad-integration-probe -- -D warnings`:
  exit0.
- `cargo fmt --all -- --check` and staged whitespace checks: exit0.
- OBS reuse26, fixed-section moments11 and removal-stage reader29offline
  regressions: all pass. These66tests do not launch OBS or native audio.
- `main-merge-verification-counts.json` pins the workspace log SHA256
  `90F5188C49987F72D91E68EFA78EC2FDFCA81BE80B49D11F077F1C26D462C382`
  and Clippy log SHA256
  `6A8FA0D5B0B9E5254D39A6C518D0197940E1495142FE86C6CEDDCDE05D989AAF`.
- At09:57, all29original draft/cache file hashes remain unchanged. Generated
  untracked integration caches are excluded from the commit.

At09:54 the read-only direct-remote query still reports an October30 boot,
not a post-install reboot. No automatic reboot/native retry follows. Package613
still awaits the user's reboot and a fresh driver/provenance check. S1-S5
native waveform, lifecycle, naming, desktop and sustained/calibrated gates
remain open. The original goal is not complete.
