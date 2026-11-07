# S1-S3 software integration checkpoint

Date: 2026-11-02, Asia/Shanghai. Tested source: `db1be9f` on
`feat/minimal-io-latency`. This is a software integration checkpoint, not an
overall S1-S3 qualification PASS or permission to promote lower defaults.

## Fresh integrated checks

An independent `git archive` of the exact commit excludes the uncommitted
changes in all eighteen preexisting dirty paths. Tracked files remain present
at their committed versions; untracked files are absent. Tests run in that
source export and reuse only the worktree Cargo target directory. No native
audio client, GUI application, device, driver installation or recorder is
opened on the development host.

| Command / surface | Fresh result |
| --- | --- |
| `cargo fmt --all -- --check` | Exit 0 |
| `cargo check --locked --workspace --all-targets` | Exit 0 |
| `cargo test --locked --workspace` | 2178 passed, 8 ignored, 0 failed |
| `cargo test --locked -p moiren-app --features gpui-test --lib` | 657 passed, 3 ignored, 0 failed |
| `cargo test --locked -p moiren-app --features gpui-test --test ui_acceptance` | 13 passed, 0 failed |
| `cargo clippy --locked --workspace --all-targets --features moiren-app/gpui-test -- -D warnings` | Exit 0 |
| E2E latency probe / Shared render preflight example tests | 678 / 5 passed; 1 probe fixture ignored |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | 239 passed, 0 failed |
| All twelve `scripts/tests/*.tests.py` suites, using `python -B` | 226 passed, 0 failed |

The Python checks cover OBS analysis, retained Process PCM/replay/coverage,
VAD timing association, indexed spacing and SRC archives/phase/cascade. They
analyze synthetic or retained data; passing does not certify a new native run.
Workspace, feature-enabled application, examples and Python counts are separate
test invocations, not a count of unique behaviors. Ignored hardware/fixture
tests are not silently run or represented as passed. Linux, remote CI and Miri
are not rerun by this Windows checkpoint.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-s3-integration-20261102-110312`.
`intent.json` pins the source archive/hash, commands and preserved-path hashes;
`results.json`, `extra-results.json` and `python-results.json` retain command
exit codes and individual logs. No push or memory update occurs.

All eighteen original file hashes still match after verification. Independent
review confirms the reported counts and qualification boundary; its wording
finding is corrected by distinguishing excluded changes from committed paths.

## Native evidence stays separate

The latest [Process CLI acquisition](2026-11-02-process-cli-packet-capture.md)
has normal source/monitor/task exit and complete stored packet histories, but
only 18/19 graph codes satisfy unchanged waveform gates. The preceding
[VAD acquisition](2026-11-02-matched-graph-packet-capture.md) resolves 17/19 and
its desktop harness fails during shutdown observation/cleanup. These refusals
are retained; this software checkpoint neither reruns nor upgrades them.

At 11:06:44, a read-only remote audit confirms the authorized target has no
Moiren/probe/Python owner, the three retained tasks are Ready with their original
results, the pinned application hash is unchanged, and external OBS/QQMusic
process creation identities match. No remote audio or new long-running task is
launched. Protected processes are not stopped or reconfigured.

## Development boundary

The [bounded software scope review](2026-11-02-desktop-process-capture-scope.md)
finds no additional necessary original S1-S3 software integration gap. Keep
the implemented independent budgets, exact identity/admission, prepared-owner
replacement and requested/active diagnostics. Unsupported source-age and
policy-composition capabilities remain explicit refusals, not new baseline work.

Remaining qualification is not calibration alone: waveform correspondence,
native lower-setting acceptance, historical delivery-gap and VAD epoch errors,
and a calibrated physical reference remain open. Existing acquisition failures
do not establish a new production defect or a source-path latency winner.
Keep Compatible defaults and quality gates unchanged. The withdrawn retired-PCM
fail-fast experiment stays withdrawn; first-packet investigation stays deferred.

Do not block independent application development on unavailable simultaneous
physical recording equipment. Future long tests should answer a specific new
question, run finitely on the authorized remote target in parallel with
development, and retain exact configuration, owner identity and cleanup results.
An unchanged soak or another nominal marker fit is not the default next step.
S4 naming and S5 lower-default qualification remain separately scoped work.
The already recorded S4 naming milestone is the next independent lifecycle
work: first reconcile existing stopped/streaming/held-client/race receipts,
then run only missing scoped cases or fix a reproduced defect. It does not
require physical latency recording equipment or an unchanged stability soak.
