# Retained read-only target readiness

Date: 2026-11-08, Asia/Shanghai. This advances the actual removal diagnostic
entry for S1; it does not repair or qualify removal, waveform, naming or
physical latency. The user has not confirmed a reboot. No remote collection,
driver/device action, audio/OBS/ETW acquisition or reboot occurs in this slice.

## Actual entry and retained composition

`scripts/vad-removal/invoke-preflight.ps1` now uses the production
`target-readiness.ps1` composition. Confirmation and exact target host/SID
are required before imports, discovery, device collection or evidence creation.
This read-only entry is separate from the still-incomplete full diagnostic.
It cannot approve native scheduler, ETW, decoder, task deletion or audio work.

The owner retains both original components:

- `preflight-session.ps1` starts one `collector-worker.ps1` runspace with the
  migrated real `readonly-preflight.ps1` collector. Its controller PID,
  creation and actual session are retained; session0 is legal and is never
  rewritten as an interactive worker session.
- `session-identity.ps1` enumerates Explorer candidates, retains each original
  process before handle/image/token reads, and selects the target SID's actual
  nonzero Limited session. Multiple observed sessions require an explicit
  matching selection; missing shell, access denial or unknown identity refuses.
  Revalidation reads the same selected processes, never reopens a PID or
  depends on unrelated unselected processes remaining alive.

The user-session discovery uses the existing process-token reader, without
WTSQueryUserToken or a SeTcb assumption. It neither checks desktop unlock nor
proves future worker audio access. A logged-on shell is a discovery prerequisite
of this runner, not a claim that all audio requires Explorer. Actual Limited
worker/probe binding remains the final authority.

Readiness validates the actual collector snapshot: explicit-zone fresh times,
post-install boot corroborated by typed uptime, frozen613 SYS/INF hashes,
signature/catalog membership, healthy PnP binding, empty native owners/pending
tasks/raw children, complete explicit external owners, all six unique default
roles, render volume/mute and security observations. An empty external-owner
list is legal when explicitly observed complete. File/PnP provenance still is
not an in-memory driver-image attestation or evidence that removal is fixed.

The collector keeps original inner finite-process handles through failure and
timeout, drains both original pipes and flushes, and closes only after observed
natural completion. Late completion stays failed. A failure does not authorize
kill, restart, rediscovery, replacement owner or native follow-up. Entry failure
continues observing the original runspace and children.

The create-new flushed/readback-hashed `preflight-receipt.json` pins the accepted
collector snapshot. `target-readiness.json` links that exact original receipt
with the separately observed worker identity and real controller session.
Assertions revalidate both original owners, current evidence and immutable pin
JSON; changed source, nested owners, receipt content or replacement pins refuse.
The original worker identity excludes refreshed observation time, so ordinary
revalidation does not invent a new process identity. All readiness results keep
`NativeApproved=false`, `Qualified=false` and `AudioAccessProven=false`.

## Verification and scope

The existing Windows CI matrix now includes24 preparation suites. Parent
PowerShell5.1 and7 each pass all289checks across those24suites; all48 original
suite invocations exit0. Execution logs and exact source/log hashes are retained in:

`D:\coding\moiren-evidence\20261108-112100-collector-worker`.

`readiness-final-matrix-receipts.json` records the complete48-invocation result
with current source/log hashes. A final review adds an explicit target-host
refusal to the actual collector test and exercises the entry child under the
current interpreter. Both changed suites are rerun under both interpreters;
unchanged passing suite logs remain original rather than being relaunched.
The existing offline removal reader separately passes29 tests.
Workflow YAML, PowerShell parsers, Rust formatting and diff checks pass.

`removal-target-readiness-final-source-and-logs.zip` retains the script dependencies,
workflow/README and original logs. All107members independently verify length
and SHA256 after closing/reopening the archive. The231601-byte archive SHA256 is
`FC5A0973CFE896FD846D780AF2683045AEF04EF011FEC88B6F3C8B26F185641D`;
`removal-target-readiness-final-archive-verified.json` records the full result.

The eight added suites cover41 migrated collector checks,13 selected-session
checks,9 preflight checks,9 actual readiness composition checks and4 actual
entry refusals. Devices, task scheduling and process/token discovery are
mocked. Collector composition uses real runspaces and harmless finite children;
read-tool integration uses actual redirected success/nonzero child exits.
Actual production entry tests exercise no-confirmation/wrong-host refusal
under each interpreter from an isolated entry without sibling helpers, so
an early import would change the observed refusal reason. No target
token/device access is exercised. Host uptime uses actual GetTickCount64, and
host refusal reads the current Windows identity; the suites are not a claim
that every Windows API is mocked or that no native API executes. Audio, device
mutation, scheduler and ETW operations are absent.

Failing-then-passing evidence includes the absent preflight/session/composed
APIs, source-repin refusal, unselected-session independence and receipt-pin
replacement. Independent static review identifies the receipt-repin defect;
the parent reproduces it as9checks/1failure before fixing original pin capture
and path/JSON comparison. Both component and composed suites retain the
regression. Review clears the final production hashes:

```text
preflight-session.ps1
A20AF55C5F433CDBC5B0AA4D54D354410AC4961A302DCC5CFE3038939F2CCF46
session-identity.ps1
35DE3840918CAC369AB75724B7161D5AE17B79CC423CF4D2FCB3CDAD3B149CE3
target-readiness.ps1
7D2D3C00651D2A159E5C5E76B6FC800931B9909E00316EEB400C2B7A91F9400B
invoke-preflight.ps1
3B881C3FFEE22AD57EF9A9A3380C7E225F5DF3163D1A744924BD003B818FDC93
```

The real collector and runspace definitions are byte-identical to their frozen
evidence originals; migrated test changes are paths and the explicit
development-host safeguard above. Alternate
host collectors are explicitly `InjectedCollectorSource`. A repository source
hash taken at construction pins the current run, not a reviewed release or
permission to execute native work. This is correlation within a writable
same-user environment, not protection against a hostile process.

## Remaining full diagnostic and original acceptance

The combined native diagnostic still needs reviewed release/source pins,
exclusive target ownership, the privileged trace controller, exact Limited
scheduler/worker/probe dispatch, actual finish/decode, independently compared
safety-after observations and verified archive transfer/cleanup. Scheduler,
trace, default decode and task deletion guards remain unconditionally closed.
Readiness does not open them. Synchronous COM/CIM/hash calls are not hard
cancellation; no durable controller-crash recovery is claimed.

Installed613, its matching symbols and rollback package are unchanged. The
locally pinned managed probe is not relabelled, deployed or run. Use the separate
remote target directly after user-owned reboot confirmation and fresh checks;
no VM or blanket unlocked-desktop condition is introduced. Reuse the fixed
dedicated OBS installation without executable copies or firewall changes.
Previously policy-rejected host fixture deletion is not retried or bypassed.
All original S0-S5 native and physical acceptance gates remain in force.
