# Closed Process prefix measured-history coverage

Date: 2026-11-02, Asia/Shanghai. Base source: `fbacd74`. This is a
control-side admission correction, not an observed audio corruption fix,
new native acceptance or physical latency qualification.

## Reproduced authorization gap

The live native window checks that whole-drain and whole-wake observations
cover its entire packet/read archive. The closed `final_measurement()` path
previously validated its measurement histories individually but omitted that
same cross-history coverage check. A valid bounded suffix is not proof for
an entire retained prefix, even after the native writers have joined.

Real, headless native-observation fixtures demonstrate four incorrect accepts:

- A consumer ring contains 257 whole wakes; its first 192-frame wake is
  overwritten by 256 later 96-frame wakes. The producer retains all 129
  two-packet drains, while the archive retains all 258 packets and 516 graph
  reads. Before the fix, final measurement returns a 96-frame observed demand.
- A producer ring overwrites its first complete-drain event while the consumer
  history and the complete archived packet/read prefix remain available.
- A final archived queue read has no corresponding completed whole-wake event.
- A final archived packet has no corresponding completed drain/delivery event.

The consumer-overwrite and two missing-tail assertions fail together against
unchanged production code; the producer-overwrite assertion fails separately. A complete-history
positive control already passes. These are actual assertion failures, not
compilation errors, edited DTOs or hardware tests.

This does not establish a smaller unsafe candidate on the frozen native route.
Existing Shared preparation retains a 704-frame demand floor there; the
synthetic example uses a 96-frame prepared floor. Cumulative producer maxima
and ordinary suffix-based measurements retain their existing meanings. No
native underrun, audible defect or physical delay follows from these fixtures.

## Narrow correction

After closed trace validation and the existing ingress-policy refusal,
`final_measurement()` snapshots `WindowEvidence` and invokes the same
`require_coverage()` used by live windows. Both measured histories must retain
sequence one without overwrite; delivery packet counts, drain frame totals and
whole-wake frame totals must match the full archived packet/read prefix.
The admitted measurement and publication-gap envelope use that same artifact.

Only closed full-prefix authorization changes. Generic measured suffix APIs,
RT publication/storage, raw archive export, diagnostic replay, transport
defaults, native periods and audio processing are unchanged. A refused
measurement leaves the complete `artifact()` and `ordered_trace()` available.
No collector/ring limit or initial-output policy is enlarged.

Five regressions cover the four refusals and positive control. An existing
registry-replacement fixture now performs real four-frame graph segments
summing to its declared 64/128-frame whole wakes; it retains the original
late-demand and owner-identity assertions. Its first adjustment exceeded the
fixture's original per-read bound and failed; that receipt is retained. The
corrected fixture changes no replay gate.

## Fresh checks

Source is a separate `git archive fbacd74` export with exactly the two intended
source/test overlays. The 27 original dirty/cache files are excluded and
preserved. The development host runs compilation and headless tests only.

| Surface | Result |
| --- | --- |
| Closed/live collector tests | 24 passed, 0 failed |
| Windows audio library, default | 796 passed, 4 ignored, 0 failed |
| Windows audio library, `vad-probe` | 796 passed, 4 ignored, 0 failed |
| Scoped strict Clippy and owned-file rustfmt | Exit 0 |
| Fixed application release build | Exit 0 |
| Feature-enabled workspace tests | 2342 passed, 9 ignored, 0 failed |
| Feature-enabled workspace all-target check / strict Clippy | Exit 0 / 0 |
| Workspace formatting | Exit 0 |

Before this correction, the independent clean baseline also freshly passes
5 zero-refill, 51 marker/qualification and 20 measured-UI tests. Those baseline
results are not represented as fixed-build native acceptance.

Full feature-enabled workspace tests, all-target check/strict Clippy and
workspace formatting run against the same frozen two-file overlay. Their
integration receipts retain separate commands, source hashes and log hashes.
These counts are separate invocations, not unique behaviors; ignored tests
are not represented as passed. Remote CI, Linux and Miri are not rerun.

Evidence roots:

- `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-closed-coverage-20261102-2215`
- `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-window-success-20261102-2210`

The first retains RED/failed-fixture/GREEN logs, exact source overlays, build
receipts and the fixed application. The second retains baseline build/tests,
bounded UIA runner contracts and remote preflight. No failed receipt is
overwritten or reclassified as a native success.

## Remote acceptance boundary

The proposed single desktop test freezes Engine48k/F32/block64, Shared336,
render512/wake64/callback64, draft capacity16384/target2048/jitter96 and policy
4096/4096/480/2/100000. Source starts before the short capture, stays alive
through normal Stop/join, Evaluate and conditional explicit Apply, and is
stopped after normal application Quit. A genuine native candidate and exact
requested/active new-owner confirmation are required; refusal is retained
without an automatic retry or parameter adjustment.

The current GUI does not expose both final measured history arrays. Its
candidate/Apply result must therefore remain distinct from independently
audited whole-history completeness; the runner explicitly reports that
coverage as unavailable, rather than inventing native data.

At 22:18, remote preparation creates only the isolated empty directory on
`LAPTOP-NP49RG6H`. At 22:21:52, read-only preflight reports 3095264 KiB free
physical memory, below the unchanged 8388608 KiB launch requirement. No native
test is started, no application is killed to free memory, and the guard is
not relaxed. Protected OBS/QQMusic identities, driver SYS and boot remain
unchanged; no owned native/analysis process exists.

At 22:25:53, the reviewed fixed application and five pinned runner components
are deployed and remotely hash/length verified; all four PowerShell scripts
parse on the target. Free memory is then 1656272 KiB, still below the same
launch requirement. The task is not registered and no run directory exists.
The ready application is
`D:\MoirenTest\20261102-window-success-2210\bin\moiren-app.exe`, SHA-256
`419CD7F9254C1AE3C45936AEB107215C841C1E7023A83EE0667B2609CE891363`,
34174976 bytes. Its source is the exact base plus reviewed two-file overlays,
not an unmodified commit build. No driver installation is performed.

This resource refusal does not block software development. It adds no
unchanged stability soak, lower-default promotion, path-winner claim or
physical/sub-10-ms result. First-packet investigation remains deferred.
The original S1-S3 goal stays active.
