# Joined receipt native acceptance

## Scope

Continue the original S3 desktop Stop/join/Evaluate/explicit Apply acceptance
with the opt-in receipt binary described in
[the diagnostic implementation record](2026-11-02-joined-process-trace-receipt.md).
Native UI and audio execute only on `moiren-lan`; the development host performs
source inspection and headless checks only. Physical latency, a smaller default
and sustained waveform quality are separate gates. First-packet investigation
remains deferred.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-joined-receipt-20261102-2235`.
The original intent, deployment, failed launch commands and resource refusals
are retained; later observations do not rewrite them.

## Resource Diagnosis

The previous below-8-GiB readings were investigated rather than lowering the
guard or treating memory as an indefinitely external blocker. A read-only
inventory identified an obsolete owned report reader: remote PowerShell
PID11576, creation FILETIME `134380988356828571`, exact command SHA-256
`952C1DEA4F403A76D6EDCC0228C7CF1A52FF3C3860CE86BAD313099D35926C88`.
Its command read the preceding VAD report through `Get-Content -Raw` and
`ConvertFrom-Json`. It still held approximately25-28GB working set.

On November2 at23:08:15 +08:00, cleanup verified and held that exact
PID/creation/executable/command identity before terminating only that obsolete
reader. No development-host process or remote OBS/QQMusic process was stopped.
Its cleanup receipt recorded28,272,304KiB free. The first cleanup command's
PowerShell parser failure is retained separately; it made no process change.
Subsequent inspection must use bounded or streaming report extraction, not
repeat this whole-report PowerShell parse.

## First Native Attempt

The scheduled task launched once at23:08:36 +08:00 and reached a terminal
worker failure at23:09:23. It did not start an audio owner or activate the
source. The UI retained exact editor values16384/2048/96 and policy selections
4096/4096/480/2/10ms, but the runner's later exact Text assertion failed:

```text
Unique owned native text missing: Capacity / target / jitter (frames): 16384 / 2048 / 96
```

The archived screenshot shows the policy controls after scrolling; the native
snapshot does not contain the earlier summary Text. Source inspection further
finds that the summary uses the shared plain `label` div, without an explicit
accessibility role, unlike the feedback/admission Text nodes. Merely scrolling
is therefore not an established correction: the runner assumed a native Text
contract that this summary does not provide. This is a pre-Start runner failure,
not a measured audio failure or successful native receipt acquisition.
The application had to be forcibly terminated after its bounded close deadline;
that exit-1 remains recorded, not normal Quit evidence. The source exits0.

The exact app/source owners are gone. Worker cleanup records unchanged default
roles, no render-volume changes, unchanged loaded driver and protected OBS/
QQMusic identities, and no cleanup errors. Five compact result/log/screenshot
artifacts copied into `native-first` have matching remote/local lengths and
SHA-256. The independent numeric checker is not launched on this failed
pre-Start lifecycle. Successful Evaluate/Apply and actual joined-receipt
consistency remain open.

A runner-only correction must preserve the exact draft gate through the three
owned native ValuePattern editors, keep the original failure immutable and use
a new attempt directory. It must reject missing, duplicate, wrong-owner or
wrong-value evidence rather than silently omitting the gate.
It must not change the binary, policy,400ms hot window,900ms hot deadline or
history coverage requirements. No native retry is authorized by a timeout or
an unchanged failure alone.

## Runner Correction And New Attempt

The correction adds a joint current read of all three exact editors before
leaving that view. It requires three distinct runtime IDs, the current app PID,
enabled ValuePatterns and exact strings16384/2048/96. The successful candidate
path additionally checks the evaluated feedback's exact tuple; the existing
five policy fields, no-violation/exposure and final requested/active/native
confirmation gates remain unchanged. It removes only the assertion against the
plain-div summary's nonexistent native Text contract. No application or driver
source change is made.

A genuine missing-helper assertion RED is retained. Nine correction tests pass
in Windows PowerShell5.1, including changed/missing/duplicate/wrong-owner input,
missing ValuePattern, wrong evaluated draft and wrong policy refusals. Seventeen
existing runner checks also pass in a fresh parent execution. AST comparisons
verify the hot-window/lifecycle/cleanup tail and navigation helpers are unchanged;
an independent read-only review finds no blocker. These are runner checks, not
audio-quality evidence.

A new immutable package uses host evidence
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-joined-receipt-20261102-2320`
and remote root `D:\MoirenTest\20261102-joined-receipt-2320`.
The same34,150,912-byte app is copied remotely, not rebuilt or transferred again;
its SHA-256 remains
`67025240CDDA7CFC6C277928FE20897806D4BB8D498D21B7E99F7003071AE974`.
All eight deployed dependencies pass hash verification. Preflight has
26,550,280KiB free, no owner/task/run and unchanged protected identities.

The new task launches once at23:20:11 +08:00. All three native editor readbacks
now pass with distinct runtime IDs and the exact frozen tuple. The source starts
and later submits its full six-second schedule, stops normally and exits0.
The attempt nevertheless fails `Exact Stop never enabled within hot deadline`;
no successful hot-window timing receipt, joined-prefix evaluation, Apply or
independent actual receipt audit is obtained.

Logs show host opening at23:20:46.017, cleanup stage at23:20:46.036 and actual
audio owner generation1 starting at23:20:46.126. Thus the worker's initialized
`audio_started=false` is not authoritative: this field is never updated by the
runner, and the native log proves a late owner start during cleanup. The failed
hot helper does not persist its Start-return/monotonic timing observations, so
these wall clocks do not establish startup duration or the cause of missing
enabled Stop. The UI source uses a stable `transport` element ID; it does not
prove an inherently different Start/Stop element. No audio-loss, driver or
cached-element defect is inferred from this incomplete chronology.

The app requires bounded forced termination and exits-1; worker/supervisor
exit1. Task Ready/no owners is confirmed after cleanup. Default roles, render
volume/mute, loaded driver and protected OBS/QQMusic identities stay unchanged;
no cleanup error is recorded. The18compact files, including native snapshot,
source receipts, logs and supervisor outcome, are verified by individual
length/SHA-256 after a124,298-byte download; the settings screenshot is reviewed.
The archive SHA-256 is recorded in `archive-verification.json`. No large raw PCM
or full report JSON is transferred or parsed. No third attempt, longer window,
relaxed deadline, different buffer or unchanged soak follows this failure.

## Remaining Scope

Independent original-requirement audits find no demonstrated missing mandatory
S1-S3 software implementation to invent as a parallel task. Existing closed VAD
content reconciliation needs no additional gain/codec repair or unchanged fit.
This does not mean only physical calibration remains: successful closed-prefix
native acceptance, lower-setting native quality and matched route/clock evidence
remain open, in addition to sustained/physical qualification. The current Stop
confirmation failure must be localized with bounded timing/state evidence before
another acquisition; it is not sufficient reason to change production timing.
Remote tests may run beside independent development when their resources and
owners are isolated, but repeated identical failed tests are not progress.
