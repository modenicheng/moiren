# Reviewed Terminal Metadata Package Staged Without Execution

Date: 2026-11-05, Asia/Shanghai. Code checkpoint: `fc05652`.
This is a two-file package handoff, not a terminal/recorder metadata audit,
native audio acquisition or waveform/physical qualification result.

## Scope And Execution Boundary

The existing reviewed package's README separately authorizes create-once SCP
placement of only `read-terminal-recorder.ps1` and `input-manifest.json`, with
exact remote length/hash verification before execution. Its reader requires
the exact target SID and a genuinely non-admin token. The older combined
deployer additionally requires non-admin before placement because it
immediately invokes the reader; that program remains unchanged and is not
reused.

Independent source review confirms that separate inert placement is within the
existing two-file handoff scope. It does not authorize an admin execution of
the reader, an alternate token/task/RunAs helper, ACL or execution-policy
changes, executable/raw-container transfer or original-input reads.

The same contract's foreground requirement means one synchronous observed
invocation, not an active foreground HWND, manual keyboard operation or
session1. It explicitly permits limited-account SSH session0. The current
lock-screen observation is therefore a desktop-input limitation, not a
separate blocker on this metadata reader. The non-admin requirement remains
unsatisfied by the current SSH execution identity.

## Verified Placement

The original package is:
`D:\coding\moiren-evidence\20261104-pipeline-resume\terminal-recorder-audit`.

All 32 frozen package entries match their lengths and SHA-256 hashes before
and after placement. The fresh stage-only wrapper and generated command
previews are independently reviewed and parse under explicit PS5.1 and PS7.6.6.
Ordinary Python assertions are confirmed enabled. The wrapper uses synchronous
plain PowerShell commands, strict known-host SSH, fixed host/SID, bounded
transport timeouts and create-new local logs. A timeout is indeterminate and
never triggers an automatic retry.

At 04:30:28..04:30:33 host time, one placement:

1. Confirms `LAPTOP-NP49RG6H`, exact SID
   `S-1-5-21-3080296863-1242338503-1346362208-1001`, session0 and `admin=true`.
   The exact package is absent and its parent is an ordinary directory.
2. Creates `D:\MoirenTest\20261105-terminal-recorder-audit-0125` once, without
   Force or overwrite.
3. SCP-transfers only the following two fixed inert payloads.
4. Verifies exactly two regular, non-reparse files, exact lengths and hashes.
5. Stops. It does not invoke the reader or any original-input metadata tool.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `read-terminal-recorder.ps1` | 17535 | `2B239259A4D6399A1468434FA16CD9DC5CA6D914A89A1918329DE82BA96535DD` |
| `input-manifest.json` | 14355 | `C0E5C991D73011054A1C096AD17C04E600801455EC31B6E3C9B42D5AEE74F835` |

All five placement/verification transport calls terminate with exit0 and empty
stderr. Local evidence is
`D:\coding\moiren-evidence\20261105-s4-idle-admission\s1-stage-only`.
`completion.json` explicitly retains `placement_token_admin=true`,
`placement_complete=true`, `reader_invoked=false`, `ffprobe_invoked=false`,
`original_inputs_read=false`, `token_or_acl_changes=false`,
`native_audio_started=false`, `audit_complete=false` and `qualified=false`.

The closed local compact ZIP contains 24 payload files plus its manifest,
12,646 bytes, SHA-256
`123B65DD2FC46F8952E7FD3D59D23D5422EC6CCD88F62522732B309A1A51B162`.
Every archive entry is independently byte-verified against the original file.
It contains placement/provenance/parse records, not raw audio or a metadata
audit receipt.

Successful admin placement does not prove that a later ordinary token can read
the package or original inputs. Neither a metadata receipt nor packet PTS,
terminal-padding explanation, changed residual count or audio-quality pass
is acquired. The original 2827/689 residuals, 48,000 unknown frames per owner
and both delivery-gap failures remain unchanged.

## Existing Next Invocation, Not Executed

Once the exact permitted non-admin token and existing-input access are actually
available, the original reviewed command is:

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive `
  -File "D:\MoirenTest\20261105-terminal-recorder-audit-0125\read-terminal-recorder.ps1" `
  -ManifestPath "D:\MoirenTest\20261105-terminal-recorder-audit-0125\input-manifest.json" `
  -ExpectedManifestSha C0E5C991D73011054A1C096AD17C04E600801455EC31B6E3C9B42D5AEE74F835 `
  -ExpectedScriptSha 2B239259A4D6399A1468434FA16CD9DC5CA6D914A89A1918329DE82BA96535DD `
  -ReviewedOffline
```

Use the existing synchronous receipt discipline: disabled stdin, create-new
host stdout/stderr files, strict known-host SSH, finite 120-second transport
observation and no repeat after ambiguous execution. The unchanged reader
checks current original pins, protected identities, persisted policy and
resource limits before/after its one existing pinned ffprobe child. A normal
exit alone is not enough: verify the closed successful receipt, input pins,
empty stderr and cleanup before locally analyzing it once. Do not rerun the
old combined deployer or overwrite this now-populated package directory.

## Exact-Head Software CI Closure

Run `41902537510` completes all20 jobs successfully on exact `fc05652`,
including Windows/Ubuntu tests and strict Clippy, driver host, all Miri suites
and the aggregate. The final aggregate completes at 04:22:49 in GitHub's UTC
timestamp converted to +08:00. The existing watcher exits0; it is not
restarted or replaced. Its host wall timestamp is recorded separately and
does not supply latency evidence.

This closes software CI for the published PCM-audit and S4 pre-client repairs.
Later readiness/handoff documents do not change that tested code. No native
S1-S5, smaller default, VAD/Process winner, sub-10-ms result or device-limited
impossibility follows from this green CI or inert placement.
