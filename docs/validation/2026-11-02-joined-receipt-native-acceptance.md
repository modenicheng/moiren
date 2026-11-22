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

## Start/Stop Timing Investigation

A fresh read-only poll at23:28:20 +08:00 confirms the2320task is terminal
Ready/result1 with no app/source/supervisor owner. Its files remain immutable.
Source inspection confirms an asynchronous action boundary: AccessKit's Invoke
becomes Click, and the actual gpui0.3.9 callback sends the action to an unbounded
queue; a foreground task later invokes the application's accessibility handler.
Returning from Invoke therefore does not acknowledge Moiren Start consumption,
native owner creation or a new accessibility-tree state.

No fixed two-second completion wait is found in that inspected path. The
roughly2.27-second policy-action record spacing includes saves, a200ms helper
sleep and subsequent UIA queries; it is not a measurement of Invoke duration.
The visible UI's33ms poll and service's at-most10ms active park are scheduling
settings, not proven response-time bounds. The stable transport ID likewise
does not prove cached UIA reads are current. These source observations narrow
the questions without establishing the preceding deadline failure's cause.

A separate2340evidence package is prepared to retain bounded in-memory phase
and state observations, including failure paths. Its timing origin must remain
before Start Invoke, not move to Invoke return;400ms/900ms and all native
admission/coverage gates remain unchanged. Phase entry/return, Stop action
entry, actual Stop Invoke entry/return and post-join confirmation are distinct
facts. No observation alone supplies the absent native acceptance result.

The2340runner now attaches its diagnostic object before source activation.
Fixed Start/Stop action and actual Invoke enter/return/error slots survive
ordinary exceptions; first/last state and at most eight transitions remain
bounded, with an omitted count and independently reserved phase fields. The
existing stopwatch supplies every hot timestamp, and serialization happens in
the enclosing `finally` only after the hot action returns or throws. A UIA call
that never returns can still prevent worker persistence; the held supervisor's
outer limit remains the fallback, not fabricated missing timestamps.

Stop-action entry is explicitly not a successful Stop Invoke or joined owner.
The old `stop_called` field remains a labelled legacy alias of action entry,
not a success flag. Unread native `IsEnabled` is null rather than false, and the
previous never-updated `audio_started=false` is replaced by unknown until actual
native confirmation. Ordinary non-hot actions separately bracket only their
Invoke call; save/query/sleep overhead is outside that duration.

The missing-diagnostic assertion fails before implementation. Eight focused
checks then pass. Independent review finds an added `IsEnabled` read on every
Start poll; a new getter-count regression fails before correction. The final
helper preserves the original one-Name read and only reads enabled for exact
Stop, reusing those values for control and diagnostics. Nine focused and17prior
runner checks pass on Windows PowerShell5.1; the earlier static root-mismatch
failures remain retained rather than reported as product failures. A second
read-only review finds no remaining blocker. The app, draft, policy,400/900ms
timing, STA execution and150s/240s lifecycle limits remain unchanged.

### Actual STA Timing Receipt

One instrumented2340task launches at23:41:20 +08:00. Its retained hot receipt
records STA, Start Invoke entry5ms and return2023ms on the same stopwatch:
2018ms is spent inside the Invoke call, before Stop can be attempted. The Stop
action enters at2026ms, observes one `Start` state at2034ms with enabled unknown
(correctly unread), and fails the unchanged900ms deadline. Final elapsed is
2052ms; actual Stop Invoke entry/return remain null. This is not a completed
400ms audio window, nor proof that a Stop command was sent.

The ten ordinary Invoke calls independently take2004-2330ms each, excluding
their surrounding save/query/sleep work. This turns the previous two-second
Invoke hypothesis into an actual measured client-call observation for this run;
it still does not identify COM/apartment, provider or scheduling causality.
Native generation1 starts at23:41:55.632 after failure cleanup begins at55.589.
The app requires forced cleanup and exits-1; source exits0 with its complete
submission. Task/worker/supervisor are terminal with no owners left, unchanged
protected processes/defaults/render controls/driver and no cleanup error. All18
compact artifacts are individually hash/length verified after download. No
Evaluate/Apply, normal joined history or numeric receipt audit is obtained.

### Single-Variable MTA Comparison

The next distinct hypothesis changes only the evidence client's apartment:
`-Mta` is passed before `-File` to the child PowerShell worker, which verifies
its actual apartment before native work. The C# supervisor's STA attribute is
not changed. Source, app hash, audio settings, policies, hot timing origin and
diagnostic code stay identical. A fresh2350directory preserves both earlier
failures; this is not an unchanged retry or a relaxed deadline.

Five source-invariance checks follow a genuine missing-MTA assertion RED;
nine diagnostic and17existing runner checks also pass. The recompiled supervisor
is not executed on the development host. Independent review finds no safety
blocker to one bounded comparison, without claiming that MTA must solve the
delay or that a single changed outcome proves STA caused it. The remote binary
is copied locally on the target and its hash remains unchanged.

The MTA task launches once at23:45:59 +08:00 and confirms actual MTA. Start
Invoke enters at5ms and returns at2013ms:2008ms remains inside the call. Stop
action enters2015ms; its sole state is still Start at2021ms with enabled unknown.
Final elapsed is2041ms, actual Stop Invoke is never entered, and the unchanged
deadline fails. The ten ordinary calls take2005-2019ms. Merely switching this
worker to MTA does not remove the measured delay; STA alone is not a sufficient
explanation for the preceding result. The causal layer remains unlocalized.

Generation1 starts at23:46:33.974 after cleanup begins at33.907. App exits-1
after bounded forced cleanup; source submits completely and exits0. Task,
worker and supervisor finish with result1 and no owners. Defaults, render
volume/mute, driver and protected OBS/QQMusic identities remain unchanged;
cleanup errors are empty. Another18compact artifacts are length/hash verified
after a124,947-byte archive download. No normal join, evaluated candidate,
Apply, actual receipt audit, waveform or physical qualification is claimed.

The host's reviewed `timing-comparison.png` and companion CSV retain all22
actual Invoke durations, separate STA/MTA and mark the unchanged900ms hot gate.
They explicitly describe client-call time, not audio latency. Both original
result-file hashes and generated artifact hashes are in
`s3-mta-timing-20261102-2350\timing-comparison.json`. Neither archive contains
large PCM/report transfers; the app is not rebuilt just to alter the runner.

No additional audio attempt follows this failed single-variable hypothesis.
The next diagnostic should isolate managed-client dispatch from provider and
foreground consumption on a stopped UI-only surface before spending another
short native-prefix attempt. Two-second Invoke does not justify extending the
capture window past measured-history coverage or changing production timers.
The original S1-S3 objective remains active; independent development remains
permitted while target tests are properly isolated.

### Stopped-UI Comparison Refused Before Measurement

The distinct inner/outer diagnostic uses evidence root
`s3-uia-inner-20261102-2357` and remote root
`D:\MoirenTest\20261102-uia-inner-2357`. Its task actually launches once at
**2026-11-03 00:02:59 +08:00**; the directory's previous-day preparation label
is not the execution date. The precompiled C# helper brackets only its
`InvokePattern.Invoke()` call, with caller thread/apartment and explicit error/
entry/return fields. PowerShell separately brackets the outer call. Four fixed
Settings/Close cycles use PowerShell/C#/C#/PowerShell, with state confirmation
outside timing, rather than audio Start/Stop or Evaluate/Apply.

Independent review catches an enabled-Save assumption before launch: untouched
Settings deliberately disables Save. A genuine regression RED is retained;
the corrected worker checks owned presence instead, and awaits asynchronous
Settings closure outside timing. Eight source checks and six helper checks pass
on Windows PowerShell5.1. The supervisor is explicitly narrowed to one held app
owner; the source process is removed rather than reusing its two-owner contract.
The unchanged app is copied within the target, with its original hash; isolated
APPDATA/TEMP and a Limited interactive task do not modify user preferences.

The actual attempt does **not** remain stopped. Its new preferences set
`restore_last_project=false`, which enables the product's existing automatic
catalog/default-session startup. Logs record DAWN output and Steam microphone
opening, then generation1 starting at00:03:02.049. The runner times out waiting
for enabled Start before entering any measured action. Therefore the intended
no-audio boundary was violated by the runner's incorrect startup assumption:
zero Invoke samples, `no_audio_owner_log=false`, `completed=false` and forced
app exit-1 remain explicit. No inner/outer timing conclusion, new audio-quality
claim or successful normal Quit follows. No second attempt is launched.

Source review localizes the contract to `service/mod.rs::start_service` and
`managed/driver.rs`'s Catalog completion: absence of a restored project enables
automatic startup. The existing
`automatic_startup_uses_catalog_defaults_once_and_stop_stays_authoritative`
regression covers that behavior; zero default gain does not prevent device open.
The already-used stopped startup contract is a valid graph-bearing restored
project with `session:null`, not a missing project or a null session without a
graph. This finding requires an evidence-runner startup guard, not a speculative
production change. The acquired failed package stays immutable.

The worker finishes at00:03:17; task/supervisor end1, with no app/source owners,
unchanged defaults/render volume/mute/driver and protected OBS/QQMusic identities.
Eleven compact files are individually length/SHA-256 verified after a9,332-byte
download. A separate EncodedCommand inspection stalls and grows to4,990,066,688
bytes working set; its shell observation is interrupted, then exact remote
PID26064, full creation FILETIME, executable and decoded command hash are
rechecked before held termination at00:09:06. This is an owned diagnostic-reader
failure, not a user process or audio failure. The initial precision-mismatch
cleanup refusal and an archive-script parser failure are retained; neither
changes a process. Subsequent inspection uses bounded file-based scripts and
copies compact logs instead of serializing the larger inspection object.

This closes the current diagnostic increment without extending the900ms gate,
changing production timers/defaults, reopening first-packet work or rerunning
the same acquisition. Native closed-prefix success, lower-setting quality and
physical qualification remain open; an automation limitation is not a newly
demonstrated severe stability defect. Independent product work may proceed
without waiting for unavailable physical recording hardware.
