# Current Release Probes And Target Preconditions

Date: 2026-11-05, Asia/Shanghai. This prepares current-source artifacts for
later remote acceptance. It does not run native audio, desktop UI or drivers.

## Source And Release Pins

Both ordinary optimized example builds complete successfully from clean commit
`73e1fc2253cb66ecac68f1748ac40899f5e5aa6f`, tree
`d63c786a32e0527f8f40eac8b344fa72b94b32e5`:

- `cargo build --locked --release -p moiren-app --features vad-integration-probe --example vad_integration`
- `cargo build --locked --release -p moiren-windows-audio --example e2e_latency_probe`

Artifact directory:
`D:\coding\moiren-evidence\cargo-target-minimal-latency\release\examples`.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `vad_integration.exe` | 7806976 | `D882CE481FE980E3E01664492E724D9094DD47D118C3F178AC0F686D2543CEE5` |
| `e2e_latency_probe.exe` | 7729152 | `3B94B87A5689C3A3A75A31F5FCCDB58A2EA77FD4E3401AA6190D9B80D6157DC8` |

Build logs, exact commands, terminal exit-0 receipts and binary pins are retained
under `D:\coding\moiren-evidence\20261105-post-pcm-checkpoint`.
The VAD integration release binary is not executed. The E2E release binary runs
only through the existing real PowerShell 7 offline qualification regression,
using synthetic reports and local hash fixtures. That regression exits 0 and
checks p99 misses, calibration-uncertainty boundaries, retained failures and
the deliberately synthetic passing fixture. It opens no device or client and
does not turn that fixture into hardware evidence. Neither binary is deployed.

These are current-source release examples, not a rebuild or deployment of the
ordinary desktop application, a signed driver package or a native test pass.

## Fresh Read-Only Target Check

At host 03:36:14, the existing strict-known-host `moiren-lan` SSH connection
returns machine `LAPTOP-NP49RG6H`, the expected SID
`S-1-5-21-3080296863-1242338503-1346362208-1001`, session 0 and
`admin: true`. It returns the exact literal planned package path
`D:\MoirenTest\20261105-terminal-recorder-audit-0125` and confirms it absent.
The target's wall timestamp is about 22 seconds ahead of the host; no clock
setting is changed and neither clock is used as latency evidence here.

An initial read-only wrapper incorrectly encoded that package path using a
Python string escape. Its account/token fields remain actual observations,
but its package-existence field is not relied on. The corrected raw-literal
wrapper returns and asserts the full exact path, host and SID. Both receipts
are preserved along with `preflight-correction-note.json`; no audit execution
is repeated because neither invocation ran the audit or deployed anything.

The metadata reader requires an actually non-admin token. Its reviewed guard
therefore remains unsatisfied; do not bypass it with an admin process, task,
run-as helper or raw MKV transfer. No ffprobe or remote package is launched,
no files are created remotely, and no process/device/default/driver state is
changed. This account check alone is not a full endpoint/foreign-owner safety
attestation for a future native test.

The read-only VMware inventory at 03:34 reports zero running VMs. The existing
encrypted driver VM still needs user unlock/start; no password retry or host
driver installation is attempted.

## CI And Remaining Qualification

The existing `919d10f` CI `41896388334` remains live at this preparation
checkpoint. Its same retained watcher is reused; no rerun, manual dispatch or
superseding push is issued. Later local PCM receipt repair `0523359` and this
release preparation do not borrow an earlier CI result.

Full S0-S5 completion remains unproven: matched-route and full-waveform
correspondence, native period/block sustained quality, ordinary desktop
retained Stop/Evaluate/Apply, Windows name/default-role/held-client/race/
repetition/sibling cases, sustained load/fault runs and calibrated physical
p95/p99 must still pass. A smaller default, VAD/Process winner, sub-10-ms
result or device-limited impossibility is not established by these builds.
PR26 stays Draft/open and must be split before merge.
