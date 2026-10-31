# Current application deployment and desktop retry - 2026-10-31

## Direction and scope

The endpoint direction remains the contract already merged in PR44:

| Windows virtual endpoint | Moiren graph role | Other applications |
| --- | --- | --- |
| Virtual speaker / Playback | Input / Source | Output / render |
| Virtual microphone / MIC | Output / Sink | Input / capture |

Root main and the remote main ref both remain `6b4ffe6`. Foundations
PR43/44/45/47 are merged; experimental PR26 is not merged wholesale.

Four focused `vad_export` application library tests pass on experimental
`e1a7c2f17749b2abb4f98a2c3fef36af67c6aed5`. They cover app-side input/output
directions, wrong-shape rejection and atomic replacement/clearing of physical
bindings. The current application tree is unchanged from `af70aa9`; this
receipt refreshes binary provenance rather than claiming another routing fix.

## Build and deployment

The first application build using the shared root target fails with missing
dependency APIs. The same tracked source passes the focused tests and release
build using the existing worktree-local target, jobs2. No source repair or cache
clean is performed. Incompatible shared artifacts are suspected, not established
as the sole cause of the first failure.

The release EXE is33697792 bytes, SHA256
`72CB86DA667A7D098D6B98338BB8324CB720BCCFAB2A494D3A9F0F74A85A9D71`.
It is copied side by side to the physical target at
`D:\MoirenTest\20261031-app-e1a7c2f\bin\moiren-app.exe`.
Target length/hash and source pin match. Help, monitor help and the exact
offline gain0.25 output all exit0, with isolated APPDATA for accepted smokes.
The first help attempt has valid stdout/empty stderr but no usable
Start-Process exit status; it is retained, not counted as an accepted smoke.

Installed driver609 remains Running, loaded-path SYS SHA256
`C9F7FF8273239B97436DF75A5D9E7584347F54A79CCFA21E1889EE546E4C591F`.
No driver install, reboot, cache cleanup, endpoint-volume/mute or default-role
write occurs. Earlier app copies and driver rollback packages are retained.

## Current binary native desktop attempt

`desktop-e1a7c2f-process1` runs under the existing console session1 with a
limited interactive token. SSH remains session0. A hidden production desktop
uses isolated preferences and exactly `Process -> unity Bus -> DAWN`,
48kHz/F32/Engine256. The separately owned changing-signal source explicitly
renders only to the same authorized DAWN headphones. This is not an independent
reference or a physical latency measurement.

The helper is adapted from the historical successful desktop process5 test,
with exact app/probe/catalog/driver pins. Review finds and fixes three harness
risks before acquisition: catalog execution after failed preflight, cleanup
exceptions/unbounded joins, and success being assigned before final cleanup
and unchanged-state checks. PS5.1 parsing and a real wrong-host rejection
fixture pass. An independent review checks the corrected script SHA256
`6BCD83F8742B18721081479CAB89D36AA48D84F562DF3ECCFF06407CC437FA2F`.

The task launches at UTC03:08:51; the script records its start at03:10:53.6825757
and final receipt at03:11:10.8308212. Scheduler launch is not treated as script
execution or audio-start time. The helper's `01-running.json` tree is actually
saved before clicking Start; its filename is not Running evidence.

The actual Start Invoke returns. Native logs then confirm generation1 Running,
the pinned DAWN endpoint, Shared Float32 stereo48kHz, and1056 actual native
buffer frames. These frames are not the Process input target or E2E latency.

The next Settings Invoke throws `ElementNotAvailableException`. Logs at
11:11:00.388 retain GPUI `WindowsWindow::drop` errors `0x80040102` and
`0x80070578`. The failed tree has only a root window. Neither a stale HWND,
an accessibility action defect nor a production lifecycle regression is yet
established as the cause. No measured Apply, requested/active comparison or
generation2 replacement is reached.

WM_CLOSE does not exit the owned app within10s; bounded owned-process forced
termination follows, exit-1. Source Stop/join exits0 after15.254419s with732000
acknowledged source frames and an incomplete schedule. Final `Completed=false`,
task result1, no remaining audio/build owners and no boot change. Render
volume/mute/default-role snapshots are unchanged. Empty `CleanupErrors` does
not mean graceful app shutdown or a joined final telemetry tally.

This is a retained failed current-binary regression, not a replacement for or
retraction of the historical `3ac8484` desktop requested/active PASS. Hidden UIA
evidence is not visual acceptance; no PCM quality or latency pass follows.

## Evidence and open gates

Host evidence: `D:\coding\moiren-evidence\20261031-app-e1a7c2f`.
The file-only archive `desktop-e1a7c2f-process1-evidence.zip` is35838 bytes,
SHA256 `F27684E060243E6983B1913BD691B5623AC7746B262C74FA295D11034F7C6C1E`.
Root and an independent read-only audit verify all25 payload files plus
manifest, exact member set, CRCs, lengths and hashes. Target originals remain.

Next work is a targeted window/action-lifecycle diagnostic, not repeated blind
UI Invoke retries. The separate opt-in MIC native-period probe control is
developed in an independent worktree. Smaller targets require actual period
and delivery admission in both directions; no default or quality threshold is
changed to obtain a pass. Full S1-S3 and calibrated sub-10-ms qualification
remain open.
