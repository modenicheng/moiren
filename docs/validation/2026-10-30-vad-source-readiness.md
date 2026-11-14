# VAD source admission and lower DAWN operating budget

Date: 2026-10-30. Source changes: `aee8c1b` and `9c86a1f`.
Target: `LAPTOP-NP49RG6H` / `192.168.12.97`.
**S1-S3 remain incomplete; these diagnostics are not physical latency evidence.**

## Startup defect and bounded probe repair

The provider's idle playback worker polls at10ms. Engine Running and an open
VAD gate can precede its first PCM READ submission. Starting the probe-owned
native source immediately can therefore leave the first driver transfer
without an eligible READ. This is a probe startup coordination defect, not
evidence that lowering a production input target is safe.

Three old-probe five-second repetitions use Exclusive256 physical output,
Engine128, minimum VAD source144 and target1024/capacity4800, with jitter96,
declared packet144/gap192 and recovery-cap1. All initially observe zero READ
submissions/completions. Cumulative driver drops are0,0,1; the third repetition
first records that startup drop at+10.1967ms. Final completed packets
are1666,1666,1665.
The earlier invalid observation-interval1 invocations exit2 before native work;
their three logs are retained, not counted as repetitions.

The repair waits before source activation for the still-Paused owned playback
export to report exactly four accepted submissions, four attempts and zero
completions/faults. It pins the admitted five-part stamp, stable identity,
channels, packet/format and Engine generation/progress. Fresh query provenance,
open gate, prepared transport, zero driver loss and a single absolute five-second
deadline are required. Missing/changed/faulted/late observations refuse source
activation. This applies to both ordinary and supervised VAD probe preparation.

The retained `native.engine.source_start_admission` survives later/final Engine
snapshots and includes initial/last observations, identity/shape, observation
count, duration and failure. This is an **observational accepted-submission
prefix**, not a native pending-request-depth handshake or physical latency.
Relaxed telemetry does not prove four requests currently pending in the driver.
No production queue/default, provider scheduling or driver code changes.

Three new-probe repetitions complete5.0046354/5.0044377/5.004504seconds, each
with1666 completions. Each observes four accepted READs and zero completions
before activation after two observations and10.3265/10.5763/10.485ms of startup
waiting. Cumulative driver drops and provider loss/gap/reset/recovery counters
remain zero through retained active observations. That startup wait is not
steady-state transport residency. Removal then adds one separately retained
reset/recovery event; it is not relabeled as an in-run event or erased.

## Lower physical-master candidate

The same native source144 and DAWN output admit Exclusive256 / Engine32 /
target768 / capacity4800, retaining packet144/gap192/jitter96/recovery-cap1.
The whole physical wake is384frames, SRC transfer385, low-water417 and modeled
minimum705. Requested target/prime/servo768 is observed exactly; spare4800-frame
capacity is not its operating target.

The35.0034312-second diagnostic completes11666 provider packets and the entire
1440000-frame source schedule. Maximum worker completion interval is4.1382ms,
above the declared192-frame/4ms gap. Worker completion and native arrival are
different boundaries: this neither proves a native-gap violation nor validates
the declared delivery bound. Actual source/provider packet shape is144frames;
the retained prepared budget still has `native_packet_frames: null` and must
not be described as a published negotiated-shape admission receipt.
Cumulative driver drops, provider carrier/quota/stale/drop/silence/reset/recovery,
missing sequence slots, playback xruns, short reads and recovery-prime silence
are zero in retained active/final observations. Initial prime-muted32frames
remain explicit; native source silent preroll144frames is separately retained.
Final physical output reports zero dropped/silent frames,
6571 wakes and zero timeouts. Source Stop/join and Removed/Dormant/Closed
retirement complete normally.

Explicit64/128 output requests each refuse the freshly queried minimum144frames
at48kHz, before source activation. They never silently become256. Their reports
retain the preparation failure and absent admitted generation; both exports
are Removed and hosts Closed afterward. No successful native period64/128 or
source activation is claimed.

All diagnostic executions exit1/unqualified. They acquire no PCM, have no
independent common reference and produce no latency distribution. This is one
shorter-budget native acceptance/statistics receipt, not a continuity pass,
production default promotion, VAD/Process winner or stable sub-10ms result.
The pure-virtual microphone route has a different clock/recovery budget and
must not inherit the physical-master candidate's verdict.

## Verification, artifacts and target state

- Local probe suite525 passed; strict example test Clippy, fmt and diff checks
  passed. The readiness slice previously recorded its failing regression and
 524-test green suite; the identity/shape follow-up adds one serializer regression.
- Independent read-only reviews accept the observational contract and the
  identity/shape evidence follow-up. They explicitly reject a pending-depth claim.
- Remote cached release probe suites524 and525 pass at their respective source
  revisions; both optimized probe builds complete. Final deployed probe7004160bytes,
  SHA-256 `F9F9E61020C9C3548C8FB185795A6210E6F9A406BE267DDB562DAA029F6AD634`.
- Archive `dawn-vad-source-readiness.zip`:296956bytes,109 manifest files /110entries,
  SHA-256 `9D4EF493AE43C57F8FD3083640899A55013C3BBCDD91FEB2094B21CDFB6D78E0`.
  Host verifies the exact entry set and every file length/hash before extraction.
  The local verifier checks258 observations per five-second repetition and264
  in the35-second case, plus refusal/build/retirement receipts.
- Final audit2026-10-30T09:38:36+08:00 verifies all six exact owned tasks terminal,
  then unregisters them. No app/probe/build process or owned task remains at that
  snapshot. No external process is stopped. App/original probe/recorder/source
  binaries remain hash-identical; the installed driver remains Running with SYS
  SHA-256 `CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.
- All render volume/mute and default roles remain unchanged. DAWN alone is the
  audible destination; Realtek remains muted at volume0. No volume/default/clock
  writes or security changes occur. Retained Cargo target3407485343bytes;
  D free254964916224bytes and C free8130420736bytes. No cache clean occurs.

Raw evidence, verifier, summary and final audit are under
`target/physical-1921681297-20261029/`. The archive retains scripts, failed CLI
attempts, old/new native reports, full bounded startup traces and both build
receipts. Later virtual-route waveform attempts are separate evidence, not
part of this archive or a reason to rewrite its short physical-master result.
