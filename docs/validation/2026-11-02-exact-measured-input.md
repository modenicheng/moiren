# Exact measured Process input - 2026-11-02

## Scope and implementation

Continue S1-S3 development while finite native tests run on the separate
`LAPTOP-NP49RG6H` target. Host work is limited to compilation, headless tests
and analysis; no host audio process or device is started or stopped.
First-packet investigation remains deferred. Physical recording/calibration
equipment is unavailable and does not block this application configuration work.

The CLI can already request an admitted target of1536frames, but the desktop
only offered fixed presets that omitted it. The measured Process editor now
offers exact integer text entry for capacity, target and jitter, preserving the
existing presets as an explicit alternative. Values are48kHz input frames:

- Capacity4..262144; target2..262143; jitter0..262143.
- Parse validity does not replace native relationship/full-budget admission.
- Invalid, empty, fractional, negative or out-of-range text blocks Apply.
  The last valid draft is retained for editing, never silently dispatched as
  a fallback for invalid text. Activation rechecks current text, not only the
  preceding rendered button state.
- Existing source PID/creation identity, graph/session/native-owner freshness,
  explicit restart and exact active confirmation remain unchanged. Inputs are
  session-local; no measured proof is persisted as an unconditional preset.
- Switching back to presets discards invalid text explicitly while retaining
  the last valid values. Input entity identity rejects stale subscriptions;
  busy transactions do not accept edits. The shell respects input typing focus.

Read-only review finds no confirmed correctness bug. Ordinary, live-window and
evaluated activation share the same current-text conflict guard. Diagnostic
evaluation can use the displayed last-valid draft without starting audio;
evaluated Apply still requires valid text and the matching admitted candidate.
Direct live/evaluated stale-event coverage remains less extensive than ordinary
Apply coverage; this is a coverage limit, not a demonstrated bypass.

## Parallel native controls

The preceding render-capable executable is independently exercised through real
desktop UI Automation at target2048/capacity16384/jitter96. A90-second hold
after owner replacement retains matching requested/active summaries. Stop/Quit,
app/source exit0, no remaining owners and unchanged original OBS12648,
QQMusic15096, default roles and render volumes are confirmed. This control
does not exercise the new exact1536editor.

Its remote run is
`D:\MoirenTest\20261102-s3-desktop-render-0039\runs\desktop-render-target2048`.
The raw `NativeConfigConfirmed=false` is an incorrect log-format regex, not a
native refusal. Separate preserved postanalysis confirms the actually logged
`buffer_frames=738 requested_period_frames=Some(336)`. That log alone does
not independently read the actual period or effective render/input queues.

Downloaded and hash-verified control artifacts, visually inspected screenshot:

| Local artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-desktop-render-0048-result.json` | 206439 | `5895EA1EF3D367CC5FD3144DAA14C3B5379D5F9CA847AD0B6351EC487732E201` |
| `s3-desktop-render-0048-active.png` | 310326 | `91D4D4B775271767580ACD304478F7F606A8BE96756345425DBA9BEF0F04E26B` |

Root evidence directory throughout:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

## First exact run and discovered accessibility defect

First exact app SHA256
`D410244F5EBAEC84F7E8BE9F14FC2F83A5C5E39F3A160F66AF33B86AF9302038`,
33930752bytes; manifest `s3-exact-source-manifest-0051.json` pins HEADa571467
and all retained dirty sources. Remote run:
`D:\MoirenTest\20261102-s3-exact-input-0050\runs\desktop-exact-target1536`.

Native UIA finds exactly three scoped owned Edit controls and uses their
ValuePattern to enter16384/1536/96, reading each value back. Apply replaces
owner1with owner2. Requested and active summaries match the exact source
PID5308/creation134380255543059852 and selection1, both immediately and after
180seconds. Render512/wake64/callback64, Shared period request336 and Engine64
are preloaded independently. Source also renders directly to DAWN, so this
is configuration/lifecycle evidence, not isolated waveform or physical latency.

The final running Diagnostics UIA capture omits its named rows: they have
`aria_label` but no accessible role. A screenshot shows visible diagnostics,
while the native tree contains only interactive controls. A focused regression
fails with `a11y_role=None` rather than Label. Adding a role makes a focused
role test pass, but a second180-second native attempt still has empty names.
A15-viewport scroll control shows the same visible diagnostics and confirms
normal Stop/Quit/app-source exit0; its overall diagnostic verdict stays false.
Role-only was insufficient, not a completed native fix.

The AccessKit Windows name implementation derives this text role's UIA Name
from its value, not its label. Existing measured summaries already set both.
A second RED/GREEN regression retains the observed missing value, then verifies
identical `key: value` label and text value plus the Label role. The final helper
preserves row IDs, visible text, layout and audio semantics. Read-only review
finds no concrete issue in either small accessibility change.

This first run remains incomplete: the diagnostic assertion throws before
Stop/Quit, fallback window-close does not exit, and only the task-owned remote
app is terminated. Source exits0, cleanup leaves no owners, original OBS/QQMusic,
default roles and volume/mute controls are unchanged. Do not relabel the run
as graceful completion or overwrite its failed receipt.

| Downloaded artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `s3-exact-attempt-result-0101.json` | 176397 | `6866613B8BB8A25D4EA647B534361104F41BB2DC0254683DEFB7B7659C76472D` |
| `s3-exact-attempt-active-0101.png` | 298869 | `74435A476C6795DFCC72E82FC29155D20FA686E67490C79FF0113F476AD24BB0` |
| `s3-exact-diag-0058.png` | 279483 | `53213CC376EB92441877998D06AD692EC56411957A8C14A7A30ACB68A2B36B42` |

Both screenshots are visually inspected. The raw failed tree and app log remain
remote with closed-file hashes in `s3-exact-attempt-manifest-0101.stdout.log`.

## Final executable

Usable remote app:
`D:\MoirenTest\20261102-s3-exact-value-0115\bin\moiren-app.exe`.
SHA256 `604B598519405FB7E8C1B70B47E680063E3BBBFF6965EFCE44F06D0F9313AC4E`,
33930752bytes. Manifest `s3-exact-value-source-manifest-0115.json` pins source
and binary; retained unrelated diagnostics are disclosed, not claimed committed.

Loaded611driver remains Running, SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
Source probe remains separately pinned SHA256
`3FB5330D790767F68B0114162F137E685AE472D17717CF5C4967D8EFA81A2E28`;
its revision is not inferred from the app revision.

Final task `Moiren-S3ExactValue-20261102-0115` completes01:16:21+08:00.
The real desktop enters16384/1536/96and replaces owner1with owner2, then
confirms the same requested/active source identity after a30-second hold.
Running native UIA now reads all three required diagnostics directly:

- Master actual48000Hz,336frames(7.00ms),capacity738; Client3,fallback none.
- Effective prepared render Target512/capacity738frames.
- Effective prepared input Prime1536/capacity16384frames.

At the captured observation, underrun/dropped/reset/backlog recovery counters
are0. Delivery has3421gap observations,max111367*100ns=11.1367ms,and two
gap-bound violations; packet/unavailable-clock/regressed-clock counts are0.
Therefore `Completed=true` proves configuration/UIA/lifecycle completion, not
clean sustained service. It is not a waveform-continuity or physical result.

Native Stop/Quit,app/source exit0,no forced termination,cleanup errors or
remaining owners. Original OBS12648/QQMusic15096,default roles,render
volume/mute and boot identity remain unchanged. Final audit finds task
Ready/result0. Root receipts `s3-exact-value-manifest-0119.stdout.log` and
`s3-exact-value-result-0118.stdout.log` retain native timing and cleanup.
Downloaded result `s3-exact-value-result-0120.json`,246703bytes,SHA256
`B947D2BC8566921090AAB0DC2A4E514615A825DC485D551B45C62E11FA3C1E30`;
visually inspected active screenshot `s3-exact-value-active-0120.png`,299528bytes,
SHA256 `41DEF6C31D62F53BF3357D27AC12BE5A4E48EA832438ED288A526237C073118D`.
Both downloads match closed remote file length and SHA256.

The intermediate role-only app SHA256
`F482036CCECB4F3D1F8FBAE9FAB9A272069B17ACF8C61CCC05C87A21D6731642`
remains in the0103deployment. That180-second attempt also fails the diagnostics
assertion and requires owned-app termination. The0110scroll control instead
defers the assertion verdict until after normal Stop/Quit, exits app/source0
and leaves no owners. Its raw `Completed=false` is preserved. Downloaded result
`s3-exact-scroll-result-0115.json`,306985bytes,SHA256
`5F5A34485428E14B6C184A3D38C4EA3A3573919BC5116299E2128BBF327F457D`;
visually inspected screen `s3-exact-scroll-screen-0115.png`,279403bytes,SHA256
`0AA7B08CEBC0604B0F5FA7522005224D338B8704B886A1ACECF9718D7B2AABDE`.

## Software verification and delivery

Final source, including diagnostic text-value fix:

| Check | Result |
| --- | --- |
| `cargo test --locked --workspace -j 1` | 2089 passed,8ignored; exit0 |
| App library with `gpui-test`,`-j 1` | 615 passed,3ignored; exit0 |
| Workspace/all-target strict Clippy | exit0 |
| Release app build | exit0 |
| `cargo fmt --all -- --check`;`git diff --check` | exit0 |

Final logs:`s3-exact-value-workspace-0113.log`,`s3-exact-value-app-0113.log`,
`s3-exact-value-clippy-0113.log`,`s3-exact-value-release-0112.log`.
Regression evidence includes the original missing exact-open control RED,
exact nonpreset selection and confirmation GREEN,the18invalid all-field cases
with activation before the next frame,and missing-role/missing-value RED/GREEN.
Earlier intermediate API/type compilation failures are retained in their logs,
not counted as behavioral RED evidence. Two independent read-only review
passes find no confirmed correctness defect; native evidence is checked
separately rather than inferred from reviewer or headless results.

All18preexisting unrelated dirty-file hashes match the initial manifest,
including `gate.log`. They remain excluded from the scoped local commit.
No push,host audio/device execution,driver installation or memory update.

S1-S3 remain incomplete. No isolated continuous-waveform, matched VAD/Process,
calibrated physical result, defensible physical lower bound or permission to
promote a smaller default follows from these configuration controls.
