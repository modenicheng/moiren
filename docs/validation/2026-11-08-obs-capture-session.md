# Fixed-installation OBS capture lifecycle

Date: 2026-11-08, Asia/Shanghai. This extends the
[fixed-path reuse helper](2026-11-08-obs-reuse-helper.md), without launching OBS,
changing a driver, or claiming S1/S5 acquisition or waveform acceptance.

## Implemented integration boundary

The new modules compose one retained capture session:

- `scripts/obs-acquisition.py` prepares the existing installation, retains the
  launch owner, waits for authenticated readiness, and configures the same DAWN
  source, track1, no monitoring, zero sync offset and unity volume as the frozen
  matched-source experiment. Configuration mutations have no automatic retry.
- `scripts/obs-websocket.py` implements the authenticated OBS v5 JSON protocol
  with one deadline across connection/Hello/Identify/Identified, and one deadline
  across each request send/events/response. It checks exact response identities,
  types and status, bounds JSON and raw partial I/O, sanitizes exceptions, and
  poisons uncertain requests instead of repeating a mutation.
- `scripts/obs-recording.py` retains responsibility before StartRecord dispatch.
  A lost reply cannot be cancelled by an initial inactive observation. Cleanup
  requires observed active, one StopRecord dispatch, then observed inactive.
  Later observation may finish cleanup, but cannot erase an earlier failure.
- `scripts/obs-capture-session.py` retains the setup and recording objects before
  their operations, and combines their verdicts. A successful OBS normal close
  cannot override a failed recording transition or failed setup.

The fixed installation stays at
`D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable`. No executable tree
is copied, moved or created per run; the user's ordinary OBS is not adopted or
terminated. The existing identity/SID/executable/port/lease guards remain in
`obs-reuse.py`. No firewall or other security policy changes are introduced.

Each native case must retain `ObsCaptureSession` before `start()`, provide the
case's finite watchdog/check/launch/join and exact-owner normal-window-close
callback, and invoke `finish()` during its existing failure-safe teardown.
`session.client` exposes bounded requests; `session.recording.stop_response`
retains the StopRecord response for separate path/media validation.

Cleanup order is recording inactive -> websocket shutdown -> normal owned
window close -> exact process join -> bounded configuration archive -> lease
release. Unknown launcher ownership, unresolved recording or a live failed
close retains the lease; no next route, duplicate launch, automatic process
search, forced termination or stale-lock break is provided by these modules.
The enclosing runner still owns exceptional held-process cleanup and must
retain that failure rather than call it acceptance.

`capture_lifecycle_complete`, `boundary_complete` and `recording_complete` are
only transition/lifecycle facts. Every combined receipt keeps
`acquisition_accepted=false` and `qualified=false`. Media existence, length,
hash, tracks, waveform correspondence, continuity and calibrated latency remain
independent required evidence. Receipt creation is create-new, not overwrite.

## Verification and limitations

Evidence root: `D:\coding\moiren-evidence\20261108-102500-obs-acquisition`.

Fresh file/mocked-boundary suites pass:

| Suite | Tests |
| --- | ---: |
| Existing fixed-path reuse | 26 |
| Setup and normal-close boundary | 33 |
| Authenticated bounded websocket | 36 |
| Recording transition ownership | 14 |
| Composed capture session | 12 |
| Total | 121 |

Recording and capture-session suites retain failing missing-implementation
checks before implementation and passing final runs. Composed tests exercise
the actual JSON client with a fake socket, real lease/configuration helper and
retained fake process. Lost StartRecord and StopRecord replies poison the
actual client, leave recording unresolved, and prove no socket/window close,
lease release or dispatch retry occurs. Recoverable fake-RPC cases separately
exercise later cleanup observation without converting failure into success.
All four new suites are wired into the existing CI fmt job. CI surface-mapping
self-tests pass58 checks; syntax, ASCII and whitespace checks pass. The original
worktree's29draft/cache pins remain unchanged.

The actual target's existing websocket-client1.7.0 source is read back without
importing it or opening a socket; five source files total45831bytes. Its supplied
socket, timeout, immediate shutdown and subprotocol API match the adapter.
This is dependency compatibility inspection, not an actual network handshake.
The host has no websocket-client installation and none is installed for this
work. Real OBS/authentication/recording acceptance remains untested here.

The10:26 read-only target check reports the same October30 boot, no test/OBS
owners and BootAfterDeployment=false. Its uptime field is null and is not used
as uptime evidence. The613 transition still awaits the user's reboot; no native
test, automatic reboot, endpoint reset, repeated rename or unchanged failed
Process/VAD pair is launched.

The historical `20261107-matched-waveform-2255/run-guarded.py` is immutable and
retired: it still contains the old application-copy setup. These composable
modules do not rewrite or silently reactivate it. A separately reviewed/pinned
native runner still needs fresh post-reboot driver and external-owner guards,
then this session boundary instead of the copying setup. Full S0-S5 remains
open, including direct Removed, actual naming/identity preservation, accepted
matched waveforms and sustained/calibrated low-latency qualification.
