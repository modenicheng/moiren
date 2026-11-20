# Remote build cache retention policy

Date: 2026-10-29. This is the current policy for the separate physical build
target, following the user's correction to avoid repeated full rebuilds.
It supersedes earlier instructions to clean after each completed build,
deployment or evidence archive. Historical cleanup receipts are unchanged.

## Default: keep reusable build outputs

- Reuse the single `D:\MoirenBuild\target` across compatible task worktrees.
  Preserve `D:\MoirenBuild\cargo-home` and task-local temporary/cache paths on D:.
  Do not create a new target tree per agent, commit or verification stage.
- Do not run `cargo clean` merely because a build, test, deployment, archive
  or work session has finished. An empty target is not a completion criterion.
- Keep compatible build environment, features and profiles stable where
  practical. Retaining Cargo artifacts avoids unnecessary dependency rebuilds
  even with the existing `CARGO_INCREMENTAL=0`; this policy does not silently
  change rustc incremental mode, debug settings or validation coverage.
- Check disk space before substantial build phases, not by deleting artifacts
  after every command. Compilation must still not overlap audio acquisition.

## Pressure-triggered cleanup only

The existing 60-GiB free-space floor and 50-GiB target-size guard remain
inspection/pause thresholds, not automatic full-clean triggers. If either
guard is reached, inspect owned usage and the upcoming build first.

1. Prefer verified obsolete staging files and redundant outputs over the
   active reusable target. Preserve the current deployment, known-good
   rollback, source, installed driver, and measurement evidence.
2. If Cargo artifacts must be reclaimed, select the smallest justified
   obsolete package/profile or separate stale target. Use a whole-target
   `cargo clean` only for actual storage pressure that selective cleanup
   cannot resolve, demonstrated cache corruption, or an explicit clean-build
   requirement. Do not clean speculatively to prevent a possible build error.
3. Inspect active processes and resolve every deletion path inside the owned
   workspace before deletion. Never clean a running build/acquisition target
   or remove user/system files. Archive and hash-verify evidence before pruning.
4. Report the reason and reclaimed space; then resume ordinary cache reuse.
   Do not attach a routine full clean to deployment or verification scripts.

## Current starting point

The previous cleanup has already left the shared target empty; it cannot be
undone. The latest inspected snapshot has target bytes 0, no active build or
acquisition, and D free 272,456,163,328 bytes (about 254 GiB). A subsequent
first build will need to recreate its artifacts; retain them afterward.
No additional cleanup or rebuild is needed just to apply this policy.

Evidence: `target/physical-1921681297-20261029/` contains the verified
`evidence-recorder-period-diagnostics-5382369.zip` target-state receipt and
`recorder-period-space-inventory.jsonl`. These are capacity observations,
not audio-quality or physical-latency results.
