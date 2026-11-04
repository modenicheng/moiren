# S4 PCM Completion Uses The Existing Readmission Contract

Date: 2026-11-05, Asia/Shanghai. Source base: `fb87ec8`. This corrects the
existing streaming harness; it adds no repetition mode or transport policy.

## Source-Supported Mismatch

The PCM loop already accepts a temporary gate closure only when the same
native identity has an observed newer stream generation and `GateProgress`
admits it within the existing five-second/phase bound. However, after reaching
the strict 48,000-frame PCM window, its second snapshot read immediately
requires open gates and fails rather than waiting for a legal re-admission.
The ordinary PCM coordinator already distinguishes readiness from owner
validity. A closure between the pre-pump and completion snapshots exposes the
inconsistent streaming-harness behavior.

That second snapshot also bypasses the per-gate state machine and final
publication-age check. A snapshot just inside the two-second bound before
pumping can be stale after pumping yet still satisfy the former success path.
These are deterministic source-contract defects, not observed native failures.

## Correction And Unchanged Gates

The actual coordinator now uses one portable `PcmPhase::end_ready` method for
both observations. It retains the actual runtime observation, validates the
same owner/project/revision and both endpoints' `GateProgress`, and returns
pending rather than success or failure for an otherwise legal temporary
closure. Success still needs both open gates, two successful Starts and the
original strict signal window. Native identity, stamp regression, explicit
errors, unexplained closure and deadline violations remain failures.

Both calls pass the age of the actual Arc publication tracked by the
coordinator. Only a changed Arc refreshes that timestamp. The method enforces
the existing two-second age bound; the final caller also checks the actual
wall deadline after publishing evidence. No five-second re-admission, twelve-
second PCM, settling, tolerance, direct Removed or overall watchdog changes.

The pre-edit `same_admission` wrapper remains Windows-only. Its shared owner
predicate moves unchanged into the portable module and is exercised by actual
phase readiness. New tests live in `stream/tests/readmission.rs`, keeping
lifecycle/cleanup and PCM-readiness test responsibilities separate.

## Failure-First Evidence

Evidence directory:
`D:\coding\moiren-evidence\20261105-s4-pcm-readmission`.

- The final-check logic is first mechanically extracted and connected to the
  actual coordinator without its behavior fix. Three added tests produce
  130 passes and three assertion failures. The initial wrong test import is a
  separately retained compilation error, not the assertion RED.
- The primary trajectory uses real `observe_pcm`, real `GateProgress` and
  production gate-epoch advancement: a newer generation is observed open
  before pumping, the first completion observation is closed, and only a
  later in-deadline reopening completes. Target and sibling gates are tested.
- Illegal same-stream closure, unchanged original re-admission deadline after
  further generations, missing PCM, identity/revision/binding/observation and
  explicit-error mutations remain rejected.
- A separate actual-helper freshness regression accepts exactly 2,000 ms and
  rejects 2,001 ms. Before the age guard it has one assertion failure; the
  existing limit is then enforced rather than enlarged.

| Final split-source check | Result |
| --- | --- |
| Windows opt-in example tests | 135 passed, zero failed |
| Linux stable Rust 1.99.1 opt-in example tests | 114 passed, zero failed |
| Windows app all-target strict Clippy | Exit 0 |
| Linux stable strict example Clippy | Exit 0 |
| Windows workspace all-target check with probe feature | Exit 0 |
| Windows non-test example build | Exit 0 |
| Workspace fmt and whitespace checks | Exit 0 |

The first Linux extraction build reports a native-only `same_admission`
wrapper unused. Keeping that wrapper in the Windows module resolves the cfg
boundary without suppression; original failure and subsequent checks remain
archived. The final four-file overlay is pinned in `fix-split-manifest.json`.
Independent review finds no remaining actionable issue in that source.

## Qualification Limits

No audio client, UI, OBS, device or driver operation is run. The original dirty
feature drafts remain separate. No native closure trajectory, Windows-visible
name, waveform continuity, repetition/race/default-role matrix, sustained run
or calibrated physical latency result follows. Existing quality failures and
the complete S0-S5 requirements remain open; new source needs its own CI.
