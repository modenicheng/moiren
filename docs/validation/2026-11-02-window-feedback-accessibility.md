# Process window feedback accessibility

Date: 2026-11-02, Asia/Shanghai. Baseline: clean `7508720` plus only the
three application source/test paths below. This software-only repair runs in
parallel with the separate [ordinary measured desktop acceptance](2026-11-02-current-measured-desktop.md).
That native run uses the earlier binary and does not certify this repair.

## Cause and minimal repair

The existing live-window feedback and closed-prefix admission, activation and
result rows have stable IDs and visible ARIA labels, but omit a native role and
text value. This is the same metadata omission demonstrated natively for the
separate [requested/active diagnostic rows](2026-11-02-process-diagnostics-accessibility.md).
A headless observer can read the old labels without proving a native node
exists. No native reproduction of these four particular rows is claimed.

`settings/measured/live.rs` now gives `measured-live-window-feedback` a Label
role and matching accessible text value. `settings/measured/window.rs` does
the same for `process-window-admission`, `process-window-activation` and
`process-window-feedback`. These are eight added production lines, following
the existing ordinary measured feedback pattern. IDs, displayed content,
layout, color, conditional rendering and button behavior stay unchanged.
No new helper, dependency, fixture, audio path, RT operation, policy, admission
gate, budget, default or physical qualification rule is introduced.

The existing real headless `measured_ui_tests.rs` row reader now requires
Some(Label) and value=label for these four IDs. Assertions run on the existing
rendered widgets rather than a fabricated accessibility node. Existing tests
continue to exercise policy opt-in, missing/stale evidence, evaluation without
audio requests, refusal, draft retention and exact asynchronous confirmation.
Private software candidate injection stays explicitly fixture-only; production
CallerSupplied evidence remains refused.

All four IDs have actual row-reader calls, but this is not a four-ID-by-all-
states matrix: closed-prefix activation metadata is checked on success, not
directly in pending/refused states. Closed-prefix stale-job discard also has
state-model coverage rather than a rendered metadata assertion. The same
unconditional production metadata decorators cover those states; no additional
native or full-matrix claim follows.

## Verified regression

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-window-feedback-a11y-20261102-1435`.
Clean source archive is pinned to `7508720a60fa5111163f67168a4fab717bc63ce4`;
its SHA256 is
`30A35A65AECA741B732FF06A9420C099BF59B58EDAF25F71713819EF04F8C88C`.
`source-verification.json` hashes all1336 tracked archive files and finds only
the three authorized source/test paths different. Worktree/export copies of
those three files match byte-for-byte. The eighteen original dirty files plus
nine files in the two existing Python cache directories retain all27 hashes.

Before production edits, two focused runs fail with exit101 for the intended
assertion, not a compilation or fixture error:

- `red.log`: `desktop_live_window_requires_opt_in_and_each_policy_field_before_dispatch`,
  `measured-live-window-feedback`: None versus Some(Label).
- `closed-red.log`: `retained_apply_settings_click_requires_separate_admission_then_confirms_new_owner`,
  `process-window-admission`: None versus Some(Label).

After the minimal repair, `cargo test --locked -p moiren-app --features
gpui-test --lib measured_ui_tests -j1` exits0:20 passed,0 failed,0 ignored.
Targeted formatting exits0. An earlier accidentally wrong-directory test
invocation is interrupted and retained separately in
`interrupted-wrong-cwd.json`/`.log`; it is not used as RED or GREEN evidence.

The parent independently runs `cargo test --locked --workspace --features
moiren-app/gpui-test -j1` from that verified clean export: exit0,
2307 passed,9 ignored,0 failed across71 surfaces, including13 UI acceptance
cases. The20 focused tests overlap these totals. This is Windows software
coverage, not Linux/CI/Miri or opt-in native device acceptance.
`cargo clippy --locked --workspace --all-targets --features
moiren-app/gpui-test -j1 -- -D warnings` and `cargo fmt --all -- --check`
also exit0. Exact commands, exits and durations are retained in
`workspace-checks.json`; counts are in `workspace-counts.json`. Independent
static review finds no concrete behavior regression or vacuous assertion.

## Boundary

This closes the scoped metadata omission in software; a new Windows UIA
acceptance for these four live/closed-prefix rows is still separate. The
ordinary measured desktop run does not exercise either window mode. Missing
native waveform correspondence, unavailable calibrated physical measurements
and historical refused traces do not become passes, and need not block
independent product development. First-packet work stays deferred. No new
audio soak or local host audio/device manipulation is required for this patch.
