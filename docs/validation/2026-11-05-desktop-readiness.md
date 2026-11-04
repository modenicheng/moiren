# Current Remote Desktop Observation Before S3 Acceptance

Date: 2026-11-05, Asia/Shanghai. Published code: `fc05652`.
This is a one-time current input-desktop/foreground observation, not an
application, audio, waveform or physical-latency acceptance run.

## Question And Reused Observer

An earlier stopped-input attempt only reports a combined child/obstruction
exception. A later frozen observation identifies a different-PID external
`LockScreenBackstopFrame`. Neither historical result proves the present desktop
state, particularly because ordinary exact-demand desktop acceptance succeeds
later on 2026-11-03. The recorder metadata reader's own non-admin-token guard
also does not decide whether a separately Limited/Interactive desktop task can
run.

The existing `s1-vad-waveform-joined-20261103-0515` observer is reused with only
the fresh remote directory and task name changed. Reverse substitution recovers
both original scripts byte-for-byte. There is no new API or input transport.
The observer reads the input desktop and foreground HWND/title/class. It sends
no input, changes no foreground/window state and launches no app or audio.

Local evidence:
`D:\coding\moiren-evidence\20261105-s4-idle-admission\desktop-readiness`.
Remote package: `D:\MoirenTest\20261105-desktop-readiness-0405`.
Task: `Moiren-DesktopReadiness-20261105-0405`.

Both scripts parse without errors using explicit Windows PowerShell
5.1.26100.9711 and PowerShell 7.6.6 executables. Initial receipts mistakenly
labelled PS5 actually use PS7 and remain retained; only the verified explicit
interpreter receipts support the two-shell result.

## Actual Single Execution And Closure

Preflight confirms the expected host/SID, SSH session0, Windows build22631,
24,333,804 KiB free memory and 144,440,942,592 bytes free on D:. The fresh
package and task are absent. The two transferred scripts match their exact
prepared lengths and SHA-256 hashes before the one task launch.

The existing launcher requests Interactive/Limited, IgnoreNew and a 45-second
execution limit. The observer checks actual session1. No existing task is
restarted and no user window, process or setting is manipulated.

The task terminates Ready/result0. Its exact observer command has no remaining
process. The collected 330-byte receipt independently matches the remote file:

```text
FB3F084E4B60CC31838709E829AD05CD08A3568CEB2C24413BFD0F6A66497411
```

At the observation:

- Actual session: 1.
- Input desktop: `Default`.
- Foreground HWND: 66882.
- Foreground class: `Windows.UI.Core.CoreWindow`.
- Foreground title: Windows default lock-screen interface.
- `audio_launched=false`, `ui_action_sent=false`, `qualified=false`.

Host collection completes at 04:08:02; the target records
`2026-11-04T20:08:17.3269760Z`. These are different machines' wall clocks,
not latency coordinates. The foreground observation is sufficient to defer
owned-window input acceptance; it is not an authoritative WTS locked/unlocked
flag, successful input consumption or a cause of the earlier UIA timing.

Before/after OBS12648 and QQMusic15096 creation FILETIME identities match.
That preserves process identity evidence, not uninterrupted foreign audio.
Both deployed script pins still match after closure. `completion.json`,
the preflight, launch, exact task polls, closed pins and downloaded original
receipt retain the actual result. No audio probe, app, source, recorder,
ffprobe, driver or device exercise runs.

The closed local compact archive has 53 payload files plus its manifest,
22,484 bytes, SHA-256
`E6DD25351972CBB25E3440EB9026A68D8819434610909EB1165961FC79F9A127`.
Every entry is independently byte-verified, including the remote-pinned
original receipt. There is no raw audio or new native acquisition in it.

## Limits And Next Prerequisites

The reused observer checks receipt absence then calls `WriteAllText`; it does
not provide atomic CreateNew against a hostile concurrent writer. This actual
execution uses a newly created isolated directory, a single newly registered
IgnoreNew task, verified script pins and post-terminal receipt hashing. No
concurrent writer or overwrite is observed. Its opened desktop handle is closed
in `finally`; the terminal process/task evidence closes this actual execution.

S3 still needs an unobstructed interactive target and successful ordinary
retained Stop/join/Evaluate/explicit Apply. The existing retained archive's
overlapping packet/read envelopes remain an independent genuine refusal.
No repeated capture is started in hope of obtaining an unambiguous archive.

The 03:58 VMware inventory separately returns zero running VMs; driver-target
unlock/start remains necessary for S4. The metadata reader's non-admin
foreground requirement remains unsatisfied and is not bypassed through this
observation task. Full S0-S5 acceptance, original waveform/gap failures and
physical qualification remain open.
