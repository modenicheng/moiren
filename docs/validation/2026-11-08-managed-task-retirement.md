# Exact managed task retirement and fresh probe pin

Date: 2026-11-08, Asia/Shanghai. This advances the guarded removal pipeline,
including the user's requirement not to accumulate obsolete test tasks and
executables. It does not operate the scheduler, remote machine or audio driver.

## Retirement composition

`Complete-RemovalManagedTaskCleanup` first binds the session's original
readback to its exact trace owner and original confirmed stopped receipt.
Another genuinely stopped trace cannot replace this run's live trace. Pending
STOP cannot authorize task cleanup; a stopped MORE_DATA result may permit
ownership cleanup without inventing known/lossless final counters.

The function obtains a fresh scheduler observation and recomputes full terminal
worker/probe ownership through the existing retained process checks. A cached
`Session.Terminal`, detached reported exit or successful task state alone does
not authorize cleanup. An independently observed error exit may be terminal
ownership without being native diagnostic success.

`Remove-RemovalOwnedTask` revalidates the exact nonce-named task's definition,
principal, action, original XML, state, last result and absence of instances.
It rechecks the original worker exit. It then writes original XML create-new
in the run directory and independently verifies length/SHA256 before deleting
the exact task once. Only COM HRESULT0x80070002 establishes absence on readback;
access errors, changed tasks and ambiguous delete replies retain the original
owner and never authorize a retry. XML collisions do not overwrite evidence.
No task stop, process kill, boot/security change or directory deletion exists
in this path. The real cleanup approval guard still unconditionally refuses
until the combined target orchestration is reviewed.

Independent review reproduces a missing original readback correlation, then
verifies its repair and whole-session substitution regression. This guards
internal accidental replacement, not hostile same-user code mutation or atomic
protection against another scheduler administrator replacing a task between
the final read and DeleteTask.

## Verification

All14 noninteractive removal suites pass under Windows PowerShell5.1 and
PowerShell7: **177 checks per interpreter**. The task-retirement suite has
nine cases and uses real production definition validation, scheduler-observation
policy, task/worker/probe terminal composition, create-new file IO and hash
readback. Only external scheduler and process observations are fixtures.
Missing-entry RED and foreign-trace-substitution behavioral RED logs are
retained. An initial readback-error fixture incorrectly injected its failure
before deletion; that fixture is corrected before the final green runs.

The separate real harmless worker/token suite4, worker/probe suite6 and actual
Rust-test/PowerShell receipt suite13 pass under both interpreters. They invoke
no native audio CLI or scheduler. The neighboring offline removal-stage reader
also passes29tests. Workflow YAML and diff whitespace checks pass.

Evidence reuses
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.
`retirement-final-test-receipts.json` pins all28 final suite logs. Real child and
cross-language final logs have explicit `NATIVE_EXECUTED=false` results.

## Managed release, not a target run

The exact committed Rust sources at
`26cd706f773f9bfb75e9572fc66594b5f932b918` build successfully with:

```text
cargo build --locked --release -p moiren-windows-audio --example e2e_latency_probe
```

The new7752704-byte EXE SHA256 is
`C8AB9DB102B91CE0AE3724D1856279678019909FB0E9F8F271C77B3453A63266`.
Its PDB is separately pinned in `managed-probe-release-pin.json`. The three
members of `managed-probe-release-26cd706.zip` independently verify length/hash;
the4074584-byte archive SHA256 is
`A53B9BADC1A5413BD538CDFF96BAFA6562A71438C7F86D479573E4A4D5AD5BE0`.
This is one local build archive, not another remote OBS/probe installation.
Neither release EXE nor PDB is deployed or executed. The harmless cross-language
tests use the separate Rust test executable, not this native release EXE.

The historical old probe is not relabelled, and installed diagnostic613 remains
unchanged. Fresh deployment/pins, post-user-reboot provenance and complete
collector/privileged trace/Limited worker orchestration remain required. The
user alone reboots the standalone direct-SSH target. No VM, blanket desktop
unlock or OBS is needed for the planned eight-second removal diagnostic.

The preceding identity head0098c69 completes CI42452098679 with20successful
jobs; its original watcher55609 exits0. The next published ETW head26cd706
starts its own CI42454664245 and does not inherit that result. PR26 stays Draft.

Already policy-rejected host-fixture deletion is not retried through another
mechanism. No complete artifact-cleanup or native S0-S5 pass is claimed.
