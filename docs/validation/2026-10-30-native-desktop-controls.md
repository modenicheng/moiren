# Native desktop controls and shutdown receipt

Recorded 2026-10-30 (Asia/Shanghai), continuing the 2026-10-29 audit.
Physical target: `LAPTOP-NP49RG6H` / `192.168.12.97`. Its unchanged clock
reports 2026-10-25; no cross-machine wall-clock subtraction is used.

This adds native production-desktop control evidence. It does not establish
Process budget Apply, visual acceptance, waveform continuity or physical
latency. The full S1-S3 objective remains incomplete.

## Application and execution boundary

The deployed `moiren-app.exe` is unchanged source `e195bfe`, 33,385,984 bytes,
SHA-256 `09A0053326179928FB20E6F90DD1472501EF4A7F2D10907BEE0E9C550566F93A`.
The probe remains `f72e03c`. The installed MoirenVad service remains Running;
its loaded-path SYS hash is
`CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.

SSH runs in session 0. The existing user's active console is session 1.
Temporary, bounded tasks run hidden PowerShell helpers with the user's
interactive, limited token. Each app process has its own isolated APPDATA
under the owned test run; normal user preferences are not used.

The app itself is started hidden. An exact process-ID/title HWND lookup,
followed by UI Automation `FromHandle`, identifies the production Moiren
window. `PrintWindow` requests make its accessibility tree available. The
returned screenshot is black and is not visual-rendering evidence.

Although the helper does not click Start, ordinary application startup opens
the default native audio owner. Logs retain the Realtek Shared Float32 output
with a 1056-frame buffer and Steam Streaming Microphone input with Shared
format conversion. Engine processing is 48 kHz / block 256. This default
project includes Gain and Balance; it is not S1's minimum IO/Bus route.
No compilation overlaps this native activity.

## Confirmed native actions

Final run: `desktop-native-controls5`.

| Action | Authoritative result |
| --- | --- |
| Invoke Stop | UI changes to Start; logs record owner cleanup finishing and generation 1 being reaped with cleaning 0. |
| Invoke Settings | Native accessibility tree contains the actual settings tabs. |
| Invoke Input buffers | Native tree contains measured-budget, live-window, retained-evaluation and Expert buffer controls. No Process source is selected and no budget Apply is invoked. |
| Invoke Diagnostics | Native tree contains the application's Quit Moiren control. |
| Scroll within settings | Two targeted `WM_MOUSEWHEEL` messages, delta -480 each, bring the complete Quit button inside the measured content viewport. |
| Invoke Quit Moiren | Process exits normally within the 15-second wait, exit code 0, with no forced termination. |

Before the final Quit invocation, the button's UIA rectangle is
`[632, 1380, 1536, 49]`; the footer starts at y=1542. The test checks the
button's full rectangle against the tab/footer content bounds rather than
trusting `IsOffscreen` alone. These are native accessibility invocations and
targeted scroll messages, not physical pointer clicks on a visible window.

This extends the earlier native idle-renderer and CLI/service receipts.
It proves that the production desktop can dispatch Stop to the native owner,
navigate these settings and exit normally under the declared test conditions.
It does not prove a successful native desktop Process-owner replacement,
requested/active budget confirmation or sustained audio quality.

## Earlier harness failures retained

The archive preserves all eight attempts rather than converting task exit 0
into application success. The helpers catch errors in `result.json`, so the
scheduled-task result alone is insufficient.

- `desktop-session-audit1` requires a visible HWND despite hidden startup.
  `desktop-session-audit2` selects the console window with an overly broad
  title match. Neither is a product-window failure.
- `desktop-session-audit3` waits for the exact Moiren title and obtains its
  native accessibility tree. Its black PrintWindow image and forced process
  termination are not rendering or graceful-cleanup passes.
- `desktop-native-controls1` searches the desktop UIA root, which does not
  yield this hidden window. The next attempt binds the HWND but inspects the
  tree before the explicit draw request. Neither dispatches a control action.
- `desktop-native-controls3` and `4` complete Stop/settings navigation, but
  invoke Quit while its rectangle is below the content viewport. UIA reports
  `IsOffscreen=false` and Invoke returns, yet no exit follows. Run 4 also
  retains a timed-out, noninvasive CDB attempt and partial thread stacks.
  These observations do not establish an application shutdown deadlock.
- The final run changes the harness to scroll the target fully into view
  before invoking it, and then verifies real process exit. No Rust, driver or
  dependency code is changed.

## Evidence, cleanup and remaining work

File-only archive: `desktop-native-controls-e195bfe-files.zip`, 92,963 bytes,
SHA-256 `8BD40CCFC1785AFC9D2DE14EBEBF87F737CC3C100B6C1C6A18CCB01BD9A2C489`.
All 70 manifest-listed files / 71 entries are host-verified by exact entry set,
length and SHA-256. Reports, native trees, original failures, logs, scripts,
binary/driver identity and cache state are retained.

The original Compress-Archive output additionally contains 15 empty-directory
entries and is refused by the existing exact-entry verifier. Repacking the
same staged files as file-only entries preserves every manifest-listed file;
the verifier then passes without changing its acceptance logic.

After verification, all eight terminal, action-checked helper tasks are
unregistered. Fresh read-only catalog data confirms unchanged default-role
IDs and Realtek mute=true / volume=0. No app, probe, compiler or debugger
process remains. The separate post-cleanup receipt is
`target/physical-1921681297-20261029/desktop-native-controls-post-cleanup.json`,
SHA-256 `57DFA202DB4EEF05A52BC61A440D17B9A933EB67CE7392A8A39BB7749BE75E02`.
It is outside the preceding archive.

Remote main remains clean at `497ac7c`; task source remains clean at `f72e03c`.
Shared Cargo target retains 915,203,047 bytes. No build, cache cleanup, driver
installation or push is needed for this receipt.

Native desktop Process budget Apply and confirmation remain open. S1 still
needs a declared usable physical output, an independent common reference and
calibration, and accepted waveform evidence. No lower default, path winner,
continuous-output pass or stable sub-10-ms result follows from these controls.
