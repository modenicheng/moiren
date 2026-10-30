# Native testing resumed after explicit authorization

Date: 2026-10-30, Asia/Shanghai. Target: `LAPTOP-NP49RG6H`,
original SSH identity `192.168.12.97`; the current user-supplied transport is
`frp-rug.com:14857`, verified against that existing host key. The user explicitly authorizes continued testing and
automatic retries after a blue screen. This supersedes the testing pause
recorded in the earlier dated crash, clock-loss and portability receipts;
those historical receipts and failures remain unchanged.

After a suspected crash, preserve the run directory, launch identity, available
dump/events and last observations. Reconnect and distinguish a changed boot
from an SSH timeout. Verify machine, installed driver/binaries, present children,
free space and existing audio/build processes before retrying. Do not launch a
duplicate if the original task is still running. A new attempt needs a new
evidence directory; a reboot or terminal task result is not successful cleanup
of the interrupted attempt. Reconnect/check/retry does not require another
conversational approval. This is not permission to change GPU/security settings,
install on the development host, terminate unrelated work or request reboots.

Only the explicitly pinned DAWN headphone endpoint is an audible physical sink.
Do not change any speaker volume, mute state or default role. Remote builds and
audio acquisition do not overlap; reuse the existing D: Cargo cache. Retain and
verify evidence off Cargo output before any eventual disk cleanup.

## First resumed acquisitions

Probe source `9de94e4`, SHA-256
`851954D6FCC4224C35AA6CD29D569183C22D901D8BC637956571EB89BA967D8A`;
installed SYS SHA-256
`CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.
Direct SSH launches work without an interactive console session. Both runs use
the exact VAD playback -> unity Bus -> VAD microphone software route, 48 kHz
stereo, Engine32, capacity4800, jitter96, declared packet/gap480 and minimum
playback period144. Correctness/3000 ms is scheduling grace, not latency.

- `resume-vad-software1`, target768/cap4: source stops after4.3010574seconds,
  acknowledging206208 of1440000 scheduled frames. Provider gate identity changes;
  collection and its operational cleanup verdict fail. Final joined accounting
  retains5 clamped periods /160 omitted Engine frames, maximum backlog9 and no
  timeline reset. This is not proof of the cause of the provider transition.
- `resume-vad-software2-cap8`, target1024/cap8: collection and cleanup complete,
  all1440000 scheduled frames acknowledged. Final virtual-clock accounting has
  zero clamped frames/resets, maximum backlog2,305 late-wake bursts and1457248
  rendered frames. Retained active playback/microphone provider loss, silence,
  epoch-reset and recovery counters are zero. Nevertheless the native recorder
  flags all1456320 stored frames silent and its actual PCM is entirely zero;
  one startup discontinuity is retained. `quality_passed:false`. Operational
  exit0 is explicitly not a usable waveform, continuity or latency pass.

No new blue screen occurs in these two attempts; boot remains
`2026-10-30T09:50:49.659845+08:00`. Both finish without forced termination,
leave only the root PnP device present/OK and leave no owned audio/build process.
All render volume/mute values and default roles remain unchanged. No driver
installation, application-runtime replacement or cache clean occurs.

Raw scripts, PCM, packet metadata and reports live outside Cargo output in
`D:\coding\moiren-evidence\20261030-native-resume`. The host verifies32 downloaded
files /42288278bytes against a fresh remote length/SHA-256 inventory before any
deletion. These failed/partial quality results do not complete S1-S3 or establish
a VAD/Process latency winner, safe lower default or stable sub-10-ms result.

## All-zero waveform localization

`resume-vad-software3-audit` repeats the1024/cap8 route and also records the
probe's process tree for20seconds. Both stored recordings are entirely zero.
Its fresh active catalog shows playback and its source session unmuted at1.0;
the microphone is unmuted at0.5368811. No settings are written. Exclusive render
can be invisible to Process Loopback, so this alone cannot localize a source
failure.

`resume-vad-software4-default` changes only the native playback period request
back to its default480frames. Operational collection/cleanup complete, but all
1455840 stored frames remain flagged silent. The minimum144 request is therefore
not a sufficient explanation for this all-silent observation.

The separate `resume-dawn-shared-source1` bypasses VAD and Engine entirely:
the owned marker source renders Shared directly to pinned DAWN, while a sibling
process records that source tree. It stores959520frames with no artifact loss,
discontinuity, timestamp-error or native SILENT flags; actual PCM is nevertheless
entirely zero. Thus a VAD-only defect is not established. Logged-off/session0
execution differs from previous nonzero interactive acquisitions, but is only
a hypothesis, not a proven cause or a general Windows session rule.

Probe-only source `821a3e1` adds acknowledged nonzero-frame count and first
nonzero native submission position. This closes a diagnostic gap: scheduled
frame count did not independently show that any nonzero samples were generated.
Both values commit only after successful `ReleaseBuffer`; failed writes do not
advance them. They inspect existing generated samples using bounded scalar work,
without clocks, logging, allocation or audio-value changes. No production audio,
driver, queue, period or default changes are made. These values are neither a
native delivery acknowledgement nor proof of encoded/copied bytes or playback.

The regression first fails with missing serialized evidence (`null` versus2048).
It checks144/480/1056-frame chunks, exact2048 nonzero generated marker frames,
native-preroll coordinates, failed-release rollback and postroll stability.
Host and target release probe suites each pass529tests; host strict probe/test
Clippy, fmt and diff checks pass. Independent source review finds no P1/P2 issue.
The first exact-name test invocation selects zero tests; that log is retained
and is not counted as the failing regression.

The new probe is deployed beside the existing binary,7029248bytes, SHA-256
`9D73CD1A1867B24EEEE2CD1C61EEE68BDCBB9244C77578416C59DC48327E4E55`.
Its repeated Shared DAWN source run confirms20480 nonzero generated frames
accepted by `ReleaseBuffer`, first native position25056 including1056-frame
preroll. The sibling recorder still stores959520 entirely zero frames. This
narrows the unknown boundary beyond source generation; it does not assign the
cause to the recorder, Windows, session policy or the physical device.

At21:13 the user supplies the new SSH forward `frp-rug.com:14857`. The first
connection verifies the existing192.168.12.97 host key through `HostKeyAlias`
and confirms the same machine and unchanged09:50:49boot, no new dump, no owned
build/audio process, exact Running driver hash and only the root PnP device.
The preceding LAN timeouts are network-path failures, not evidence of a new
blue screen. Native work resumes through that forward without duplicate launch.

## Native microphone boundary and exclusive acquisition

`resume-vad-software5-payload` repeats the complete virtual graph with the new
source counter. All1440000 source frames are acknowledged, including29696
nonzero generated frames; the first nonzero native position is24144, accounting
for144frames of native preroll. The joined graph clock reports no clamps or
resets and maximum backlog2. Nevertheless all1455840 Shared recorder frames are
flagged silent and actual retained PCM is zero. This is not a source-delivery
or continuity pass despite operational collection/cleanup completion.

The separate `vad_native` probe bypasses the Engine and Shared recorder:

- `resume-vad-native-direct1`: minimum144 playback -> actual driver READ,
  Mono/Stereo x PCM16/Float32. Each case receives95760 signal frames, zero silent
  frames and zero discontinuities.
- `resume-vad-native-microphone1`: minimum144 driver WRITE -> exclusive capture.
  Each case obtains signal, but the cases retain1..8 dropped packets and
  2016..4896 silent frames. Command exit0 only establishes the probe's minimum
  signal condition, not zero-fault quality.
- `resume-vad-native-microphone-default1`: default480 driver WRITE -> exclusive
  capture. Each of the four cases receives95520 signal frames, zero silent
  frames, zero drops and zero discontinuities over its short2second window.

These constant-pattern native probes prove neither changing-waveform delivery
through the complete Engine/Bus graph nor physical latency. They motivate an
explicit exclusive recorder on the exact owned microphone, rather than another
identical Shared retry or an unproven production-driver change.

Probe collection schema2 now accepts optional `capture_mode:"exclusive"` only
for the owned microphone route. Omission remains Shared; a physical route or
schema1 cannot select exclusive, and native refusal never falls back to Shared.
The capture request stays Float32 at the native default period. Existing exact
owned-endpoint resolution, evidence bounds and Stop/join ownership are retained.

Independent review catches a prerequisite in the existing production WASAPI
adapter: its read path uses Shared-only `GetNextPacketSize` and treats exclusive
`AUDCLNT_E_BUFFER_ERROR` as fatal. Two regressions fail with those exact HRESULT
branches before the fix. Commit `9516837` bypasses that query only for Exclusive
and treats only its documented unavailable-buffer result as an empty pass;
other native failures still propagate. Zero-frame results acquire no lease or
signal evidence; nonempty packets keep their existing RAII release. An empty
pass returns to the owner deadline/stop check and native wait, not a busy loop.
Commit `67054cc` separately introduces the probe opt-in. Host verification passes
621 library tests (2 ignored),531 probe tests, strict probe/test Clippy, fmt and
diff checks. Follow-up independent review finds no remaining P1/P2 issue. Native
complete-graph results are recorded separately, not inferred from these tests.

## Complete-graph exclusive waveform acquired

`resume-vad-software6-exclusive`, source `67054cc`, changes the recorder request
to Exclusive while retaining the prior Engine32 / target1024 / capacity4800 /
cap8 graph and minimum144 source. The microphone actually uses480frame packets.
It stores1456320frames of finite PCM,62988 nonzero scalar samples and peak
0.04741464. Native capture has zero silent frames, drops, position gaps,
discontinuities and timestamp-error packets. Both active providers retain zero
drops, silence, epoch resets and recovery. The joined virtual clock has no
clamps/resets and maximum backlog2. Source acknowledges all1440000frames,
including29696 generated nonzero frames. Collection and cleanup both complete.

This first successful acquisition exposes stale metadata: the collection
config and actual native request are Exclusive, but route intent still says
Shared. Do not edit the historical raw files. Commit `4749d35` copies the
validated config mode once into control-side route intent before artifacts or
native preparation. A new dry-run regression first fails Shared versus
Exclusive, then checks the corrected request in both manifest and report;
existing Shared-default coverage stays intact. Host532 probe tests and strict
Clippy/fmt/diff checks pass, and independent review finds no P1/P2 issue.

`resume-vad-software7-exclusive-report` reruns the same complete route with the
corrected binary. Both top-level and native graph recorder requests correctly
say Exclusive. It again stores1456320frames,62986 nonzero scalar samples, with
the same zero native-capture/provider loss counters and no joined clock
clamps/resets (maximum backlog2). Source acknowledges all1440000frames and29696
nonzero frames. Operational collection/cleanup and native-statistics quality
pass; neither result is physical qualification.

Retrospective single-channel xorshift-template correlation is deliberately
stricter than a nonzero check and does not relabel duplicated stereo as an
independent reference. At the unchanged exploratory0.7 threshold, software6
has28/29 peaks (marker27 scores0.69618); software7 has27/29 (markers7/25 score
0.67733/0.69174). Nearby onsets exist but are below threshold; do not lower the
threshold or declare marker continuity passed. Inter-marker onsets shorten
slightly during servo settling; nominal schedule subtraction is not a latency
measurement. Raw PCM and `marker-presence.json` retain these limits. The Shared
all-zero cause remains unresolved: nonzero Exclusive acquisition narrows the
mode-dependent boundary but proves no Windows session policy or driver defect.

Remote release library verification passes621tests (2 ignored) and the probe
passes532tests at `4749d35`; the release app also builds successfully. It is
deployed beside the original as `bin/moiren-app-4749d35.exe`,33490944bytes,
SHA-256 `DF1902F2CC6CC7CCA95422495AB6A56CD892AE766013CF353E7D4E6B3A26B733`.
The corrected probe is7026176bytes, SHA-256
`7446AAE64F6642B1558179F567836B3D2078F73A81C72E3B41D2BB2288A75413`.
No GUI is launched into the logged-off console or concurrently with capture.
The original app/probe and installed driver remain unchanged; native test
cleanup leaves only root PnP present/OK, no owned audio/build process, and all
speaker volume/mute and default roles unchanged. No new bugcheck occurs.

## Evidence retention

The first archive `native-resume-20261030.zip` contains167 manifest files and
168 ZIP entries,144693250 uncompressed bytes. Its1863005byte payload has SHA-256
`4901700ED4AC53F6E7902963A5C194A5DDAEA4A95DEA18B385BB7EE9B997EBF6`.
Host verification checks archive identity, exact entry names, every file length
and SHA-256 before extraction. The archive includes runs through
`resume-vad-software5-payload`, the minimum-period direct native probes, scripts,
build logs and final identity/catalog audit. The later default480 microphone
run is downloaded separately and is not falsely included in this archive.
No remote evidence or Cargo cache is deleted.

The separate `native-resume-exclusive-20261030.zip` retains the default480
microphone probe, both complete-graph exclusive acquisitions, both release-build
receipts and the app/probe deployment audit. It contains74 manifest files /
75entries,52810491 uncompressed bytes,1059212 archive bytes, SHA-256
`22D5EEFC4B3406F03CFEFFC506A438B157819C97E859E1932BF5B7A1BDF59E4C`.
Host verifies exact names, all lengths/hashes and byte-for-byte agreement with
the initial software6/7 downloads. Marker-presence analysis is host-derived and
kept alongside the archive rather than falsely included in its manifest.

## Main integration and upstream deployment

Merge `a29b13a` incorporates main `c719546` (PR40's idle UI wakeup work).
Fresh authenticated remote-ref inspection confirms that is still main on
2026-10-30. The initial GitHub SSH fetch refuses public-key authentication;
an invocation-local HTTPS credential helper succeeds without changing remotes,
stored credentials or SSH configuration. Main is an ancestor of the merge.

The merge preserves branch-specific cleanup, trace, native telemetry,
requested/active transport budgets and measured selections in snapshot equality.
Three real-mailbox regressions first fail when those fields are omitted, then
pass with deduplication restricted to genuinely identical snapshots. Joined
cleanup compares full host state and retained immutable trace Arc identity,
without walking its history on every publication.

Independent review also finds an interaction with upstream's one-second
observation throttle: a new native owner within the same app generation could
retain the previous owner's telemetry. The merged driver keys observation
batching on app generation, native owner and execution state, invalidates stale
native/professional caches before throttling, and bypasses the interval on an
identity transition. An actual-method regression fails with the old owner
visible, then passes through WaitingForResources -> Running without stale
observations reappearing. Initial test import/visibility compile failures remain
in host logs and are not counted as behavioral RED evidence. Final independent
review finds no remaining scoped P1/P2 issue.

Fresh verification at `a29b13a`:

- Host app library:482 passed /2 ignored; audio library:621 /2 ignored;
  probe:532 passed. Strict app/audio all-target Clippy, fmt and diff checks pass.
- Full host app `--features gpui-test`:575 library tests plus62 integration tests
  pass, zero failures,4 ignored across all targets. No ignored native-window or
  audio test is claimed executed.
- Target cached release:482 app library /2 ignored,621 audio library /2 ignored,
  532 probe tests pass; app and probe builds complete. These finite tests do not
  open native audio. Build and native acquisition never overlap.

The new artifacts deploy side by side under
`D:\MoirenTest\20261029-0118a54\bin`:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `moiren-app-a29b13a.exe` |33456640|`37B00F938AC27E2C1ACF958A116D0C9F3C0A7F7FADEABFF51FFDD88F16D435CE`|
| `e2e_latency_probe-a29b13a.exe` |7026176|`D895DF0AAA197A600E748980D3F4E51A870721D667DC1D3E603AEE19257C852C`|

The original app/probe, previous rollback versions and installed driver bytes
are unchanged. This is a new application/probe deployment, not a driver upgrade
or a claim that the GUI has been launched in the logged-off console.

## Merged-binary complete-route repeat

`resume-vad-software8-main-integrated` repeats software7's exact declared
configuration with the merged probe. It completes collection and cleanup,
stores1456320 finite frames /62970 nonzero scalar samples, peak0.04750768.
Capture silence, drops, position gaps, overflow, discontinuities and timestamp
errors are all zero. Both boundary probes retain zero xruns, and the joined
clock retains zero clamped frames/periods and timeline resets, maximum backlog2.
Source acknowledges1440000 frames including29696 generated nonzero frames.
Both the collection config and recorded route request correctly say Exclusive.

Native-statistics `quality_passed:true` remains separate from
`quality_complete:false`, `boundary_quality.quality_complete:false` and
`qualified:false`. The same retrospective0.7 template threshold finds27/29
peaks: markers2/6 score0.677725/0.697082. Neither marker continuity nor latency
qualification passes. No threshold is lowered, no input/output target is
changed, and no source schedule is treated as an independent recorder reference.

The comparison with earlier Shared acquisitions is not factor-isolated:
archived Shared software2 uses driver-side PCM16/480, while the Exclusive
software6/7 path uses Float32/480. Native client mode is not the only changed
boundary. Shared standalone DAWN -> ProcessLoopback also bypasses the VAD
driver, so no common driver, session-policy or APO cause is established. A
next discriminating VAD diagnostic would inspect format-aware encoded WRITE
payload immediately before submission and its terminal ticket completion,
rather than inferring signal from zero silence/drop counters or repeating an
identical all-zero Shared run. No speculative production-driver change is made.

Final target audit retains the same boot, no minidump, only root PnP present/OK,
no owned build/audio process, unchanged render volume/mute and default roles.
D: remains253753569280bytes free; Cargo caches and raw evidence are retained.

`native-resume-main-20261030.zip` retains run8, build/test/deployment receipts,
scripts and final audit:38 manifest files /39 entries,26340572 uncompressed
bytes,521732 archive bytes, SHA-256
`53A30D4A51E95F4CAF0AE048A8ABCD005C13527255189CABA36B60A075F60097`.
Host verifies exact entry names and every length/hash before extraction. The
derived marker analysis stays beside the archive. No remote deletion occurs.
The full S1-S3 objective, Shared waveform localization and independent
calibrated physical latency remain open; this checkpoint does not establish a
VAD/Process winner, a lower safe default or stable physical output below10ms.

## Production Process measurement readiness

The real app `monitor` path exposes a main-integration regression not covered
by an isolated transport probe. Its display observation is throttled to1Hz.
`resume-app-main-process` starts collection from Running, then at1001ms tries
to admit the still-incomplete startup snapshot and exits1 for missing packet
delivery. A separate unchanged2-second measurement completes replacement to
generation2 and3001ms of requested execution. The first launch wrapper also
incorrectly feeds INFO-prefixed stdout to `ConvertFrom-Json`; that wrapper
failure is preserved separately from the valid nested JSON and native refusal.
Neither the longer successful run nor a parser repair erases the original
production readiness defect.

Commits `014f8bd` and `cbe551d` separate two boundaries:

- CLI waits for matching Running native ownership, within its existing fixed
  startup/collection deadline. Missing samples do not renew the deadline.
- Measured service Apply reads the real native registry once after preflight,
  outside the outer host mutex, rather than admitting from the display cache.
  The fresh sample must match running owner, node and Process identity; final
  actual-host recheck remains. No cached DTO fallback, observation timestamp
  rewrite or change to1Hz display publication is introduced.

Regression cases first fail with fresh-good/cached-unknown and
fresh-unknown/cached-good disagreement, then pass. They also cover wrong native
owner, changed Process identity, malformed histories, zero reads on preflight
refusal and exactly one read on successful admission. Test-only readers do not
replace the production registry path. Commit `955ccbe` retains complete invalid
evidence as a hard refusal.

The target release at `cbe551d` is deployed side by side as
`moiren-app-cbe551d.exe`,33439744bytes, SHA-256
`DE832F4EDE1B64BFCEE5CB280C888FD90FEFDFAC32378015234B499FA078F4DD`.
Two exact1-second-measurement native repetitions then distinguish a successful
generation2 replacement /3000ms run from another startup prefix containing
only one producer event. Both stop and join owned processes normally; no
volume, mute, default-role, installed-driver or buffer changes occur.

Commit `95dbf6e` makes missing observation, insufficient history and absent
overlap typed CLI readiness waits, still under the original deadline. Review
then catches a first-error ordering defect: an insufficient producer history
can conceal a saturated consumer, malformed short history or delivery clock
fault. This candidate is built remotely but deliberately not deployed.

Commits `7dc6123` and `854e8a9` validate every present component and every
independently checkable delivery/producer relationship before returning a
readiness error. A genuine0/1-event prefix must first have valid counts,
sequence, frames and clocks. Missing peers do not hide known clock,
saturation, arithmetic or shape faults; no evidence is synthesized. Existing
budget thresholds, audio scheduling, RT behavior and display cadence remain
unchanged. CLI fixtures now distinguish genuinely missing delivery in both
mirrored fields from an inconsistent DTO, and a coherent short producer from
a truncated malformed history.

Behavioral RED evidence retains31passing/5failing library cases and a separate
cross-supply0passing/1failing case. Final focused library tests pass37/37;
the CLI mixed-fault test first fails, then all13 CLI measurement tests pass.
An intermediate fixture-only failure is retained rather than counted as a
production regression. Fresh full host verification at `854e8a9` passes629
audio library tests /2ignored,488 app library tests /2ignored and532 probe
tests. Strict app/audio all-target Clippy, workspace fmt and diff checks pass.

The native functional configuration remains `ProcessLoopback -> unity Bus ->
DAWN`,48kHz stereo, Engine256, Shared output default480 period /1056 buffer,
explicit measured target4096 /capacity16384 /jitter96,1second measurement and
3seconds requested execution. The owned low-amplitude changing-marker source
also renders to the same pinned DAWN device. This is a finite functional Apply
smoke, not a low-latency preset, independent physical reference, recorded
waveform or full-run/post-join quality accounting. Its latest1Hz counters must
not be described as final joined counters or stable sub-10ms output.

Historical verified archives retain all original failures and successes:

| Archive | Bytes | Files / entries | SHA-256 |
| --- | ---: | ---: | --- |
| `native-resume-app-main-20261030.zip` |80944|55 /56|`D22BBE3A42DD8E1DEA723C9410DF30C9EA33334BD94BE35E32545141F2FA6000`|
| `native-measured-fresh-20261030.zip` |77175|45 /46|`9FD6F4456943F858E4EB231E949215A390665D368E8C7CFD538F9D1175000AAE`|

Both archives have exact-name and per-file length/SHA verification on the host.
Their raw INFO-prefixed stdout is preserved; extracted monitor JSON is derived
host-side, not falsely listed in the original archive manifest. In the second
archive, the source audit is already `95dbf6e`, while the actually deployed
`cbe551d` app identity is in `build/deployment.json`; the audit's older
`DeployedAppHash` field names the retained `a29b13a` app, not the active test
binary.

On2026-10-31 the new FRP connection again verifies the original host key,
unchanged09:50:49 boot, Running driver and no build/audio processes before
the next build. The one retained System1001 event is the already-recorded
2026-10-30 09:51:04 bugcheck, not a new crash. D: has253715595264bytes free;
no cache or remote evidence cleanup is needed. Fresh authenticated fetch
confirms main remains `c719546`, already an ancestor of this branch.

The complete host `gpui-test` command is rerun serially at `854e8a9`:581
library plus62 integration tests pass, zero failures and4 ignored across all
targets. The previous full run at `95dbf6e` retains an unrelated compact-mixer
2-second UI timeout (580passed/1failed/3ignored); that exact test passes alone,
and the final serial full run passes without a threshold or production UI
change. It is not relabeled as a successful earlier run.

The hash-verified clean-source bundle for `854e8a9` builds on the target using
the existing cache, jobs2 and finite20-minute stage deadlines. Target release
audio629 /2ignored and app488 /2ignored pass, followed by a successful release
app build. It deploys side by side as `moiren-app-854e8a9.exe`,33460224bytes,
SHA-256 `3EAAFE820E689F524DD10963541CFA568709CF1FDC16866F3A2C8CBC1C3492C1`.
Only this corrected candidate is deployed; the superseded `95dbf6e` build is
not. Existing app/probe files and installed driver bytes remain unchanged.
Native acquisition starts only after all remote build processes exit.

Three corrected real-app repetitions all exit0, confirm generation2, complete
the requested run and cleanup, and leave no owned process. Requested budgets
and1-second measurement stay unchanged:

| Run suffix | Measurement ms | Running ms | Measured max gap frames | Latest captured/written frames | Later gap-bound violations |
| --- | ---: | ---: | ---: | ---: | ---: |
| `final-process1` |1001|3002|532|96960 /96960|4|
| `final-process2` |1012|3002|520|96960 /96960|15|
| `final-process3` |1004|3000|547|96480 /96480|0|

All latest displayed bridge snapshots have zero dropped/silent/underrun frames,
discontinuities and resets, with5888 priming /224 prime-discarded frames. The
first two also contain later inter-delivery gap-bound violations4/15; their
later maxima112538/114734 in100ns units exceed the measured532/520-frame
declarations. Run3's maximum113822 remains within its547-frame declaration.
These violations remain visible: the finite measured prefix does not predict
future scheduling, and successful Apply/exit is not a quality pass. No threshold,
buffer or counter is weakened or reset to manufacture acceptance. No waveform
or independent physical reference is recorded; every report says
`qualified:false`.

All three source processes exit0; render volume/mute and default roles are
unchanged. Final audit retains the same boot, Running SYS hash, original
rollback binaries, only the root present/OK and no owned build/audio processes.
No driver reinstall or Cargo clean occurs. `native-measured-final-20261031.zip`
retains66files /67entries,2378119 uncompressed bytes,165471 archive bytes,
SHA-256 `039EC2A0A69DB7AA492B4B79BE0A35FA540A91BE2A58BA83868FEEBE08BEDF49`.
The host verifies exact entry names and every length/SHA before extraction;
all three derived JSON reports stay beside, not inside, the raw archive.

The earlier batched push `9aaf9a7` passes all20 CI jobs in run41072113481.
That result belongs to its exact older head and does not verify the newer
readiness/fresh-sampling commits. PR26 remains draft/open; it is not merged.
Full S1-S3 acceptance, Shared waveform localization, matched physical routes
and calibrated stable sub-10ms output remain open.
