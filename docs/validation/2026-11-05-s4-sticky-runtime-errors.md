# S4 Explicit Runtime Errors Remain Failures

Date: 2026-11-05, Asia/Shanghai. Source base: `361d177`. This is an
acceptance-harness correction, not a native audio or driver repair.

## Source-Supported Defects

The streaming coordinator records actual service snapshots while waiting for
the rename transition and direct old Removed evidence. Their service, graph
and provider error fields were not retained as failures. A later successful
Apply or QUERY could clear those fields, allowing the same Case to complete.
The first/last raw snapshots alone did not make an intermediate error sticky.

The shared `Evidence::exited` predicate also accepted closed gates and Removing
resources while an explicit service or graph cleanup error was present. A
successful service join could consequently allow the report to say passed.
The existing application regression
`explicit_exit_does_not_wait_for_failed_native_retirement_to_report_absence`
demonstrates that a shutdown REMOVE timeout can coexist with drained ownership
and retained Removing resources. Join is not proof of error-free retirement.

## Correction

- The actual transition and retirement observation paths retain each explicit
  service/graph/provider error before checking positive lifecycle proof.
- Repeated identical stage/source/text observations add only one failure;
  later clean snapshots cannot remove it. First/last raw evidence remains.
- Exit acceptance rejects explicit service or graph errors. Error-free
  Removing remains allowed and still does not claim final PDO disappearance.
- Old-client native invalidation, strict Windows name, direct Removed identity,
  PCM thresholds, Start/Stop accounting and all deadlines remain unchanged.

This restores the documented fail-closed contract; it adds no repetition mode,
new transport policy, driver operation or weaker qualification criterion.

## Failure-First Verification

Evidence directory:
`D:\coding\moiren-evidence\20261105-s4-error-latching`.

Three added regressions exercise the actual observation/exit methods. The
transition and retirement cases each cover service, graph and provider errors,
duplicate polling and later clean observations. Before the implementation
change, the Windows example executes 127 passing tests and three assertion
failures. After the correction it executes 130 passing tests, zero failures.

| Check | Observed result |
| --- | --- |
| Windows opt-in `vad_integration` example | 130 passed, zero failed |
| Linux Rust 1.99.1 opt-in example | 109 passed, zero failed |
| Windows app all-target strict Clippy with probe feature | Exit 0 |
| Linux Rust 1.99.1 strict example Clippy | Exit 0 |
| Windows workspace all-target check with probe feature | Exit 0 |
| Non-test Windows example build | Exit 0; binary pin in build receipt |
| Existing application shutdown-timeout regression | One passed, zero failed |
| Workspace fmt and diff whitespace checks | Exit 0 |

The Linux run uses a complete `git archive 361d177` plus the three exact files
listed in `fix-manifest.json`; it does not absorb original dirty-worktree
drafts. An independent read-only review finds no actionable issue in the fix.

## Earlier Stable-Source Verification

The separate `20261105-s4-stable-recheck` directory verifies unmodified
`361d177`: Linux Rust 1.99.1 example tests pass 106, and corrected strict
example Clippy exits 0 at 02:21:01. The first shell-generated Clippy command
accidentally passes a trailing CR in `warnings`; despite exit 0 it emits an
unknown-lint warning and is **not** strict-lint evidence. Its original log and
argv remain retained. A new LF-only invocation supplies exact `-D warnings`
and completes without warnings; no product source or lint gate is relaxed.

## Limits

These commands start no native clients, audio/UI application, OBS process,
driver installation or remote acquisition. No native/name/waveform, sustained
or physical acceptance follows. CI for `303e481` belongs to that older source;
this correction requires its own published-head CI. Repeated rename, races,
independent sibling/default-role checks and the remaining S0-S5 gates stay open.
