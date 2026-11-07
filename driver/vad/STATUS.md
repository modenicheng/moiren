# Moiren VAD driver status

This file is the canonical owner of VAD run and gate status; the other
`docs/vad-*.md` records are dated evidence logs and link here for the current
verdict. Status updated 2026-11-07.

## Direct remote target amendment - 2026-11-07

The user now selects the existing separate remote computer directly. VMware
startup/unlock is no longer a driver-work prerequisite; older VM-only planning
statements below are historical. Retain package/signature/SYS pins, rollback,
endpoint and protected-client guards, with disruptive install/reboot/security
steps handled explicitly. Development-host driver/audio/UI operation remains
outside scope. An accessible unlocked desktop is needed only for ordinary
visible-input acceptance, not as a blanket gate for SSH, CLI, metadata or
noninteractive audio work. Those still require their actual token/session/
device prerequisites. The hit snapshot is not a WTS lock-state verdict.

## Remote-priority revalidation - 2026-11-07

The [removal-stage diagnostic slice](../../docs/validation/2026-11-07-vad-removal-stage-diagnostics.md)
adds bounded exact-runtime Remove/ReleaseHardware/Cleanup/Destroy-entry evidence
and passive QUERY-only WPP, without changing ABI, LastStatus, ownership or the
direct-Removed gate. First/last/count and sticky failure remain explicit; slot
reuse reports lost pending evidence rather than erasing it silently. New
production-extracted RED/GREEN cases,21MSVC suites,16Windows/GCC portable suites,
WDK Analyze/InfVerif and matching TMF generation pass. No Linux/native pass or
runtime21 root cause is established. Direct remote612 remains unchanged; a
guarded diagnostic deployment/acquisition is still required.

The later [paired-waveform cleanup receipt](../../docs/validation/2026-11-07-matched-waveform-cleanup.md)
uses the established graceful OBS correction and a regression-tested refusal
of non-null VAD native errors. Both OBS instances now close normally. Process
source/graph/reference Stop and join complete; VAD source Stop/worker join and
Closed Engine cleanup also complete, but runtime21 remains Removing/Closed
until the unchanged20s cleanup deadline. The actual pair fails, Ready1, with
no owners and successful before/after safety audits. A later absent PnP child
is not direct Removed evidence. Source review identifies a ReleaseHardware
status-observability gap, not a proven root cause or repaired product defect.
No unchanged native retry, timeout extension, reset or identity rotation follows.
Canonical v3/v4 analysis resolves all six Process/VAD reference codes and
Process return0,2,3,4,5; code1 section matching remains unresolved. V4 locates
the closed after-error VAD MKV but refuses its30.700s duration at the unchanged
30s short-run cap. All28canonical and4new synthetic checks pass remotely;
input hashes and166archive members verify, as do the original29draft/cache
pins. Full S0-S5 and matched/physical qualification remain open.

The later [waveform checkpoint](../../docs/validation/2026-11-07-matched-waveform-checkpoint.md)
uses clean `b08f61b` releases and the unchanged612 driver directly on the
standalone target. Actual Process source/reference/graph data and closed OBS
media are retained, but forced OBS shutdown makes the pair fail before VAD.
The already documented close-order correction is recovered with15passing
pure checks and no further acquisition. Canonical v3 analysis resolves all
six reference codes and return codes2,3,5, preserving partial/quality failure.
Its28synthetic checks and actual offline task pass; input hashes, both full
native safety audits, all100compact archive members and original29draft/cache
pins verify. Neither a VAD pass nor matched/physical qualification follows.

### Retained history and endpoint-store consumption diverge

The [history/consumption receipt](../../docs/validation/2026-11-07-endpoint-history-and-consumption.md)
records two guarded read-only native queries at21:26 and21:38 on the standalone
remote target. Typed raw PDO dates distinguish older fixed fixtures from seeded
controls, without proving MMDevice creation or disablement origin. Exact COM
GetDevice succeeds for all six retained endpoint IDs, including fixed Playback
whose SWD node is absent from a separate102-entry PnP inventory. All are
NOTPRESENT after cleanup; that does not change the preceding live DISABLED
verdict. Fixed Playback's endpoint property store lacks EnableEndpointByDefault
(S_OK/VT_EMPTY), whereas both seeded Playback stores retain0x101 despite
identical currently staged interface policy. All retain OUTPUT_UNDEFINED,
UnknownFormFactor and NeverSetAsDefaultEndpoint0x307. This proves a retained
property divergence, not its cause or permission to reset/enable a device.
Endpoint adapter labels and final names remain composed, unlike the exact
names independently read from KS interface properties. No name gate is relaxed.
Both13-file terminal archives independently verify length/SHA; driver/boot,
defaults/render/security/protected identity and idle audits pass. No audio
Start, installation, endpoint enablement/rotation or desktop input occurs.

Published0fdc2c6 completes exact-head CI42374879801 with20/20 jobs successful;
original watcher99946 exits0. This covers the product-state slice, not native
or physical qualification. S4 additionally needs reconciliation of the current
name-derived KS reference with Windows endpoint identity/settings preservation;
new endpoint IDs must not be treated as a harmless naming refresh. Full S0-S5
remains open and PR26 stays Draft/open, not a wholesale merge candidate.

### Exact interface policy and product Windows-state visibility

The [policy/state receipt](../../docs/validation/2026-11-07-endpoint-state-and-policy.md)
records read-only observations on the standalone remote target at20:15..20:20.
All18 exact persisted Audio/flow/Realtime registrations resolve and open.
Fixed and earlier seeded Playback both stage EnableEndpointByDefault0x101
and OUTPUT_UNDEFINED; all directions retain NeverSetAsDefaultEndpoint0x307.
The staged interface/EP0 names match their recorded requested names.
This rules out currently missing staged enablement, not when Windows consumed
it, disablement origin or persisted-state migration. Four exact phantom-allowed
PDO reads report no FriendlyName value, a fixed Models DeviceDesc and the
latest requested BusReportedDeviceDesc. These removed-device observations
do not prove live publication timing or a fix for the composed MMDevice suffix.
Helper failures/corrections are retained; all61 terminal files independently
verify remote/local/archive lengths and hashes. No enablement, identity
rotation, native client, install/restart, default/volume or security write occurs.

The product now carries the selected MMDevice's raw GetState flags and an
independent state-read error through provider/service/probe to Exports and
diagnostics. UNKNOWN, DISABLED, NOTPRESENT, UNPLUGGED and unexpected raw bits
remain explicit; ACTIVE is labelled as not proof of flow. Successful raw name
readback survives a separate state-read failure. Cache, exact identity,
Windows-name acceptance, lifecycle and client-admission rules are unchanged.
The pinned17-file source passes2636 Windows workspace/headless tests with
10 ignored,155 Windows example tests,133 Linux example tests,17 Linux observer
tests,618 Linux app tests with2 ignored,7 Linux headless acceptance tests and
strict Windows/Linux workspace Clippy. Clean released004809 then runs once
directly on the remote target at20:52:50..20:53:14. Product state observations
change from initial NOTPRESENT for both exact identities to Microphone ACTIVE
and Playback DISABLED during the idle hold; independent failure enumeration
agrees on both exact MMDevice IDs. Raw name/state reads succeed, demonstrating
the new propagation, not a cause or audio pass. The run fails before probe
Start with zero acquired signal frames. Join/release/cleanup and external
state/idle audits pass; report acceptance fails. All17 terminal archive files
verify lengths/hashes. No enablement/identity rotation/unchanged retry follows.
Published6401849's own CI completes20/20 successfully; the later0fdc2c6
publication also completes20/20 as recorded above. Full S0-S5 remains open.

### Later PCM failure snapshot: exact Playback is disabled

The [enumeration receipt](../../docs/validation/2026-11-07-rename-enumeration-evidence.md)
now retains the independent fixed-fixture PCM refusals at19:34 and19:43.
The second uses clean8318308 with failure-only both-flow metadata; software
tests pass154 on Windows/132 on Linux and strict Clippy passes. Native ALL-state
collection completes97 Playback/53 Microphone rows without truncation or a
collection error. Each direction has one exact root/stable-ID/flow match and
matches its live provider checkpoint. Playback is state2 DISABLED; Microphone
is state1 ACTIVE. Playback already reports OUTPUT_UNDEFINED in its connected
topology, so category alone does not explain the difference from the earlier
seeded fixture. The disabled exact row cannot enter ACTIVE-only admission at
this failure snapshot. Disablement origin, state throughout the polling window,
creation-policy consumption and persisted-state migration remain unproven.

Both PCM reports fail before probe Start, retain zero acquired samples and do
not verify PCM. Engine timeline advancement during the idle hold is observed;
Starts0 is not a no-native-activity claim. Cleanup/join/release and external
state/idle audits pass, but report acceptance fails. Both17-entry terminal
archives independently verify lengths/hashes. No endpoint is enabled, identity
rotated, historical device deleted or gate relaxed. Next discriminate exact
interface EP0 staging from persisted endpoint policy with read-only evidence.
This is not a lock-screen diagnosis or S1/S4/S5 completion. Prior published
c890e50 CI run42367038940 finishes all20 jobs successfully at19:45; later
diagnostic commits still need their own exact-head CI.

### Later held-rename observation and failure-only enumeration metadata

The [enumeration receipt](../../docs/validation/2026-11-07-rename-enumeration-evidence.md)
records a reviewed failure-only ALL-state metadata addition, without changing
ACTIVE client admission or exact-name/lifecycle gates. Software suites pass
152 on Windows/130 on Linux with strict Clippy. A clean9e5a5e3 release runs
at19:29 on the same explicit fixture and unchanged612 driver: both held and
replacement clients initialize/release, direct old Removed and fresh Engine
confirmation succeed, but both raw Windows names retain adapter suffixes.
The run is failed/unqualified. Enumeration does not fail this time, so neither
failure-time native collector nor a timeout fix is proven. Starts0/no PCM;
join/release and external-state/idle audits pass. Full S0-S5 remains open.

### Later direct remote 612 deployment and native refusals

The [direct remote receipt](../../docs/validation/2026-11-07-direct-remote-category.md)
supersedes the unchanged611 deployment observation below. At18:17 onNovember7,
the reviewed finite installer adds the category/default-build candidate and
restarts only `ROOT\MEDIA\0001`, with both helpers exit0. The target reports
`oem206.inf`, version3.1.1.612, Running/PnP OK/problem0 and service-file SYS SHA
`B25CA2E8DB8DD3E98D9B90AA6CF19822C879E6CA13D52B35D160CEF2056D1737`.
The exact611 opt-in diagnostic package is preserved and independently verified
on target/host; no rollback is executed. There is no reboot, security/default/
volume write, host installation or protected-client manipulation.

Source8761d07's frozen native Stage1 passes Apply/Stop/join. Streaming fails
before any Start or rename: fixed Playback identity/name is not found among
ACTIVE candidates despite an ALL-state old-name observation. A separately
identified explicit fresh seeded held-client diagnostic initializes Playback
and observes its requested base name plus adapter suffix, but then fails Stop
with a sticky no-ready-native-packet-stream worker error and nonempty public
exit cleanup_error. Neither native refusal is renamed into a pass, and no
PCM, exact-name, removal/replacement or continuous-audio acceptance follows.
No test owner or present child remains after either bounded operation.

Source investigation finds that initial Paused/zero-packet allocation can
prematurely prepare the first Playback owner. The focused source fix passes
the final workspace/headless tests,145 opt-in integration example tests and
strict workspace/all-target Clippy at18:40. The startup exception is limited
to the first Paused/zero-frame observation with all recovery epochs zero.
This does not establish this run's unique cause or completion of S4/S5.

The clean corrected release from4d9195a is tested at19:01 on the same explicit
seeded fixture and unchanged612 driver. Both held clients now initialize and
release; both renames retain direct old-Removed evidence. Playback replacement
MMDevice enumeration still times out. Microphone replacement initialization,
release and fresh Engine confirmation succeed, but exact Windows FriendlyName
still fails on the adapter suffix. The run remains failed/unqualified, with no
Start/PCM claim. Its cleanup_error is null, join/control release succeed and
the later wrapper confirms no test owner/present child and unchanged external
state. This is progress beyond the earlier failure, not full acceptance or a
proof of its unique cause. The requested-name suffix and full native matrix
remain open, as do physical and sustained gates.

The [remote-priority receipt](../../docs/validation/2026-11-07-remote-priority.md)
records current target evidence, not new audio or driver qualification. The
physical target remains reachable with the unchanged Running SYS hash. A
fresh desktop observer sees PowerShell rather than the historical lock-screen
foreground, but a subsequent stopped-only control refuses strict owned input
before posting any click. Its bounded cleanup leaves no test owner or native
audio start; task result1 and forced test-app exit remain failures. Defaults,
render controls and protected process identities are unchanged. Historical
OBS12648 is now absent, adding a frozen metadata-reader guard failure; SSH
still has an admin token. The existing encrypted driver VM remains stopped,
and no new package installation, host native operation, unchanged retry,
Windows-name acceptance or physical-latency verdict occurs.

A separate local-only retained desktop runner correction follows the existing
service contract: predecessor1 -> stopped intent2 -> replacement owner3, with
the measured request still bound to predecessor1. The fresh evidence copy
changes five expectation sites, passes34 pure checks on each PowerShell
version and1software-backed Rust service test covering F32/F64. Historical
pins/contracts are unchanged; no corrected runner deployment or native run
occurs. The final17:31:38 read-only target check still has no owned test
processes or running owned tasks, unchanged protected identities and a Running
driver; the host VMware inventory is zero. S3 and all other native/physical
gates remain open.

The same receipt's later one-shot stopped-only discriminator records the
actual Settings-point cover at17:42:07+08:00: foreign PID13100/HWND855132,
`LockScreenBackstopFrame`, not the owned Moiren root. It sends no diagnostic
input or audio. Owned Close fails to end the app within10s, so held forced
cleanup and task1 remain lifecycle failures. All13 raw artifacts are verified;
defaults, render controls, protected identities and Running driver hash are
unchanged. The17:44:02 final check has no owned process/running Moiren task.
The snapshot is not a WTS lock-state verdict, a retrospective1717 explanation,
input-delivery proof or a product/DPI defect. No retry or bypass follows.

## Pipeline integration checkpoint - 2026-11-05

The [current integration checkpoint](../../docs/validation/2026-11-04-pipeline-checkpoint.md)
preserves the full S0-S5 requirements. Callback observation is committed as
`1b33b68`; integration commit `b471975` incorporates pinned remote main
`95972f2` in a separate worktree, preserving the original PCM/probe/validation
drafts. Final-source local workspace verification records 2607 passed,
10 ignored and 0 failed; all-target check, strict Clippy, RT and probe checks
also pass. The release completes at 00:31:55 and is pinned but not deployed.
The initial meter-wake failure and original cache regressions remain archived
alongside their focused and full-suite successful reruns.

Fresh exact-head CI `41868136039` is not green: Linux callback-helper cfg and
the native plugin fixture under compiler Miri expose portability/test-boundary
failures. Scoped repairs are committed as `fa47ffc`; local callback Windows/
Linux and both VAD Miri modes pass, native compiler passes 54 tests on each OS,
and compiler Miri passes 53 portable tests in each model. Windows audio-library
regression passes 873 tests (5 ignored); strict engine/audio Clippy and the
117-test VAD example pass. CI-matching Linux Clippy and new exact-head CI are
still pending. No lint gate or native audio acceptance is weakened. PR26
remains Draft and must retain its split-before-merge policy rather than merge
wholesale.

The next head `796e5ce` starts CI `41874555383`: both VAD Miri jobs and compiler
Stacked Miri pass, but Linux all-target test/Clippy expose a missed portable
shadow callback caller. The follow-up preserves that allocation test, restores
portable test cfgs and exercises the helpers in a new owner-isolation library
regression instead of hiding dead code. Local Linux library/shadow, Windows
callback/shadow/Clippy and both VAD Miri checks pass; its new exact-head CI is
still required. The original cfg-only repair was incomplete, not a qualification
pass.

The next local Linux all-target run exposes a separate Windows-only trace
exporter fixture import. Gating just that module to Windows preserves portable
analyzers and callback allocation coverage. With that correction, Linux audio
all-target tests pass 1259 tests (2 ignored) at 01:20:44; Windows probe tests
pass 740 (1 ignored), and strict Windows audio Clippy/fmt pass. This is a local
platform-boundary repair, not exact-head remote CI or native qualification.

The normal follow-up push is `3c2c094`, CI `41878002933`. Its exact-head Ubuntu
test and strict Clippy (Rust 1.99.1) now pass, as does Windows Clippy. Other jobs
remain live at the observation; no all-green or native verdict follows. The
older run becomes terminal cancelled, not retroactively successful.

That `3c2c094` run subsequently completes at 02:00 with all 20 jobs successful,
including both test/Clippy platforms and every Miri suite/aggregate. This is a
verified software portability checkpoint only; the later streaming-harness
source still needs its own CI and native qualification remains unchanged.

The [S4 playback-category candidate](../../docs/validation/2026-11-05-s4-virtual-playback-category.md)
is independently reviewed and imported, with fresh 18-group host/13-group
portable checks, analyzed WDK build and valid INF. Playback now declares the
standard software output terminal rather than a physical speaker, with a
matching Association and explicit enablement; capture and default-role
exclusion remain unchanged. This is a candidate repair, not actual Windows
FriendlyName, migration, audio or default-role acceptance. Exact native name
and historical held-client gates are not relaxed; the adapter suffix may still
cause a naming failure. The unsigned integration package has separate pins
from the agent-worktree build and is not installed.

The [streaming rename harness](../../docs/validation/2026-11-05-s4-streaming-rename-harness.md)
is imported with separate old/new actual Start/PCM, exact raw-name, direct
Removed and fresh Engine/native identity gates. All 127 host example tests,
non-test build and strict Clippy pass; imported workspace/gpui regression
passes 2609 tests (10 ignored). The local Linux app check lacks desktop
development libraries. Native execution and the broader S4 matrix remain
unverified; software integration does not close naming acceptance.

After installing CI-equivalent Linux development dependencies and restoring
unchanged pinned root-logo assets omitted from the isolated archive, that
local example check passes all 106 portable tests. Earlier preparation failures
remain retained. The streaming harness commit is `15ba9d0`, not native evidence.

The [explicit-error acceptance repair](../../docs/validation/2026-11-05-s4-sticky-runtime-errors.md)
prevents later clean snapshots or a successful service join from hiding
observed transition/retirement or shutdown errors. Three actual assertion REDs
become 130 passing Windows and 109 passing Linux stable example tests; strict
Clippy/check/build and the application shutdown-timeout regression pass.
Error-free Removing remains distinct from final PDO absence. This is a
fail-closed harness correction, not native validation or a driver audio fix.

At 02:32:42 exact published streaming/FIFO head `303e481` completes CI
`41883280004` with all 20 jobs successful, including Windows/Ubuntu tests,
strict Clippy, driver host and all Miri suites/aggregate. The later
explicit-error repair still requires its own published-head CI. Neither
software result changes the native qualification verdict.

The [PCM readiness correction](../../docs/validation/2026-11-05-s4-pcm-readmission.md)
`52d0c39` makes the streaming completion snapshot use the same bounded native
gate state machine as its main loop. Legal temporary Start re-admission remains
pending until both gates reopen; existing strict PCM/identity/deadline gates
are preserved. Actual snapshot age is rechecked after pumping. Failure-first
tests pass 135 on Windows and 114 on Linux stable, with strict Clippy/check/build
and 2623 workspace/headless tests (10 ignored) passing. No native trajectory or
broader S4 gate is qualified by these software regressions.

Exact earlier head `fb87ec8` completes CI `41888724160` at 03:13:37 with all
20 jobs successful; the existing watcher exits 0. Subsequent PCM-readiness and
S5 corrections still require their own published-head CI. The
[S5 percentile-contract repair](../../docs/validation/2026-11-05-s5-percentile-target.md)
rejects a p99 target miss that the older p95-only offline evaluator accepted.
Both p95 and p99 plus existing calibration uncertainty must now be strictly
below 10 ms, as the current goal requires. Actual library/CLI/wrapper REDs are
retained. Windows/Linux qualification tests pass 26 each, full probe tests
pass 741/583 respectively (1 Windows ignore), and workspace/headless passes
2625 with 10 ignored and zero failed. No native or physical gate is closed.

Published readiness/percentile head `919d10f` has matching CI `41896388334`
still running. A later scoped
[offline PCM receipt repair](../../docs/validation/2026-11-05-s1-pcm-refusal-persistence.md)
`0523359` prevents failed output persistence from leaving a successful
structural summary and rejects overflowing JSON numbers before refusal
serialization. Failure-first tests pass 35 on both Windows and Debian;
all 17 Python suites pass 321. This later source is kept local rather than
superseding live CI. No original PCM, frozen residual or native verdict changes.

The [current release probe receipt](../../docs/validation/2026-11-05-release-probe-readiness.md)
pins both optimized examples built from clean `73e1fc2`; the release E2E
executable passes the real offline wrapper regression. No VAD integration
execution or deployment occurs. Fresh read-only SSH still returns an admin
token/session 0, so the non-admin metadata-reader guard remains unsatisfied;
VMware inventory remains zero running. These builds and account checks do not
close any naming, waveform, sustained or physical gate.

Portable checks do not alter native qualification: genuine `0x8007051A`,
epoch/rebase and waveform-continuity evidence remains open. Ordinary desktop
retained Stop/Evaluate/Apply, Windows-observed naming and lifecycle cases,
sustained load and calibrated physical p95/p99 acceptance remain incomplete.
No lower default, VAD/Process winner, physical sub-10-ms pass or device-limited
impossibility follows from the software integration. The first-packet
investigation remains deferred.

The [frozen terminal FIFO audit](../../docs/validation/2026-11-05-terminal-client-fifo-bound.md)
uses only eight hash-matching JSON metadata archives. Actual full-interval
padding/submission accounting limits each owner's remaining client queue to
512 frames; at least 177 of the replacement's original 689 residual frames
already lie in a guaranteed-departed client-FIFO prefix. Thus the entire
residual cannot be dismissed as client Stop tail. Client departure is not DAC
presentation, residual totals and both delivery-gap failures remain, and no
waveform/physical acceptance follows. The planned remote PTS audit refuses an
administrator SSH token before deployment; its non-admin gate stays intact.

The latest read-only physical-target preflight is 2026-11-05 00:48:44: no owned
audio app/probe or running Moiren task is present, and protected OBS/QQMusic
creation identities match. An older stalled transfer shell is still visible;
its presence is not an audio run and does not justify broad process cleanup.
The driver VM's first strict SSH attempt times out. At 2026-11-05 00:11 VMware reports no
running VMs; the verified dedicated VM's single headless start attempt refuses
because a password is required. User unlock/start is needed for that target.
The 00:45 read-only VMware inventory still reports zero running VMs; the failed
start is not repeated.
No new native audio, UI or driver operation is run by this checkpoint.
Historical native receipts below retain their original source and target
identities.

## Application test scope - 2026-11-01

The latest [PCM-pump admission repair and remote retest](../../docs/validation/2026-11-01-vad-pcm-pump-admission.md)
is locally committed as1956636 after10 red/green admission regressions,
2075 workspace passes,15 measured UI passes, strict Clippy/fmt and release
builds. It stops new READ/WRITE after observed local revocation while preserving
pending drain and genuine errors; it is not atomic native epoch serialization.
The authorized old remote GUI owner is stopped only after identity/hash checks
and log preservation; no host process is manipulated. Three new exact
Playback -> unity Bus -> MIC runs retain144/144 buffers and Engine32. Native1
changes epoch while open/Running after about5.3 source seconds, then retains
`0x8007051A` shutdown faults; no cause attribution is claimed. Same-configuration
bounded-WPP native2/3 complete the30-second source and cleanup but both fail
quality:6/4 native position gaps, with one MIC no-copy/silence packet in native3.
No packet flag erases those gaps. Remote plots/raw evidence remain available;
all test owners/children are gone, owned traces stopped, original OBS/QQMusic,
driver/hash/boot/defaults/render controls unchanged. New app is staged, not yet
desktop recovery-tested. Native waveform/latency and S1-S3 remain incomplete.

A [fresh desktop log fault](../../docs/validation/2026-11-01-live-gui-vad-revision-fault.md)
also occurs at13:02:26+08:00 in the running e6076f9 desktop, generation3:
`GetOverlappedResult(VAD PCM)` returns `0x8007051A` and the owner enters
`WaitingForResources`. This is not just the earlier native probe's retirement
fault. Its original INFO log lacks request direction/stamp and fault-time
endpoint state; native epoch/gate/stream changes remain hypotheses, not a
proven ABI mismatch or a reason to convert the fault to cancellation. The
later Process-only recording does not exercise this failed VAD owner.
Fault-only diagnostics are now built and staged; the retest above exercises
native teardown, not reproduction of this desktop completion call-site.
No established root repair or VAD runtime acceptance is claimed.

A separate [gate-close admission regression](../../docs/validation/2026-11-01-vad-gate-close-admission.md)
demonstrates that explicit close and preparation-fault polling could leave
endpoint-local session admission current until after native gate IO, or even
after successful close. The narrow repair revokes affected local sessions
before validated close, preserving shared sibling authority, stale-dispatch
refusal, original errors and drain/epoch obligations. This is a proved local
contract defect, not the established cause of the13:02 desktop failure. It
does not serialize PCM against native epoch changes or establish native
recovery, continuity, latency or S1-S3 completion.

The separate [retained OBS two-path analysis](../../docs/validation/2026-11-01-obs-two-path-analysis.md)
now fixes numerical/spectral/search defects and passes14 analyzer regressions
on host and target.43 strict broadband windows show relative path separation
p50=105.466ms,p95=106.226ms, not calibrated physical latency. Source noise
has64 complete reference windows with fixed141483-frame alignment and minimum
correlation0.999999875, but the reference sweep tail loses that mapping.
All rejected/non-noise candidates remain visible. No continuous returned-path,
VAD MIC, fault-recovery or sub-10-ms acceptance follows; the fresh desktop
`0x8007051A` remains open.

Application/native testing is remote-only by user direction. Missing physical
multichannel recording hardware defers calibrated physical measurement, not
software/application progress. The separate [remote OBS/QQMusic smoke](../../docs/validation/2026-11-01-remote-obs-qqmusic.md)
retains15-second concurrent Process Loopback/actual OBS recordings with finite
nonzero audio and explicit startup silence; this does not exercise VAD MIC or
the Moiren desktop graph. Installed611 driver, boot, defaults/master controls
and original OBS configuration remain unchanged. The VAD slot0 `0x8007051A`
cleanup failure remains open; gate-close/pending-request ordering is a source
inspection hypothesis, not a confirmed root fix or waived terminal error.

## Latest fixed-driver software-route checkpoint - 2026-11-01

The user-authorized [current-code regression](../../docs/validation/2026-11-01-current-vad-native-regression.md)
uses497957e including353f0db, with installed611/oem205 and its original SYS hash
unchanged. All71 requested host regressions pass. One exact Playback -> unity
Bus -> MIC native acquisition negotiates144/144 frames, Engine32, and retains
10114 packets/1456416 frames. Source submission/postroll complete, but runtime
retirement fails slot0 `0x8007051A`; collection/cleanup and quality are incomplete.
Nominal analysis resolves21/29 codes, not proved missing frames. Independent
readback verifies91130 scalar checkpoints, preserving startup/tail XRUN and
epoch barriers rather than promoting coarse zero steady counters to a pass.
The archive is host-verified before removing only the exact terminal task.
No owners/raw children remain; boot, six default roles and12 render controls
match preflight. No driver/system-control change, physical output or latency
qualification occurs. This is not an isolated SRC-fix A/B or a new driver result.

## Latest physical checkpoint - 2026-10-31

First-start MIC poll fix `9968505` requests half-packet wait (144frames ->1.5ms)
only in the fresh admitted Paused window, preserving Running-only WRITE and
all driver/buffer/default/recovery policies. Its actual same-fixture control
`mic-startup-poll1` fails after4.7s: first retained Running startup silence0
with3 host completions is followed by gate epoch1->2, driver silence2 at first
fault/6live/7removed and cleanup slot2 `0x8007051A`. Source and runtime cleanup
are incomplete; task/probe1 and all partial PCM are retained. Root verifies
the28-payload archive; the nominal method identifies only1 of5 acknowledged
source codes, with the other24 schedule codes not fully submitted. Neither
missing detections nor1576 QPC-policy invalidations are proven losses. The
new33781248-byte app is deployed side by side with help/monitor-help/offline
smokes0, not new desktop/native acceptance. Host701 audio/622 probe tests,
strict Clippy/fmt pass; no physical latency, stability or minimum is proven:
[MIC startup poll control](../../docs/validation/2026-10-31-mic-startup-poll.md).

The new Interactive/Limited exact `Playback -> unity Bus -> MIC` control
`interactive-iobus1` completes source submission, collection and cleanup, with
source480/MIC144 native frames and Engine32. Its acquired1456272-frame PCM is
finite and nonzero, but quality fails: first MIC driver silence3 becomes9 and
remains9 after removal. Host silence counters0 do not override driver silence.
The unchanged exact-nominal method resolves only4/29 marker identities;
25 missing detections and9616 packet-QPC policy invalidations are not proven
marker/packet losses. The37-payload archive is root-verified, all four earlier
loose artifacts match byte-for-byte, and independent acquired audit passes93/93
after retaining an initial92/93 checker failure. The exact terminal task is
removed after fresh guards; six default roles and12 render-master snapshots
match, driver610/boot are unchanged and no owners/children remain. No physical
output, calibrated latency, quality waiver or lower-default promotion follows:
[interactive IO/Bus control](../../docs/validation/2026-10-31-interactive-iobus-control.md).

The separate Interactive/Limited Shared-source control now retains nonzero
Process Float32 PCM (287520 frames) and accepted provider PCM16 READ data
(332640 frames), independently audited75/75. This is the existing
`Playback -> Gain(0.5) -> MIC` holder, not exact S1; neither boundary records
post-Gain MIC output. Capture is partial and the historical all-zero cause
is not isolated by a matched session-only A/B. Six default roles and12 render
master snapshots match; all four exact terminal tasks are removed after
archive verification. No settings/default/budget or driver changes occur:
[Shared VAD interactive control](../../docs/validation/2026-10-31-shared-vad-interactive-control.md).

The separate physical target retains the earlier main-PR52-integrated app
`5f9eb02` (main `74ef5dd`),33780736 bytes, SHA256
`4B3DD61FA488F5BB5C36AA9D0124AB0912D55FE79F49CFC33EE18882E5AFBCDE`,
staged side by side with earlier app copies, and explicitly enabled
native-Playback/no-copy diagnostic driver `883a5be`: `oem204.inf`, version
`3.1.1.610` (previous `2a5aee5`/609 is retained for rollback).
Installed SYS SHA256 remains
`94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`;
driver source/build inputs are unchanged, excluding dated STATUS updates.
Service Running/root PnP OK, exact installed SYS hash, target signatures and
catalog members are verified. The 609,608 and older607 rollback packages remain;
no reboot, trust/volume/default change or target build occurs. Native retries
are authorized; older paused-retry statements below describe dated history.
Only DAWN headphones may be used for audible runs. The driver acquisitions
below add no physical render client. Fresh5f9 help/monitor-help/offline smokes
pass without native audio; the earlier7023 desktop retry explicitly opens DAWN
and is not relabelled as a new5f9 desktop result:
[integrated5f9 deployment](../../docs/validation/2026-10-31-integrated5f9-deployment.md).

The earlier current-app console1 retry fails Settings UIA before measured
Apply; its forced termination and original evidence remain retained. The later
actual74f and latest7023 desktop regressions complete generation1/2
Start/restart/Stop,13 UIA Invokes, owner cleanup/reaping and normal app/source
exit0 on pinned DAWN.
Latest7023 retains47 payloads plus manifest, independently verified, and removes
the exact terminal task after archival. Boot, six default roles and12 render
endpoint master volume/mute snapshots match before/after; this is not proof
about all session/channel/capture controls or transient settings. This closes
that bounded current-binary control/lifecycle regression, not complete
source-waveform, meter-frame visual, lower-budget S3 or physical quality
acceptance:
[latest7023 application deployment and desktop receipt](../../docs/validation/2026-10-31-app-deployment-desktop.md#main-pr51-integrated-app-and-latest7023-desktop-control).

Probe `18d1552` additionally retains close-gate, stop-broadcast, worker-observed
stop, actual native Stop and join call windows. The actual indexed-v2
Playback480/MIC144 run completes collection/cleanup and stores10109 packets;
fresh585 full/22 focused host tests and independent64-check archive audit pass.
MIC tail silence1 and cleanup reset/recovery1 remain quality-incomplete.
Recorder sample timestamps cannot identify the exact driver no-copy increment;
one operation-unidentified trace revision mismatch remains. See the
[stop-window receipt](../../docs/validation/2026-10-31-capture-stop-windows.md).

The newer610 diagnostic directly retains the first actual MIC no-copy
increment's original gate/state and matching event/Notify. A pre-Start inactive
MIC attempt fails; the same-fixture retry completes10110 packets but remains
quality-incomplete. Its sole silence0->1 increment has gate2 closed/Running;
the existing callback sample, not the exact increment timestamp, falls after
stop broadcast and before native Stop entry. Notify succeeds, but retained
PCM packet identity is not independently proven. Root verifies deployment and
both-run archive; independent native audit passes96 checks. The original91/1
inventory-assertion RED and first run's temporary owned MMDevice catalog entry
remain retained; default roles and preexisting render controls are unchanged.
No counter, buffer, quality exemption or physical qualification changes:
[no-copy witness receipt](../../docs/validation/2026-10-31-microphone-no-copy-witness.md).

Probe-only `8b574cc` now preserves MIC service until native Stop/release/join;
both external stops still broadcast before waiting. Physical retirement and
installed610 driver stay unchanged. Integrated618 host tests and independent
review pass. A single actual same-fixture run completes10110 packets and
cleanup with final MIC silence0, retaining reset/recovery1 rather than erasing
them. Root verifies the27-payload archive and frozen provenance. The current
collector intentionally keeps quality_complete/qualified false; unknown15 is
historical live-read coverage, not the cause of that fixed verdict. This is
operational progress, not sustained waveform or physical latency acceptance:
[native-first retirement receipt](../../docs/validation/2026-10-31-native-first-mic-retirement.md).

The user permits an evidenced device-limited theoretical minimum only if
sub-10-ms is demonstrated impossible under the current declared conditions.
Neither that impossibility nor a calibrated physical minimum is established;
quality gates and the distinction from software diagnostics remain unchanged.

The later exact Input -> unity Bus -> DAWN Exclusive256 control retains
actual period256 but fails final host cleanup with SET_GATE `0x8007051A`.
Its17-payload failure archive is root-verified and independently audited49/0.
Probe-only `a20dfe2` moves original Playback gate close before physical-path
revocation/Stop; source review16/0 and root620 tests/Clippy/fmt/release pass.
MIC/Process paths, installed610, buffers and defaults remain unchanged. One
same256 native replay completes35 seconds with no native/cleanup error;
separate128 refuses before Running against queried device minimum144.
Both archives are verified; no physical delay or minimum is established:
[physical retirement receipt](../../docs/validation/2026-10-31-physical-soak-retirement.md).

Feature-gated `06b3daa` then permits explicit WASAPI Exclusive render144 for
diagnostics, keeping feature-off behavior and app presets/defaults unchanged.
Actual DAWN admits144 period/capacity; separate Engine64 target1152 and945
controls each finish35 seconds without native/cleanup error or reported xrun,
drop or silence. Prime32 and final reset/recovery1 are retained. Root feature-
on/off659 library/622 probe tests, strict Clippy/fmt/release and independent
source review pass; both acquired archives independently audit59/0. No PCM,
physical floor or calibrated sub10ms conclusion follows:
[exact144 receipt](../../docs/validation/2026-10-31-dawn-exact144.md).
The subsequent one-variable Engine32/target945 then Engine32/target881 controls
also finish35 seconds with actual144 and no reported native/cleanup failure;
each archive independently audits62/0. Prime32 and final reset/recovery1 remain.
881 is the current declared-policy model minimum, not a qualified default or
physical theoretical minimum; sustained signal and physical quality remain open.
The same881 budget subsequently completes305.004622s with a fresh300-second
adjacent repeated-v1 changing-source schedule, no reported native/live fault and
clean teardown. Source submission completion and the root-verified17-payload
archive remain diagnostic only: no delivered PCM/calibration or physical bound
is established. Independent acquired-evidence audit passes72/0. Driver610,
default settings and quality gates are unchanged.
The subsequent additive decode-call telemetry control keeps that305-second
source/configuration but is not a successful soak: report observed duration0,
live reset/recovery1 and recovery-prime32, QUERY dropped4, and four terminal
PCM shutdown `0x8007051A` failures remain. Only an early original-owner call
segment is eligible; its small maximum cannot describe the ambiguous suffix
or physical latency. The17-payload archive is root-verified and independently
audited53/53 with the initial52/53 harness assertion retained. Driver610 is
unchanged; no error is waived or gap/jitter budget reduced:
[decode-call receipt](../../docs/validation/2026-10-31-vad-decode-call-envelope.md).

The frozen binary/settings then complete305.0073755s with only existing WPP
enabled: no reported live reset/recovery/drop/xrun and clean native/host teardown.
Eligible call envelopes retain maximum3.708ms, not physical latency or proven
consumer delivery. Matching610 trace decode records123 events, reported loss/
format errors/unknowns0, and no ClockRebase message; run1's fault is not reproduced
or explained. Both audio and separate trace archives are integrity-verified.
Controller0/probe1/wrapper1 retain an unqualified diagnostic, not a new cleanup
failure. No PCM/calibration, gap/jitter reduction, default promotion or physical
lower bound follows:
[existing-WPP control](../../docs/validation/2026-10-31-vad-call-envelope-wpp-control.md).

Direction remains virtual speaker/Playback -> Moiren Input/Source and
virtual microphone/MIC -> Moiren Output/Sink, already merged in PR44.
A distinct existing Exclusive event Playback control now passes on the same
609 driver: all four mono/stereo PCM16/Float32 cases return matching nonzero
PCM. The actual stereo PCM16/480 case retains95040 nonzero native event-buffer
frames and95520 nonzero post-copy READ frames with equal scanned source and
destination regions. Matching exact native packet shape/layout does not make
this a sharing-only comparison: waveform, prime, ownership and source pumping
also differ from the earlier Shared fixture. No Engine/Bus/MIC/physical route
or continuity/latency qualification follows. Session0 alone does not prevent
this Exclusive control. Clean cleanup, unchanged volume/defaults, pinned source,
verified archive and limitations are in the
[Exclusive positive-control receipt](../../docs/validation/2026-10-31-exclusive-playback-control.md).

Two subsequent exact Playback/Input -> Bus -> MIC/Output collections use the
default Exclusive period, actual Float32/480 at both native ends. They retain
nonzero microphone PCM and complete source submission, but both fail EngineHost
shutdown with SET_GATE revision mismatch. A single existing-debug-logger repeat
observes a transport close refusal after Pause and later successful lifecycle
closure; decisive confirmation-query evidence is missing in those two earlier
acquisitions. No guard is
relaxed or failure called a pass. Verified archives, separate internal cleanup
failure/external absence, waveform-analysis limits and next diagnostic boundary:
[default-period IO/Bus receipt](../../docs/validation/2026-10-31-default-period-iobus.md).

A third acquisition isolates the original probe plus logging only. Both
decisive close confirmations now show same-stream generation1, gate2 -> 3,
Paused and still open; refusal is correct, not successful concurrent closure.
Native reset/retirement sequencing is the next investigation boundary, not a
reason to relax gate guards. This run also retains a separate early Running
MIC `silence_packets=1` quality fault. It is not a passing collection.
Pins, verified archive, independent audit and limits:
[close-confirmation receipt](../../docs/validation/2026-10-31-close-confirmation.md).

A probe-only virtual-route retirement slice now closes gates before external
Stop/reset, retaining both Stop broadcasts before every join and leaving the
physical route unchanged. Software regressions/review pass; the isolated probe
waits for an unrelated target app to exit, then one bounded native acquisition
completes collection/cleanup with exit0 and Closed EngineHost. It is not a full
quality pass: quality_complete=false, unknown observations and a MIC tail
silence_packets=1 remain retained. Continuous quality and latency are open:
[retirement-order checkpoint](../../docs/validation/2026-10-31-mic-retirement-order.md).

The new READ-selected, bounded native-buffer observer records the actual
consumption path and bytes before the original clearing. A retained first
attempt loses MIC readiness before any source starts. An unchanged-fixture
retry completes: the actual path is EVENT, two packets, not timer/Scratch.
All 480000 sampled native frames are zero after the existing EOS handling
and before clear. Event destination counters duplicate the same single read,
not an independent paired-copy witness. First/last EOS flags are zero; middle
EOS states are not individually retained. Original post-copy READ and provider
captures also retain 480000 all-zero frames. Native sequences 2..1001 differ
from READ/provider 1..1000; 999 logical packet IDs overlap, not exact sample
alignment. Complete 26-fragment native and unchanged 15-fragment READ groups,
source nonzero write witness, failure, cleanup and verified archive remain in
the [native-consumption receipt](../../docs/validation/2026-10-31-native-playback-ingress.md).
This narrows a zero observation to the consumed event buffer; it establishes
neither a sole cause nor nonzero continuity/physical latency. Session 0 remains
an unproven confound. App help/offline smoke passes separately; GUI/native app,
exact S1-S3 and calibrated sub-10-ms acceptance remain open. The preceding
READ-only observer checkpoint is retained in the
[Playback-copy receipt](../../docs/validation/2026-10-31-playback-copy-boundary.md).

The pause-fence repair serializes epoch publication with PCM queue draining
and adds a stream-release fence without weakening the Rust cancellation guard.
All13 MSVC host suites,8 WSL portable suites,54 pause-fence scenarios,93 Rust
provider tests and final WDK analysis/INF validation pass. Bounded old/new
native observations retain unchanged-epoch Pause on the old driver and epoch
increments on the new one; both60-second holders complete without nested
errors. The original fatal holder failure occurs in neither, so no causal
native-fix or waveform-continuity claim follows. Exact deployment, rollback,
verified archive and limitations are in the
[pause-fence receipt](../../docs/validation/2026-10-31-vad-pause-fence.md).

A separate source-process capture also retains all-zero native PCM without
starting a Process monitor or DAWN render client. Exact source identity,
nonzero-source submission within the active schedule and full cleanup are
retained, but a nonzero Process positive control and sole root cause remain
unproven. The active VAD holder fixture is not graph-free or exact S1. See the
[source-side receipt](../../docs/validation/2026-10-31-source-process-control.md).

A separately reviewed Microsoft-derived Process diagnostic records only
-1/0/+1 PCM16 values at44.1kHz, not the intended changing waveform. All API
stages complete, source submission remains within its active schedule and
cleanup is clean; format differs from the earlier48kHz Float32 recorder, so
this is not a matched implementation A/B or a demonstrated dither/session
cause. Exact boundaries, tests and verified archive are in the
[independent reference receipt](../../docs/validation/2026-10-31-ms-process-reference.md).

An opt-in artifact from the actual provider READ, before decode/carrier/Engine,
now retains480000stereo PCM16frames, all exactly zero. Of1000published packets,
991transfer callbacks fall strictly inside the changing source's conservative
Running-to-Stop window. This localizes this run's zero observation at or before
READ, not solely in downstream Engine processing, but does not establish a sole
driver/Windows/source cause or physical latency. The diagnostic adds no second
READ/Process/physical client and leaves routing/defaults unchanged. Cleanup and
memberwise archives are retained in the
[provider input receipt](../../docs/validation/2026-10-31-provider-input-capture.md).

A fixed-holder continuation now witnesses16nonzero Float32 bytes in the actual
source GetBuffer destination, matching the encoded prefix before successful
ReleaseBuffer with flags0. The subsequent provider READ still contains480000
all-zero stereo PCM16frames: all1000callbacks follow Release, and990also satisfy
the conservative source-active observation window. This narrows the unobserved interval
between client Release and accepted READ, not to a sole driver/Windows cause.
No waveform/latency/default gate changes. Exact implementation, bounds and
cleanup are in the
[source write receipt](../../docs/validation/2026-10-31-source-write-witness.md).

A cheaper exact-client read-only control check observes session Active and
session/endpoint master 1.0, both unmuted, after the first nonzero Release.
The provider still retains 480000 all-zero PCM16 frames. Those master controls
are not zero/muted at that instant; other gain/processing/transition causes
remain unproven. No new client, volume write or default/driver change occurs.
Exact snapshot timing, tests, cleanup and memberwise archive are in the
[source controls receipt](../../docs/validation/2026-10-31-source-controls.md).

Shared microphone controls now correlate a successful ACX notification with a
nonzero post-copy native-buffer prefix in the same packet. Ordinary/RAW Shared
clients still receive all-zero/SILENT PCM, including the explicit changing AC
diagnostic. This narrows the observed boundary but proves neither a sole-driver
nor a downstream-policy cause. Exact evidence and limitations are in the
[Shared microphone receipt](../../docs/validation/2026-10-31-shared-microphone-direct.md).

A VAD -> unity Bus -> DAWN Exclusive256 candidate with Engine64,
target1152/capacity4800 and actual VAD period144 completes65 seconds with
3,121,664 output frames,12,194 wakes and no output wait timeout. No waveform is
acquired, so this is not continuity or physical-latency acceptance. Initial
prime32 remains counted. See the
[DAWN control receipt](../../docs/validation/2026-10-31-vad-dawn-continuous-control.md).

The app's separate Process -> unity Bus -> DAWN Exclusive256 per-wake control
confirms replacement and10 seconds of operation; final native counters are
sampled after owner join, not copied from the last Running snapshot. It still
retains one delivery-gap violation, an all-zero independent recording and a
holder failure with nested cleanup `Cancelled`. A window-admission control
separately refuses ambiguous overlapping packet/read envelopes. Neither control
qualifies waveform delivery, and the operational per-wake result does not
override the window refusal. Both holder idle requests end early, not at their
requested180 seconds. See the
[joined app receipt](../../docs/validation/2026-10-31-app-exclusive-joined.md).

Earlier target768 controls, their failed acceptance gates and original
archives remain in the
[integration/capture-boundary receipt](../../docs/validation/2026-10-31-native-integration-capture-boundary.md).

Directions are from Moiren's perspective: virtual speaker is input/Source;
virtual microphone is output/Sink. Accepted ownership/direction/control-error
foundations and bounded UTF-16 name staging are merged separately in PR43-45
and PR47, not by merging draft PR26. PR47's final20CI checks pass; this is
bounded safety coverage, not native endpoint-name propagation acceptance.
No lower default, calibrated sub-10-ms result or VAD/Process winner is claimed.

**Long-soak gate status (updated for the 2026-10-22 run, `cdd44a2`):** the
eight-hour **idle** run passed for the first time — 28,730/28,800 verified
windows, zero epochs, flat resource trend (see the "Long-soak attempt 2"
closing section of `docs/vad-validation-2026-10-21.md`). The eight-hour
**music** phase and a full 16-hour pass remain unqualified: the music gate
tripped environmentally on this VM (capture-gap + correlation-lag wander), and
the run verdict concludes full 16h PASS is not achievable on that host without
a lag-tracker measurement fix.

The development project links ACX audio circuits and bounded Direct I/O PCM.
A test-signed package is now installed in the VMware Windows 11 target, and
bounded raw-child/control-lifetime checks have passed there. **P1 target-machine
qualification has not passed.** A metadata-only child MEDIA/null-service INF
has now been validated on target, and all four owned MMDevices map successfully.
Production INF child matching is implemented in `e8f769f`. The later
`production-binding` run removes the scratch child package, verifies all four
live MEDIA children bind the combined production INF, and passes eight exclusive
event-mode cases with half-period microphone pacing. The subsequent
`native-timer-production-phase` run also passes the full eight-case timer matrix.
These are bounded target results, not stable-repeatability or complete product
qualification: earlier timer runs aborted on real long-stall epoch mismatches,
and a fresh-OS installation has not been qualified.

## Short timer packet progress - 2026-10-29

Local `9cba24b` fixes a reproduced source-level starvation:48/49-frame timer
packets with1.1ms callbacks could continuously rebase with zero progress.
Short packets now admit one bounded quantum of residual debt (capped by mapped
frames), while every callback retains one-packet/three-span/one-transfer work
bounds. Larger packets, event mode, queues and microphone freshness remain
unchanged. Overload still quiesces and records loss; timely redue service and
native PCM retention are not guaranteed by this software policy.

All10 MSVC host suites, WDK analyzed build and native INF validation pass.
Two independent final source reviews find no Important/Critical issue. The
generated unsigned development package is neither installed nor deployed; no
native audio or target qualification occurs. Hardware gates and prior run
verdicts are unchanged. Exact RED/GREEN, bounds and final artifact hashes:
[`2026-10-29-vad-short-timer-progress.md`](../../docs/validation/2026-10-29-vad-short-timer-progress.md).

## Separate physical target - 2026-10-29

The subsequently analyzed/test-signed package is installed on the separate
physical `LAPTOP-NP49RG6H`, not the development host: `ROOT\MEDIA\0001`,
`oem147.inf`, version `18.58.53.555`, service Running / PnP OK. Exact installed
SYS hash, package verification and deployment history are retained in the
[physical-target receipt](../../docs/validation/2026-10-29-physical-target-deployment.md).
This supersedes only the earlier statement that the generated short-timer
package was not deployed; it does not retroactively change that build receipt.

Bounded native VAD IO/Bus recordings now contain changing PCM and complete
collection/cleanup at explicit2048/1024 targets with independent capacity.
They still fail independent-recorder continuity; the2048 case also retains
provider faults. No stable lower preset or calibrated sub-10-ms physical
result is established. The output remains muted and a common pre-route
reference/calibration is not available. P1 and all unfinished long-soak/music
gates remain unqualified. VM timing qualification stays deferred; these
physical results do not erase its historical failures or prove naming
propagation while a native external client holds an endpoint.

The later [native recorder/Process follow-up](../../docs/validation/2026-10-29-native-process-followup.md)
reproduces the independent recorder's raw first-packet discontinuity without a
Process graph. Later packet adjacency has no observed mismatch; startup prefix
integrity is still unknown, and the strict quality failure is not exempted.
Native app parameter replacement now has separate bounded evidence, retaining
later delivery-envelope violations. Neither finding changes the VAD provider,
physical continuity, calibration or long-soak verdicts.

The [packet-period / Engine-block matrix](../../docs/validation/2026-10-29-vad-period-block-matrix.md)
observes144/480-frame playback packets with128/256-frame Engine blocks at
target1024/capacity4800. Three operation/cleanup cases complete; all four fail
recorder quality, and the fourth also retains microphone drop/silence and a
diagnosed probe Running/timeline-availability conflation. Original failed
receipts remain unchanged. No kernel fix, physical latency verdict, lower
default or long-soak qualification follows from this bounded software route.

Probe-only [progress-health repair](../../docs/validation/2026-10-29-vad-engine-progress-health.md)
`f72e03c` now separates transient unknown timeline data from confirmed Running
state, with pinned identity and bounded progress deadlines. Final509 probe
tests pass. A new480-packet/128-block/target1024 native retest completes its
schedule/collection/cleanup but still fails recorder continuity; no transient
null recurs in this run. Installed driver and application are unchanged, build
caches are retained, and physical/long-soak qualification remains open.

### DAWN and interrupted software-route follow-up - 2026-10-30

The later [DAWN app-output receipt](../../docs/validation/2026-10-30-dawn-vad-app-pcm.md)
uses the authorized DAWN headphones rather than the muted Realtek output of
the earlier physical receipt. Production VAD -> Bus -> DAWN PCM is retained;
no independent calibrated physical reference or continuity qualification is
established. No other speaker volume/mute or default role is changed.

Probe-only [source-readiness coordination](../../docs/validation/2026-10-30-vad-source-readiness.md)
waits for a bounded accepted-READ observation before starting its owned source.
Three repaired repetitions record zero driver drops. A35-second physical-master
diagnostic admits Exclusive256 / Engine32 / target768 / capacity4800 with zero
retained active loss/reset/xrun counters, but acquires no PCM and does not qualify
its declared delivery bound. No production default/provider/kernel change.

The separate [pure-virtual follow-up](../../docs/validation/2026-10-30-vad-software-readiness-crash.md)
retains cap1 overflow/reset failures and a cap8/target1024 completed30-second
recording that still fails driver-silence/recorder-continuity gates. Cap8/target768
refuses its budget; cap4/target768 is interrupted by a0x7E bugcheck and automatic
reboot before results persist. Matched kernel symbols/images recover an access
violation at `nvlddmkm+0x262fa6` in a System worker chain, without proving sole
root cause or excluding earlier corruption/indirect Moiren involvement. No
Moiren frame appears in that recovered stack. Native audio retries are paused,
not treated as passed or repeated automatically.

The final post-reboot audit retains the same installed SYS hash, service Running,
root PnP OK and no dynamic child/app/probe/build process. All three exact terminal
owned tasks are unregistered after verified archival; Cargo caches remain.
P1, naming propagation, physical sub-10ms and outstanding long-soak/music gates
remain unqualified. No new driver installation, system-setting change or
speculative kernel patch follows from the crash.

The offline [virtual-clock observability repair](../../docs/validation/2026-10-30-virtual-clock-loss.md)
adds retained owner-level omitted-period/frame counters and mandatory final
probe validation for the pure-virtual route. Native PCM loss and Engine periods
omitted by the recovery cap remain distinct. This changes no kernel source,
clock policy, default budget or hardware gate; native retries remain paused.

The [cross-target/RT-harness follow-up](../../docs/validation/2026-10-30-native-helper-cfg.md)
retains Linux/Miri compilation and private-clock test integration repairs,
plus the final `c222df2` application/probe deployment. Installed SYS remains
unchanged, Running /root PnP OK; no native audio retry or new hardware verdict.

The [probe example-test cfg follow-up](../../docs/validation/2026-10-30-probe-test-cfg.md)
repairs four Windows-only test-build helpers and deploys only the `9de94e4`
probe after528+5 release tests and strict Clippy. App `c222df2` and installed
SYS remain unchanged. Native retries stay paused; software checks do not
close physical continuity, calibration or lower-budget qualification gates.

## Audit follow-up - 2026-10-21

The reported Removing-state retry defect is absent in the current source:
`Revoke` accepts Removing, `VadRemoveChild` retains the PDO and retries
`MarkMissing`, and failed ACX removal leaves the added flag set and returns its
failure. Commit `7ac7283` adds production-extracted removal/cleanup/destroy tests
covering repeated failures, successful submission idempotence, callback timing,
identity checks, quota retirement and reference balance. All six MSVC and
portable host suites pass. Three scratch mutations (skip the retry, swallow
failure, remove the accepted-submission guard) are rejected by the regression.
No kernel source change or new target package is required for this finding.

Commit `93a9c0c` ignores only local driver project settings and generated
x64/ARM64 Debug/Release directories. The separate user-owned `driver/moiren`
template never entered the repository and was removed from the working tree on
2026-10-22.

The authenticated-user control ACL remains the deliberate standard-user,
single-owner design. It does not authenticate the Moiren executable or isolate
mutually untrusted local users. A competing permitted process can open an idle
bus first. See the [access boundary and continuous-test procedure](../../docs/vad-soak-validation.md).
An attempted limited S4U diagnostic task still received an administrator token;
its preflight refused to launch the probe. The temporary task was removed.
This is not a standard-user target PASS.

Fresh host checks pass: workspace tests (1,322 passed, zero failed, six ignored),
workspace all-target check and strict Clippy, gpui-test acceptance, and the WDK
analyzed build/INF validation. These host results do not establish target
long-duration stability. The opt-in continuous soak and its runner require
their own completed target evidence; starting or rehearsing them is not an
eight-hour PASS. Historical timer failures and outstanding release gates below
remain open. The 2026-10-22 eight-hour idle soak pass (header note above)
closes the idle-duration gate; the music-phase and 16h duration gates stay
open.

## Implemented source

- Independent x64 Debug ACX 1.1/KMDF 1.31 package, WDK 10.0.28000.0, MEDIA root
  `ROOT\MoirenVad`. The root starts with no audio circuit. Cargo does not use WDK.
- One exclusive, non-inheritable owning control file object, standard-user ACL,
  named-open rejection and session epochs. Duplicated handles share ownership.
  Cleanup closes gates first, cancels PCM and requests asynchronous removal.
- Up to 64 dynamic raw audio PDOs, registered/removed using ACX circuit-device
  APIs. No static child-list publication. Quota includes creating/removing PDOs
  until final destruction. Shared state survives root cleanup and late children.
- Stable 128-bit user endpoint IDs determine bus-relative PDO instance IDs.
  Runtime tokens/generations remain separate. Repeated identity is refused
  while its old PDO is still alive, including active-client removal delays.
- Each circuit uses a component URI containing its direction and the complete
  stable endpoint ID. Identity survives Prepare/Release and remove/recreate,
  and does not share the earlier provisional direction-wide ComponentIDs.
  Host tests cover all 128 identity bits, direction separation and byte order.
- Child PrepareHardware stages disabled AUDIO/direction/REALTIME interfaces on
  a system worker, with matching Association and NeverSetAsDefaultEndpoint in
  EP/0. Only after success does it call AcxDeviceAddCircuit. A local allow-default
  flag changes eligibility; no code selects a system default device.
- Mono/stereo, 48 kHz PCM16/Float32, RAW/DEFAULT, one native stream per endpoint.
  The source permits native streams with Engine gate closed: render discards,
  capture emits zero. The bounded event-mode target run now observes microphone
  silence before admission and after gate closure. No tone/file paths,
  demonstration mute/volume or DSP/SRC.
- Two native MDL packets for the event-driven path, or one page-backed native
  ring for the timer-driven path. Single-packet allocation is admitted with
  segmented position/consumption and PCM-copy helpers; timer rundown precedes
  packet release. This is implemented source, not target compatibility evidence.
  QPC deadlines rebase after stalls instead of replaying every missed packet.
  Packet wrap, EOS bounds and capture availability have host regression tests.
- Circuits and pins are constructed afresh in PrepareHardware after policy
  staging. ReleaseHardware clears handles deleted by AcxDeviceRemoveCircuit;
  partial unpublished construction is discarded before retry.
- ABI v2 buffered management and OUT_DIRECT render / IN_DIRECT capture PCM.
  Up to four pending requests per endpoint, each one native packet, at most
  4,800 frames and 38,400 PCM bytes. There is no extra kernel PCM ring.
  Four slots are capacity, not mandatory concurrency. Microphone WRITE can use
  one event-paced pending request with four client slots preallocated; the
  driver still rejects payload older than one native period at consumption.
  MDLs map at passive submission, never in the timer; queued cancellation is
  owned by WDF. The timer returns a fixed four-entry completion batch, releases
  all spin locks, then completes retrieved requests with PCM rundown still held.
- Session/endpoint/stream/gate epochs fence all PCM. Gate close, stream reopen,
  resume/stall and retirement reject old requests. Every full packet-clock reset
  advances the gate epoch before PCM purge, including ReleaseHardware,
  FreeRtPackets and cleanup. Paused state and the new epoch are published under
  the same lock, blocking new-epoch admission until RUN. Same-object
  Release/Prepare cannot reuse an old stamp even when polling misses the stopped
  state; epoch exhaustion closes admission. Old microphone payloads are
  discarded by QPC age. Empty capture queues emit zero; render overflow drops.
- Root rundown protects queues from late DPC/child access. Passive per-queue
  drains serialize forwarding against cancel/removal. Control purge is restarted
  after owner cleanup; a second instance cannot take over a healthy session.
- A precreated control work item claims one cancelable request from the
  sequential queue via a spin-lock-protected single-slot handoff. Cancellation
  before claim completes exactly once; successful unmark transfers ownership to
  the worker, then completion or the existing PCM queue. No new PCM queue or
  per-request work allocation is introduced. Synchronous drains execute outside
  every EvtIo callback. Cleanup drains the control work item before restart.
- Root initialization failure shuts down even partially created PCM queues
  before EvtDeviceAdd returns. Final root cleanup releases shared storage only;
  it never uses child queue handles that WDF may already have deleted.

## Verification and remaining gates

`build.ps1 -Analyze` has passed linked SYS generation, C++ analysis with zero
warnings, API validation, native INF validation and CAT generation. That evidence
applies to the checked source revision; later source changes must rerun the check.
The five host suites are lifecycle, audio, ownership, timer and cleanup. They cover
ABI/bounds, clock/pause/resume, rational deadlines, render wrap/EOS, capture,
bounded completion batches and cancellation/claim ownership helpers. Extracted
production-helper regressions cover packet-clock reset fencing before counters
restart, stale-stamp rejection, sequence-zero admission only under a new epoch,
closed-gate reset, epoch exhaustion, event/timer allocation shapes and segmented
timer-ring PCM/gate isolation. Host tests do not exercise WDF, PnP, cancellation
or real audio APIs.

### Target observations - 2026-10-21

- Target access is established over SSH to the guest ending in `.128`, with a
  high-integrity session. The guest is VMware Windows 11 build 26200. Guest
  automatic sleep and hibernation are verified disabled; the earlier guest-access
  and power-preflight blockers no longer describe the current state.
- A test-signed package has been installed and exercised in the VM. This does
  not qualify production signing, other Windows builds or upgrade/rollback.
- Four dynamic raw children simultaneously create/remove/recreate with sibling
  preservation: PASS. This establishes the exercised raw-PDO lifecycle behavior,
  not four usable MMDevice endpoints or independent native audio routing.
- Root cleanup revokes the exact owning file object: PASS. Ten sequential `cap`
  opens, process kill and subsequent fresh reopen: PASS. No owner/data-driven
  driver fault was reported in those bounded control-lifetime checks; they did
  not exercise native PCM.
- Target `active-lifecycle-stage1` exits 0 at 01:07:49 on 2026-10-21
  (Asia/Shanghai). With an exclusive capture client held and a 480-frame buffer,
  gate closure leaves the stream Running. Remove is accepted as Removing for
  runtime PDO 25, then final Removed is observed after 25 ms while the native
  client is still held. After client release, the same stable endpoint recreates
  as runtime PDO/endpoint generation 26 with the same MMDevice ID. All six
  defaults are unchanged. Evidence: `D:/tmp/moiren-vad-20261020/active-lifecycle-stage1.result.json`,
  `active-lifecycle-stage1.stdout.log` and `active-lifecycle-stage1.stderr.log`
  in that directory. This is stage1 active-removal evidence only, not PCM,
  silence, crash-cleanup or default-policy qualification.
- App `app-integration-stage1` exits 0 at 01:11:25 on 2026-10-21
  (Asia/Shanghai), reporting PASS in 270 ms. It verifies the persisted pure
  virtual input -> Gain 0.5 -> microphone graph through production Windows
  `ManagedDriver` ownership, without starting native probe clients. Apply reaches
  App service Running with active revision 2 and open gates; Stop reaches Idle
  with endpoints Present and gates closed; Exit reaches Exited with the service
  joined and owning control file released. `pcm_verified=false` and
  `endpoint_absence_confirmed=false`: Exited/join is not final PDO absence.
  Evidence: `D:/tmp/moiren-vad-20261020/app-integration-stage1.*` (result/report
  JSON and stdout/stderr logs). Root reports the actual `SessionSpec` gate
  regression reproduced RED then fixed GREEN, with the final 229 App library
  tests passing, including the explicit run-limit regression. Final host evidence:
  `D:/tmp/moiren-vad-20261020/app-virtual-intent-final-tests.log`.
  This was Stage1 only; its native-client PCM gate was still open at that point.
- App `app-integration-stage2-readmission2` and `app-integration-stage2-readmission3`
  both exit 0 without timeout on the Windows 11 VMware guest `DRIVER-TEST`.
  Both use executable SHA256
  `FEEDC9F47333FD5C7E5016BFD8C050CA72D96025E1A56278180D4D5F75199E3C`, built
  from source commit `6c4e3c285a704fd67f49993488f89527dec2d9c1`. The persisted v2
  typed graph is VirtualPlayback -> Gain(0.5) -> VirtualMicrophone. With no
  native probe clients, a 4,004-4,005 ms idle hold observes real `frame_end`
  progress: 14 forward advances at 48 kHz and no timeline epoch change. The
  exact-root/stable-ID MMDevice mapping then starts stereo Float32, 48 kHz,
  exclusive event-driven WASAPI render and capture with 480-frame buffers and
  no SRC or fallback. Each run validates source `[0.25, -0.5]` as
  `[0.125, -0.25]` through Gain 0.5 and a second distinct signal
  `[-0.375, 0.125]` as `[-0.1875, 0.0625]`. Longest consecutive matching runs
  are 48,000/48,000 frames in run2 and 48,000/48,096 in run3; each independent
  signal phase meets the 48,000-frame threshold. During Stop, capture remains
  RUN at request, after acknowledgment, and while collecting 48,000 real silent
  frames in each run. The service joins and the owning control session releases.
  Gate telemetry observes capture stream generation 0 -> 1, a closed interval
  of 31 ms / 34 ms, then reopen. Playback stream generation also advances 0 ->
  1; its sampled gate remains open while the bounded readmission observation is
  pending, and later PCM checkpoints confirm both gates open. On service exit,
  both endpoints are only `Removing`; `endpoint_absence_confirmed=false`.
  Evidence (per tag: `.result.json`, `.report.json`, `.stderr.log`):
  `D:/tmp/moiren-vad-20261020/app-integration-stage2-readmission2` and
  `D:/tmp/moiren-vad-20261020/app-integration-stage2-readmission3`. This is two
  bounded passes, not long stress or release qualification. A subsequent
  read-only PnP snapshot found no present `MOIREN\VAD_RAW` children after the
  asynchronous service-exit removal had time to complete. That later snapshot
  is distinct from the run reports' `Removing` state and is not default-policy
  evidence. Default-device policy/arrival is not qualified; the probe does not
  change user defaults.
  Timer-mode, shared-mode and physical-device paths, eight-hour music/16-hour
  runs (the eight-hour idle run passed on 2026-10-22), 100-round
  lifecycle/crash tests, Verifier and production signing remain unqualified.
- The earlier ALL-states (mask 15) MMDevice result of 0/4 is superseded by a
  target-validated metadata-only child MEDIA/null-service INF: all four owned
  MMDevices now map successfully. Production INF matching was completed in
  `e8f769f`. Both `packet-offset-install` and `timerelapsed-install` updated the
  root using the production INF, but those earlier runs alone did not exclude
  selection of the same-rank `oem26` scratch child INF. The later production
  binding check below resolves that selection ambiguity, not fresh-OS install
  qualification.
- In an earlier run, the native probe entered its first exclusive Event Playback
  Mono Pcm16 case and started the native stream, but the first PCM transfer
  failed at `GetOverlappedResult` with revision mismatch `0x8007051A`. That result
  is historical; the later event-mode run below reaches eight successful cases.
- The first offset of the event-driven double buffer must be
  `allocation - packetSize`; the former offset 0 produced alternating half-buffer
  silence. After correction, target `native-packet-offset` passes 8/8 exclusive
  event-mode cases: mono/stereo, PCM16/Float32 and both directions. Each
  microphone gate-closed observation before admission and after closure contains
  12,000 silent frames. All six default-device observations are unchanged
  before/after the run.
- Event-mode playback records 47,040-47,520 matching frames/s with zero silence.
  Microphone records 32,640-37,920 matching frames/s and 21-32 dropped packets/s.
  Those significant microphone drops are part of the result, not a lossless
  or full-rate claim; an eight-case PASS does not remove that limitation.
- The `timerelapsed-install` timer-mode run passes all four playback cases.
  Its first microphone case records 23,136 signal frames and 25,344 silent
  frames, below the unchanged 50% matching signal threshold. This is the
  baseline before the probe-side half-period microphone pacing experiment.
- Probe microphone pacing keeps the first write immediate and at most
  one pending request, then waits half a native packet period after collecting
  Capture/CaptureDropped before the next write. Driver freshness and the signal
  threshold are unchanged. The first target run, `native-timer-halfphase`, aborts
  on a real long-stall epoch mismatch during stereo playback: not a PASS.
- The second run, `native-timer-halfphase-2`, passes the first seven cases, then
  fails after a long stall in case eight, Microphone Stereo Float32. Its failure
  diagnostic records approximately 22k signal frames, 942 silent frames and zero
  microphone drops at the abort; these are partial-run counters, not per-second
  rates or a full-case PASS. The phase shift substantially improves microphone
  deadline behavior in this observation, but does not eliminate long-stall epoch
  failures or establish an eight-case timer PASS for either of those runs. The
  pacing policy is retained as a probe fix, not a driver freshness change.
  Phase pacing alone does not prove sample-age freshness; the shallow-FIFO
  issue remains open and is being addressed separately.
- The `production-binding` check removes and verifies removal of only the
  scratch `oem26` child package, with no live children, then adds the combined
  production package using `pnputil /add-driver`. All four live children report
  Class MEDIA, INF `oem30`, section `Child.NT` and complete driver keys. This
  excludes scratch-INF selection for that run. Evidence is recorded in
  `production-binding.result`, `production-binding.properties` and
  `production-binding.stdout`. This is not a fresh-OS clean install.
- Under that verified production child binding, exclusive event mode passes
  8/8 cases. Half-period pacing records microphone matching rates of
  43,200-48,000 frames/s, 0-4,800 silent frames and 0-1 dropped packets; playback
  records 43,680-47,040 matching frames/s with zero silence. All six default-device
  observations are unchanged before/after. These later counters do not erase the
  earlier unpaced microphone-drop results.
- The `native-timer-production-phase` result exits 0 at 00:56:08 on 2026-10-21
  (Asia/Shanghai), passing all eight exclusive timer cases. Microphone matching
  rates are 46,053-47,712 frames/s, with 288-768 silent frames and zero drops in
  every microphone case. Playback matching rates are 45,056-47,104 frames/s,
  with 512-2,048 startup-silent frames. Page-rounded native buffer sizes are
  2,048 frames for mono PCM16, 1,024 for mono Float32 and stereo PCM16, and 512
  for stereo Float32. Positions advance in every case; all gate-silence checks
  contain zero signal, and defaults are unchanged. This full-matrix bounded PASS
  does not invalidate the earlier long-stall failures or establish repeatability.

Required target checks remain: fresh-OS installation and repeatable production
package installation/publication; repeated event/timer matrices, including
long-stall epoch behavior; sustained microphone delivery and PCM quality beyond
the bounded runs; investigation of earlier event-mode microphone drops; sustained
PCM rate/latency and broader event/timer client compatibility; standard-user
control (the current session is high integrity); pre-exposure default
eligibility; software mute/volume;
broader active-audio-client revoke/remove/recreate beyond the bounded stage1
result, including PCM and crash cleanup; App Stage2 native-client PCM and final
endpoint-absence checks beyond the two bounded Stage2 passes; default-device
policy and arrival qualification; timer/shared/physical-path integration; power
transitions; Verifier; eight-hour audio/16-hour runs (idle passed 2026-10-22);
100 lifecycle/crash rounds;
resource/latency reports; upgrade/rollback/uninstall; production signing and
broader Windows-version validation. Raw-child and exact-file cleanup PASS results
do not close these audio-path or long-duration gates. The Stage2 probe does not
change user defaults and does not qualify their arrival policy.

The per-endpoint component URI replaces the provisional direction-wide IDs.
A regression first reproduced equal identities for two distinct endpoints,
then passed with the complete stable-ID mapping. This fixes the source-level
identity contract. The target now passes simultaneous four-raw-child lifecycle
and sibling preservation, and the metadata-only child INF permits four owned
MMDevices to map successfully. The bounded event-mode run additionally validates
the eight native PCM cases. Concurrent sibling audio-routing isolation and
fresh-OS installation qualification remain open. Production child binding without
scratch selection and one full matrix in each native mode are now observed;
endpoint mapping alone still does not prove PCM flow.
The opt-in `vad_lifecycle --exercise --simultaneous` probe holds four raw PDO
combinations and replaces each while checking sibling identities/epochs. The
bounded target lifecycle results above remain separate from opening KS streams.

The separate opt-in `vad_native --exercise` probe now prepares four owned PDOs
and requires four distinct MMDevice identities mapped through topology adapters
and their PnP parent to the exact control root and stable endpoint IDs. It then
requests all eight exact exclusive format/direction/shape combinations, checks
PCM patterns through Direct I/O and native clients, and observes microphone
silence on the same RUN stream before admission and after gate closure. Default
mode remains exclusive event-driven. The opt-in `--exercise --timer` mode uses
no-event initialization with bounded padding/position-driven polling and no
fallback to event mode. It requires fresh clock progress during the signal phase.
The timer-capable probe passed 19 host tests before these target runs, without
contacting a driver. Failure diagnostics retain signal/silence, microphone drops,
buffer/packet sizes, stream generation, mode and native position without replacing
the original error or skipping revoke/drain. The diagnostic version passed 23
host tests; diagnostic plus half-period pacing passed 25 host tests, scoped
Clippy with warnings denied and a local build. Those checks do not qualify
target audio behavior.
The guest passes bounded eight-case exclusive event-mode runs after the
packet-offset correction. The later verified-production-binding run with
half-period pacing improves the microphone counters as reported above; earlier
significant drops remain part of the evidence.
The earlier timer baseline passes four playback cases but fails its first
microphone signal threshold. Half-period pacing improves observed microphone
delivery; its two target attempts nevertheless abort on long stalls, with the
second reaching case eight after seven PASS results. The subsequent
`native-timer-production-phase` run passes the full eight-case timer matrix, but
does not close the long-stall/repeatability gate. These results do not close
fresh-OS installation, long-duration PCM quality, policy, shared software volume
or active-audio-client gates.
