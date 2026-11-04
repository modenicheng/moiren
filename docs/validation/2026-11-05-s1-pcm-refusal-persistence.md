# Native PCM Audit Retains Persistence And Decode Refusals

Date: 2026-11-05, Asia/Shanghai. Source base: `919d10f`.
This repairs offline evidence handling with synthetic archives only. No real
PCM, OBS container, remote file, device or waveform alignment is read or changed.

## Reproduced Defects

Independent source review identifies two defects in the actual
`scripts/process-native-pcm.py` audit and CLI path:

1. `audit_owner` publishes `status: structurally_complete` before its final
   memmap flush/close. A failed output flush makes the CLI return 2 but leaves
   that successful structural receipt and no refusal on disk. The old cleanup
   also stops on the first flush error, leaving later mappings open.
2. The JSON decoder rejects literal NaN/Infinity but accepts an overflowing
   numeric token such as `1e400` as infinity. An unvalidated nested origin value
   reaches `write_report`, whose strict serialization fails after creating a
   partial, invalid `summary.json` instead of a valid refusal receipt.

The first RED run preserves a genuine flush/assertion failure plus the related
Windows open-mapping fixture-cleanup error and malformed JSON decode error.
Test-only cleanup then closes any deliberately retained maps, and the invalid
receipt is made an explicit assertion. The clean failure-first run has 32
passing tests and two assertion failures, with no fixture-cleanup error.

## Correction

- The decoder uses a finite float parser for all JSON inputs, including nested
  origin and cursor/packet metadata. Overflow is refused before it can enter
  the report; finite values, duplicate-key checks and bounded reads remain.
- Array finalization detaches the map set, attempts every flush and close,
  aggregates persistence failures and is safe to call again during cleanup.
- The owner finishes the arrays before publishing the structural receipt.
  Persistence failure records `status: refused`, a bounded reason and
  `partial_outputs_untrusted: true`. Existing partial outputs stay available;
  they are not silently repaired or deleted.
- A multi-owner regression injects an early first-owner output flush failure.
  Its valid refused receipt propagates to the root while the second owner is
  still inspected. A late-output regression also verifies every mapping closes.

This does not make structural completion a waveform or physical qualification.
Every tested receipt retains `qualified: false` and unknown physical latency.
The source protocol, accepted/offered mapping, exact recurrence, validity mask,
quality thresholds, FIFO bounds and original residuals are unchanged.

## Verification And Evidence

Evidence root: `D:\coding\moiren-evidence\20261105-s1-pcm-receipt`.
`fix-final-manifest.json` pins the two exact code/test files; the Linux overlay
rehashes them before execution.
The focused code commit is `0523359`. Independent final review finds no
actionable blocker and confirms both final source hashes; it does not claim
to have rerun the parent-executed tests.

| Check | Result |
| --- | --- |
| Windows full native-PCM audit regression | 35 passed, zero failed |
| Debian exact-file overlay regression | 35 passed, zero failed |
| All 17 existing Windows Python regression suites | 321 passed, zero failed |
| Python syntax and Git whitespace checks | Pass |

The all-suite result includes the final 35 tests; it is not an additional 35
tests. Fixtures generate synthetic PCM under bounded temporary directories.
No original source archive is reprocessed, refitted or reclassified.

Linux preparation initially lacks matplotlib. A failed nested-shell venv
command generates owned artifacts in the checkout and overwrites `.gitignore`
before refusing for missing ensurepip. Those exact newly generated paths are
moved into the evidence directory, and `.gitignore` is restored to its exact
current-HEAD Git object hash. This is not an unknown external edit or a product
patch. A literal shell script then prepares an isolated package target in the
Linux cache. Its install records a system-SciPy/target-NumPy compatibility
warning; these audit tests use NumPy/matplotlib, not SciPy, and complete. No
repository dependency, user draft or native configuration is changed.

## Remaining Gates

The published earlier `919d10f` readiness/S5 head has matching CI
`41896388334` still running at this observation. This later Python correction
does not borrow that run's result and must retain its own source identity.
Do not cancel a live CI merely to publish this increment.

Native route/waveform correspondence, S2 sustained operating budgets, ordinary
desktop retained Evaluate/Apply, S4 Windows naming/lifecycle matrix, sustained
load/fault runs and calibrated physical p95/p99 acceptance remain incomplete.
No default promotion, latency winner, sub-10-ms success or device-limited
impossibility is established. PR26 remains Draft/open and split-before-merge.
