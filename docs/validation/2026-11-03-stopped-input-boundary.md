# Stopped UI input-boundary comparison

Date: 2026-11-03, Asia/Shanghai. Software baseline: `3d2b5ca`.
This closes one bounded automation hypothesis, not S1-S3 native/audio acceptance.
No production code, audio policy, deadline, quality gate or default changes.

## Isolation and question

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-input-boundary-20261103-0040`.
Target root: `D:\MoirenTest\20261103-input-boundary-0040`.
Task: `Moiren-InputBoundary-20261103-0040`, lifecycle `task`, held worker150s,
task240s, Limited interactive session1. A new immutable run directory prevents
relaunch. The app is unchanged from the previous stopped control:

```text
34,150,912 bytes
67025240CDDA7CFC6C277928FE20897806D4BB8D498D21B7E99F7003071AE974
```

Provenance remains clean `8d080f5` plus exactly three reviewed joined-cleanup
diagnostic overlays subsequently committed in `ceaf61d`, not unmodified HEAD.
The binary is copied within the target; only small pinned helpers are uploaded.
The restored empty-graph/null-session fixture prevents automatic audio startup.
No source, transport Start/Stop, Evaluate/Apply, global mouse input, explicit
foreground/focus operation or foreign-window mutation is added. The native
AutoSetFocus=true arm can implicitly request focus on the owned target; it is
not described as globally side-effect-free.

The fixed sequence is eight managed Settings/Close calls in PS/C#/C#/PS order,
then eight native calls on one `CUIAutomation8` / `IUIAutomation2` client and
one MTA thread with focus true/false/false/true. Patterns are prepared outside
timing, scoped to the exact owned root, unique enabled button name/type/PID,
then explicitly released on their creating thread. Native QPC brackets only
Invoke; its HRESULT and UI effects are separate receipts. No timeout is set.

## Actual remote result

The task launches once at00:52:16 +08:00. Worker receipt spans00:52:17.428 to
00:53:05.404. The intended stopped runtime is observed: Start present/disabled,
no Stop, and no audio opening/start in the final drained log. All16 measured
Settings/Close calls return and their expected UI effects are separately seen.

| Measurement, four samples each | Minimum ms | Median ms | Maximum ms |
| --- | ---: | ---: | ---: |
| PowerShell outer Invoke | 2004.993 | 2017.436 | 2143.205 |
| Precompiled C# inner Invoke | 2008.383 | 2012.737 | 2025.283 |
| Native inner, AutoSetFocus=true | 2010.026 | 2015.978 | 2022.855 |
| Native inner, AutoSetFocus=false | 2009.916 | 2016.394 | 2021.710 |

Each native arm reads back its focus value. Initial, per-arm and final reads
retain connection2000ms / transaction20000ms. Disabling automatic focus does
not remove the roughly two-second call delay in this experiment. Precompiled
C# and direct native measurements likewise do not support PowerShell dispatch
as its sole cause. These are small matched operational samples, not a provider
root-cause proof or audio-latency measurement. Numerical agreement with the
connection timeout is not evidence that this timeout actually fired.

At the Settings point, read-only WindowFromPoint observes PID13100,
class `LockScreenBackstopFrame`, title `Backstop Window`, rather than the owned
app PID26196. HitRoot is the same foreign HWND; HitIsChild=false and
HitExactReceiver=false. Caller, owned window and opened input desktop names
all read `Default`; the explicitly opened desktop handle is closed. This
explains why an exact-receiver click would refuse at this point in this run.
It neither proves the earlier run had the same cover nor establishes session
lock/security state or a DPI/coordinate defect. No attempt is made to move,
close, hide, unlock or otherwise bypass the foreign window.

Normal Settings/Diagnostics/fully-revealed Quit completes; app exits0,
worker/supervisor/task exit0, completed=true, and no forced termination or
cleanup errors occur. Native patterns/context close successfully. No app,
source, probe or analysis owner remains. Protected OBS/QQMusic creation
identities, defaults, render volume/mute and running driver hash remain
unchanged. Thirteen compact files are individually length/SHA-256 verified
from a12,437-byte archive; its SHA-256 is
`CB6473BC83B60E20B1771128640698AA6931B1F1F2D4ED0C75F2499EC2350BFC`.

## Software checks and visualization

An exact clean `git archive 3d2b5ca` excludes the27 original dirty/cache files.
While the remote task runs, host-only software regressions run independently:

- SRC zero-refill:5passed.
- Marker/rate/qualification analyzers:19+8+24=51passed.
- Headless measured UI:20passed.

All three requested commands exit0. They do not open host audio devices or
launch the host desktop application; they are not native acceptance. No new
full-workspace pass is claimed for this evidence-only slice.

Evidence helpers pass61native static/compile/export checks,6observation
compile/pure/source checks and13worker source invariants. Actual missing-helper,
settings-getter and release-protection RED failures are retained. Review catches
a pattern acquired before persistence entered its finally guard; the corrected
runner releases it even if persistence/preparation validation fails. Static
checks and compilation do not certify native behavior by themselves.

`timing-samples.csv`, `timing-summary.json` and visually reviewed
`timing-comparison.png` derive from all16 actual calls, preserving both zero-
based and explicitly zoomed axes. They state NOT AUDIO LATENCY. No PCM or large
recording is transferred for this comparison.

## Stop rule and remaining work

Close the AutoSetFocus/PowerShell-only hypothesis; do not repeat an unchanged
UIA or audio soak, change the400/900ms timing contract, or invent a product
timing fix from this result. The root cause remains unknown, and this input
protocol still cannot establish the short-window native Stop timing.

Successful closed-prefix Stop/join/Evaluate/Apply, lower-setting native waveform
quality, matched route/clocks and calibrated physical/sustained qualification
remain open. First-packet investigation stays deferred. The original S1-S3
objective is not complete, but no new mandatory software hole is demonstrated.
Absent physical hardware or automation acceptance must not block independent
product development. Future useful remote tests can run concurrently with
development under exact process/config/artifact isolation; there is no claim
that a long-duration audio test is currently running.
