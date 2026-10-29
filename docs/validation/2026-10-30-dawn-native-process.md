# DAWN native Process desktop follow-up

Host date: 2026-10-30, Asia/Shanghai. Physical target:
`LAPTOP-NP49RG6H` / `192.168.12.97`. The target's unchanged clock reports
2026-10-25; no cross-machine wall-clock subtraction is used.

This receipt continues the native desktop controls audit. It is not a
waveform-continuity, physical latency or S1-S3 completion certificate.

## Authorized output and isolation

The user identifies the newly connected DAWN as a headphone amplifier with
headphones attached and authorizes its volume adjustment. No other speaker
volume or mute state may be changed. The pinned active render endpoint is:

`{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`

Catalog identity is `DAWN PRO2`, 48 kHz, stereo Float32, volume scalar 1,
mute false. Its observed default period is 100,000 units of 100 ns and its
minimum reported period is 30,000 units. Those properties are not an
end-to-end delay measurement. Realtek remains volume 0 / muted.

The test makes zero endpoint-volume, endpoint-mute or default-role writes.
Before/after catalogs compare all active render endpoint volume/mute states
and every default role. DAWN was already the default render endpoint in the
initial catalog; the harness does not assign it as default.

Each hidden production desktop instance has isolated APPDATA, a preloaded
explicit `Process Loopback -> Bus -> DAWN` project, and a pinned owned source
PID plus creation timestamp. The source also renders explicitly to DAWN;
it does not use a virtual endpoint whose external mixer routing is unknown.
Its changing markers have amplitude 0.025. Both direct and processed paths
can therefore reach DAWN. This is not an independent dual-channel reference
capture or a physical latency A/B comparison.

The project runs 48 kHz / F32 / block 256 with no extra DSP nodes and unity
Bus/send gains. Native observations confirm Shared Float32 stereo output at
48 kHz and an actual 1056-frame device buffer. That buffer is distinct from
the Process input target and does not establish physical delay.

## Original production desktop evidence

App source `e195bfe`, SHA-256
`09A0053326179928FB20E6F90DD1472501EF4A7F2D10907BEE0E9C550566F93A`,
and probe `f72e03c` remain unchanged during the first three attempts.

- `desktop-dawn-process1` restores the isolated project successfully but
  waits for Stop without starting it. A restored project is initially idle,
  unlike the earlier ordinary default-project startup. This is a harness
  failure, not failed audio preparation.
- `desktop-dawn-process2` explicitly invokes Start. DAWN opens and owner
  generation 1 starts. Its global source-label lookup finds both the graph
  sidebar and settings selector, so the harness refuses the ambiguous match.
- `desktop-dawn-process3` scopes settings lookups to the modal. It explicitly
  selects capacity 16384, target 4096, jitter 96, and invokes the real measured
  Apply. Logs record generation 1 cleanup/reaping followed by generation 2
  starting on the same DAWN endpoint. This is native owner-replacement
  evidence, not merely an Invoke return.

The last candidate is diagnostic, not a proposed low-latency preset. Its
target must not be called a measured physical delay. Native requested/active
confirmation is still unavailable in the original app: both the before and
after UIA trees omit the status/summary text. Save has no InvokePattern in
the retained post-Apply tree. The helper then fails and falls back to bounded
shutdown, ultimately forcing this owned app process to exit. GPUI logs retain
`0x80040102` / `0x80070578` window errors. That run does not establish normal
Quit or native confirmation. The source stops/joins with exit 0. All output
volume/mute states and default roles remain unchanged.

## Accessibility repair

Commit `60d4864` adds `Role::Label` to measured feedback and the shared
requested/active summary builder. Their existing IDs and exact labels remain
unchanged. It does not change settings, admission, buffers, DSP, native owner
replacement, or default selection.

The first root cause is the missing role: the two original native trees contain none
of these labels, and GPUI's accessibility implementation explicitly omits
nodes without a role even when an ID/aria label is supplied. The existing
service/UI tests cover requested versus active confirmation separately; a
fresh production native tree must verify this repair instead of treating
software fixture success as native evidence.

The attempted repair validation uses the shared `D:\MoirenBuild\target` and
existing Cargo home. Its measured/UI release-test build hits the explicit
30-minute watchdog while still compiling dependencies; the retained result
is TimedOut=true, not a test or compilation PASS. The release application
stage never starts. No new application is deployed and native run 4 is not
launched. The dependent, bounded follow-up task records a build failure and
exits without deployment. Caches are retained.

Before resuming the same source/profile with that cache, the exclusive-build
guard detects a different native Cargo build, PID 20952, with parent 15640:
`cargo build --release -p moiren-app --locked`. Its compiler commands target
`D:\moiren\target\release` and the normal user Cargo home, not this task's
`D:\MoirenBuild\target`. Its ownership is not established. The resume launch
refuses before creating its scheduled task; the external build is not stopped,
and no new acquisition runs against it. At that checkpoint the accessibility
repair is committed but not compiled/native-verified. Local formatting and
diff-whitespace checks pass. No driver reinstall or cache clean occurs.

## Main integration and native RED/GREEN

After fresh process audits confirm the unrelated builds have ended, merge
`origin/main` at `c0bd006` (PR39's library-card update) into this branch as
`ca94f98`. The remote task worktree, not the user's `D:\moiren`, builds against
the retained shared cache. The measured/UI release suite passes 68 tests and
the release app builds. An initial deployment guard refuses a still-present
build process; a later idle audit permits deployment without another build.

`desktop-dawn-process4` on `ca94f98` is the native RED test. The role repair
makes three UIA Text nodes appear, but their names are empty. The real Apply
still replaces the owner and starts generation 2; exact native confirmation
fails, and the bounded helper eventually forcibly terminates its owned app.
This failure remains retained, not relabeled as a successful control test.

Inspection of the pinned AccessKit consumer/Windows dependency sources shows
that `Role::Label` obtains its UIA name from its value, not its aria label.
Commit `3ac8484` adds matching `.aria_value(...)` to the same two existing
builders. It does not change any audio setting, model admission or DSP logic.
The same 68 release measured/UI tests pass again, with no failed tests and
503 filtered out; the measured test stage takes 174.557 seconds including
compilation. The release app stage completes in 113.076 seconds. No cache
clean or driver reinstall is performed.

The deployed `bin\moiren-app.exe` is now source
`3ac84845e40a508daeb4791caed116abc00adfb0`, 33,404,928 bytes, SHA-256:

`7F4EA0148D09DFCFBC316A683A54ACC2C6B5EC4F70FB3B9B296DE63BBB211513`

`app-manifest-dawn-value.json` records the deployment; the preceding `ca94f98`
and original `e195bfe` app copies remain available for rollback. Probe source
`f72e03c` and the installed Running driver remain unchanged. Their SHA-256s are
`3CEF3ABFFE43E6C8681176ED9628B840B87973B22D8542800835C5954E5997A4`
and `CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.

`desktop-dawn-process5` is the native GREEN test on that exact app:

- The real desktop selects the pinned Process source, capacity16384, target4096
  and jitter96, then invokes Apply measured budget and restart.
- Native logs confirm generation1 cleanup/reaping and generation2 running
  on the same pinned DAWN endpoint.
- Requested and active UIA values match exactly: NodeId(0), PID24544, creation
  time134374099583107635, the chosen three budget values, 48kHz, originating
  session1 / owner1 / revision2, selection1, and Disabled host-ingress limit.
- Feedback reads `Measured budget is active; physical latency remains
  unqualified.` The helper records Exact=true and Completed=true.
- Close settings, Stop, generation2 cleanup/reaping, Diagnostics, two targeted
  wheel scrolls and Quit all complete. App and source exit0, without forced
  termination. Every render volume/mute and default role remains unchanged.

This closes the baseline S3 production-desktop Apply/exact-confirmation gap.
The diagnostic4096-frame target is not a low-latency default or measured delay.
Hidden-window UIA evidence is not visual acceptance, nor does this run acquire
continuous PCM or a physical reference.

## VAD to DAWN diagnostic baseline

`dawn-vad-baseline1` uses exactly VAD playback -> unity Bus -> pinned DAWN at
48kHz / F32 / block256. The VAD source is Exclusive, with a default requested
period and an observed480-frame native buffer; DAWN is Shared with actual
period480 / capacity1056. Compatible input target2544 / capacity4800 is
reported separately. A changing marker schedule, amplitude0.025, submits all
1,440,000 source frames. This bounded30-second diagnostic soak retains six
observations, **no acquired PCM and no independent physical reference**.

The observed duration is30.0104193seconds. Native error/failure fields are
empty, source Stop/join succeeds, Engine closes Dormant with no cleanup error,
and VAD removal reaches Removed. Before removal, fresh provider telemetry has
2999 completed packets, max packet480, max completion interval107351 units of
100ns, zero dropped/silent/stale packets, missing sequence slots, recovery or
epoch resets. Removal adds one epoch reset to its separate cleanup snapshot;
it is not erased or counted as an in-run reset.

Joined backend evidence records zero native dropped/silent render frames and
1,442,016 submitted render frames. Playback has zero xruns/short reads/recovery
prime frames, initial prime complete and32 initial-prime muted frames, retained
as startup rather than hidden. The driver-transfer service delay maximum3235
units of100ns is a local service diagnostic, not physical route latency.
Neither zero telemetry faults nor submitted frames prove audible continuity.
Exit1 is the expected unqualified result: report status is `unqualified`, its
distribution is null and observed measurement frames are0. The generic Windows
speaker name still differs from the requested diagnostic name; this run is not
a successful FriendlyName/rename qualification.

## Remaining boundaries

The follow-up `dawn-vad-budget1` probes exact native requests before any
default change. Shared128 is explicitly refused before source Start: the
DAWN Shared engine reports the requested128 outside its336..480-frame range.
No fallback is applied, and this is not a failed audio-continuity run.

Exclusive256 is accepted exactly, with a256-frame native buffer/period and
block128. It runs30.0001756seconds and closes/removes normally, but retains
24 missing provider sequence slots across5 gap observations, a maximum
completion interval2097159 units of100ns,2 playback xruns / short-read
callbacks,32 recovery-prime muted frames, and an incomplete source schedule
(1,430,400 /1,440,000 acknowledged frames). No PCM is acquired. This is not
a stable candidate, and a small accepted device period is not physical E2E.

During that acquisition a separate `D:\moiren\meas-delivery\moiren-app.exe`
starts in the interactive session. It was absent at the initial exclusive
guard and is not owned by this task. The run is therefore not an isolated
matched-condition result; the faults cannot be attributed to the period,
driver or competing app from this evidence alone. The helper refuses before
its third, shared-target576 / input-target1792 candidate, instead of stopping
the other application or testing against it. No build is started. All output
volume/mute states and default roles remain unchanged. Further isolated
acquisition waits for a coordinated idle window; no competing app is killed
or reconfigured.

`dawn-vad-budget1.zip` is47,518bytes, SHA-256
`6F46B9D0695076808436AB4932E167035D1B867E1975934919CAD5C756BC923A`.
All20 files /21 entries are downloaded and individually verified by exact
file set, length and SHA-256. Failed/refused attempts remain retained.

### Isolated budget follow-up

`dawn-vad-budget2` starts after an idle audit and polls foreign
Cargo/rustc/link/app/probe processes once per second. Neither case observes
one. This scoped observation does not exclude other system activity, but
supersedes the competing-app limitation for these two runs. There is no
compilation during acquisition and no volume, mute or default-role write.
The source is a 30-second, 1,440,000-frame marker schedule at amplitude0.025;
the diagnostic continues with silence to the requested35 seconds.

| Case | Actual native periods and input policy | Retained failures |
| --- | --- | --- |
| Shared target576 / input1792 | DAWN period480, capacity1056, render target576; VAD source480; Engine block128; input capacity4800, target1792, jitter96, low930, modeled minimum1602; whole demand896, transfer898, FIR32 | Actual duration34.9715725seconds; 15 playback xruns / short-read callbacks; 224 recovery-prime muted frames; 72 missing native sequence slots across23 gap observations; maximum completion interval119.735ms |
| Exclusive256 / minimum VAD source / input1024 | DAWN period/capacity256; VAD source144; Engine block128; input capacity4800, target1024, jitter96, low610, modeled minimum898; whole demand576, transfer578, FIR32 | Actual duration34.9999107seconds; 8 playback xruns / short-read callbacks; 96 recovery-prime muted frames; 56 missing native sequence slots across14 gap observations; maximum completion interval47.7157ms |

Both sources acknowledge all scheduled frames, which means successful
ReleaseBuffer submission, not device playback or captured continuity.
Both retain32 initial prime-muted frames separately from recovery silence.
Driver READ-drop, provider dropped/stale/silent packet, epoch-reset and
in-run recovery counters are zero. Native sequence holes are separate
evidence and cannot be dismissed by those zero counters or interpreted
as proven loss of nonsilent PCM. No PCM is acquired, the distribution is
null, and both exits remain1/unqualified. The shorter first duration is
retained exactly and is not a full35-second pass. Both owners stop and
reach Removed/Dormant/Closed normally.

Neither smaller candidate is stable. Do not promote a smaller default or
expand a queue to hide these faults. Investigate source submission, native
ready-buffer service, provider completion and actual graph-output waveform
before assigning a cause.

`dawn-vad-budget2.zip` is76,022bytes, SHA-256
`9CD32A8877C8A1AD0CA7837D3D329E76956380D6972EA07BD76C8B67BD48508F`.
All20 files /21 entries are downloaded and verified by exact file set,
length and SHA-256. Its exact terminal scheduled task is unregistered after
checking its action and child PIDs; evidence, deployment and shared Cargo
caches are retained.

DAWN resolves the lack of an authorized physical render endpoint; previous
muted-Realtek limitations describe historical runs, not current routing.
Independent common-clock reference capture and calibration remain absent.
No physical delay distribution, VAD/Process latency winner, smaller default,
continuous-waveform pass or stable sub-10-ms result follows from this work.

## Checkpoint retention

All three completed attempt directories and the watchdog build logs are
downloaded under `target/physical-1921681297-20261029`. The 67-file host
inventory is `dawn-checkpoint-host-manifest.json`, SHA-256
`FC0EE820B04E7408308C082602DC2E891CD315AE90630EDFC1D3AC1DD2C4A192`.
It is a local evidence inventory, not an independent authenticated recording.
Remote originals remain retained; no evidence is pruned without a verified
archive. The original planned archive was not produced at that checkpoint
because run4 had not started.

The post-audit receipt confirms all five owned terminal scheduled tasks are
unregistered, no app/probe audio process remains, and the deployed app still
matches the original `e195bfe` hash. The unrelated Cargo build remains running
and untouched. Shared caches are retained; D has 268,809,793,536 bytes free
at that snapshot. C free space is 6,239,395,840 bytes; this task's configured
Cargo home, target and temporary paths remain on D.

The superseding `dawn-native-process-final.zip` is214,755bytes, SHA-256
`70FD9014525E5066C30B267ABF7F0DE676B92B39898175F27970032ACE06EAEF`.
All156 files /157 entries including the manifest are verified by exact file
set, length and SHA-256 after download. It retains attempts1-5, both successful
builds, original timeout, deployment/rollback manifest and VAD diagnostic run.
The archive state confirms clean task source3ac8484, Running driver and matching
deployed hashes. The user's separate main has advanced independently and has
an untracked `meas-delivery/`; neither is modified or cleaned by this task.
Shared cache3,370,089,867bytes is retained; D has263,376,846,848bytes free in
that snapshot. Historical zero-target/clean-main statements are not current.
