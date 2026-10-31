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

## One-pass HWND and UIA diagnostic

`desktop-e1a7c2f-process2` retains the same app, source, driver, graph and budgets.
Only the helper adds exact process identity, top-level window inventory, fresh
HWND binding, root/Settings runtime IDs and single-pass tree observations.
Settings Invoke is submitted once; an exception causes observation, not a
second action. The helper is32296 bytes, SHA256
`109FE8F13FB5E3E0FE5E90BE6BDCECF1E25EAB9874241F7411517566976DD064`.
Both author and root independently run PS5.1 parse/contract/wrong-host refusal
fixtures. Cleanup and final acceptance remain identical to process1.

The script runs UTC03:27:11.2667564 through03:27:27.8764063. App PID15476,
creation time134378908325254895, session1 and HWND591536 remain identical at
before-Start, after-Stop-button-found, pre-Settings and exception observation.
Every inventory finds the same valid `Zed::Window`; every PrintWindow returns
true and every fresh UIA root matches the owned PID/HWND. Node counts are
21/21/21/1. The exact Settings button is exposed before Invoke, but the single
Invoke throws ElementNotAvailable and the next tree contains only the root.
No main HWND replacement or loss is observed at these sampled boundaries.
This substantially narrows the stale-handle hypothesis, but does not prove
every interleaving or the sole cause of the missing subtree.

Native Start again reaches generation1/DAWN1056. No Apply is reached; the owned
app again requires forced termination, source exits0, and final Completed=false.
No diagnostic/cleanup error, remaining audio owner, boot change or endpoint
volume/mute/default-role difference is retained. Original failure is unchanged.

The local locked GPUI source clears a11y nodes/bounds/actions each frame, while
cached prepaint/paint reuse omits that state. This provides a concrete separate
candidate mechanism for missing cached controls. Historical3ac8484 already
contains Mixer caching6737864, but not Graph-wide cachingfb4671f. A scoped
accessibility-active cache bypass and genuine native regression are next;
neither a fix nor complete causal explanation is claimed from source reading.

Process2 archive is46091 bytes,31 payloads plus manifest, SHA256
`C3837EF3C67EB34FF9B279CA81C03B459E6BFF5D6A86E450F3B281518816996C`.
Root verifies CRCs, exact member set and every length/hash. After host-side
verification, both terminal action-checked tasks are unregistered; target
originals and archives remain. No periodic retry or task remains installed.

The separate opt-in MIC native-period probe control is
developed in an independent worktree. Smaller targets require actual period
and delivery admission in both directions; no default or quality threshold is
changed to obtain a pass. Full S1-S3 and calibrated sub-10-ms qualification
remain open.

## Accessibility-active cache bypass and actual desktop GREEN

`bdf3766` integrates independently reviewed `421bc4e`. The four app cache
entry points (toolbar, status, Graph controls and Mixer controls) bypass GPUI
paint/layout caching only while `window.is_a11y_active()`. Their sizing/flex
wrappers and inactive cache branches remain. No dependency, audio, driver or
buffer default changes occur. Graph19, Mixer19 and measured Settings28 tests,
strict app gpui-test all-target Clippy and formatting pass in the isolated
author worktree; those tests explicitly cover the inactive performance path,
not fabricated headless accessibility activation. Independent source review
finds no blocking issue, while noting native geometry and activation coverage
still require actual observations.

Root builds the integrated release with jobs2 and a worktree-local target.
The EXE is33715712 bytes, SHA256
`61FA3D92357BC557CBF09299A4028456ABE9AA057AC04E13920889E9BCB338DF`,
staged side by side at
`D:\MoirenTest\20261031-app-a11y-cache\bin\moiren-app.exe`.
Target size/hash, help, monitor-help and exact offline gain0.25 all pass with
exit0. The inherited `EarlierHelpAttempt` receipt text describes the old
e1a7c2f control, not an ambiguous attempt of this new binary; a separate
provenance note clarifies this without overwriting the receipt.

`desktop-a11y-process3` changes only app revision/hash and deploy/run identity
from diagnostic2. Source/catalog/driver pins, graph, candidate, invocation,
cleanup and final gate are unchanged. Root PS5.1 parsing, contract and real
wrong-host refusal fixtures pass; an independent read-only helper safety
review finds no blocker. Helper SHA256 is
`FA73C8612FF1CE3D41454D1C984B423CC8EEDA92E2061AF885BA0D69706B945F`.

The single limited console1 task launches UTC03:47:00; actual script execution
is03:49:00.8354686 through03:49:32.2184778. The app logs actual Accessibility
activation. PID9512, creation134378921421618925 and HWND722634 remain fixed
at all four diagnostic boundaries; node counts are21/21/21/21, not a
root-only failure. Settings Invoke returns, followed by Input buffers and
actual measured Apply. Native logs show generation1 cleanup/reap, then
generation2 Running on the same DAWN Shared Float32 stereo48k endpoint,
actual native buffer1056. Exact requested/active strings match the owned
Process PID5092/creation134378921419057287, selection1, target4096,
capacity16384 and jitter96 at48kHz. This is a diagnostic candidate, not a
lower-latency default or a latency measurement.

Close, Stop, reopening Settings/Diagnostics and Quit all return. Native logs
show generation2 cleanup/reap. App and source exit0 without forced termination;
Completed=true and task result0. Cleanup/diagnostic error arrays and remaining
owners are empty. Boot, render volume/mute and default roles remain unchanged;
there is no driver installation, reboot or cache clean. After host archive
verification, the exact terminal task is unregistered, with originals retained.

Host evidence: `D:\coding\moiren-evidence\20261031-app-a11y-cache`.
Archive `desktop-a11y-process3-evidence.zip` is67575 bytes,47 payload files
plus manifest, SHA256
`0157D759B35444757C26F2A21A1559C9FC36CFFCD90E1F87316EED272F69F7C4`.
Root verifies CRCs, exact member set, manifest lengths/SHA256 and extracted
bytes. The two preceding genuine native REDs remain retained unchanged.

This is a same-scenario actual desktop regression GREEN consistent with the
cache hypothesis; it does not prove every interleaving or render-frame tree.
The helper retains PrintWindow requests, not frame-counter evidence or a
visual screenshot acceptance. No waveform acquisition or calibrated physical
latency acceptance follows. Sparse-source/source-origin limitations and the
remaining S1-S3 quality/admission gates remain open.
