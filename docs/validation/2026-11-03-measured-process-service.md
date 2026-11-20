# Native direct measured Process service replacement

Date: 2026-11-03, Asia/Shanghai. Clean baseline: `f00b9ea`.
This is original S3 service-level lifecycle evidence, not full S3 acceptance.
The direct API completes; delivery-bound, waveform, desktop, retained-Evaluate,
sustained and calibrated physical acceptance do not pass by implication.

## Scope and isolation

Local evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-measured-service-20261103-0150`.
Remote evidence: `D:\MoirenTest\20261103-measured-service-0150`.
One task `Moiren-MeasuredService-20261103-0150` uses the existing public
`AppRequest::ApplyMeasuredProcessBridge` while its predecessor is Running.
It reads fresh native evidence inside the service; the 1 Hz display snapshot
does not authorize admission. No retained Evaluate or desktop action occurs.

The graph is exactly Process -> unity Bus -> physical DAWN PRO2, three nodes
and two edges, 48 kHz/F32/Engine64. Shared period336 is requested and observed
through Client3 without fallback. Render target512/wake64/callback64 remains
unchanged. Compatible baseline has capacity8192/prime2048; the explicit measured
candidate has independent capacity16384/target2048/jitter96. No smaller target,
buffer-default promotion, admission-threshold or production timing change occurs.

The source uses the held Process PID/creation identity and a six-second,
288000-frame indexed-marker schedule. Independent baseline and replacement
observation windows are fixed at2000ms, deadline3000ms. They belong to this
different direct-API experiment; the prior retained attempt's400/900ms contract
is neither extended nor rerun. These are wall-clock observation windows, not
claims of two seconds of continuous recorded audio.

Native work is remote-only, with held source/app handles, worker150s/task240s
and app90s watchdog. Host work is compilation and pure/headless regression.
Default/help is inert; the evidence-only example requires explicit exercise
and absolute project/preferences/new-output paths. The exact clean archive
excludes all27 pre-existing dirty/cache files. Only the opt-in example and its
Cargo declaration overlay that archive; neither becomes a production entry point.

```text
source archive SHA256:
97498FB06316771922F8FEB26CFCDC5794573D2EB8327C92612D86BC095488E4
executed example source SHA256:
4D175048AFF29E2D905F456089E7429F541733D6DC980130C06B53AAC34077A2
process_measured.exe: 5,200,384 bytes, SHA256:
6A151A07484960B54CD9CC077A8BF34109E979A62FDB837ACF2C45B8EC242803
```

## Actual lifecycle result

Launch occurs once at01:59:05 +08:00. Worker observation spans
01:59:06.370-01:59:17.047, elapsed10677ms. Baseline and replacement observations
each take2001ms. Final Stop/join takes13ms. App, source, worker, supervisor and
scheduled task all exit0; service runtime joins normally, without forced cleanup.

The actual service sequence is:

- Baseline Running: app generation1, consumed request1, no measured selection.
- Replacement Running: app generation2, consumed request2, requested and active
  summaries exactly equal, selection1; predecessor cleanup belongs to generation1,
  Closed with no fault or cleanup error.
- The summary retains its original request: NodeId(0), app/native generation1,
  graph revision2 and capacity16384/target2048/jitter96. Source PID5044 and
  creation time134381159493002940 match the held source. `observed_window` is
  absent: this is not the separate live-prefix API.
- Final stopped app generation3, consumed request3, cleanup generation2 Closed,
  no fault/error, measured selections cleared; final service state is Exited.

Native owner generation1 appears in both hosts because Engine generations are
host-local. The new service generation2 and joined predecessor distinguish the
replacement; requiring a different native generation would be a runner error.

Actual replacement admission is Target16384/2048/96 with empirical 48 kHz
complete-drain maximum480 frames, maximum gap517 frames and prepared whole-wake
demand704. The original `request.json`, `preflight.json`, final native snapshots
and `report.json` retain these identities independently.

## Counters and limits

The joined replacement records90240 captured/written frames and88000 output
frames. Reported dropped, silent, nonfinite, discontinuity, timestamp-error,
underrun, reset, backlog/age/ingress discard and recovery counters are zero.
Startup is separate: predecessor priming4224, replacement priming3584, and
prime discard352 frames in each host. Captured minus output is not a loss or
delay estimate; no raw PCM is recorded by this experiment.

**The replacement has one delivery gap-bound violation.** Its188 packets and
187 gap samples retain maximum110414 in100ns units (11.0414ms), exceeding the
admitted empirical517-frame gap bound. Packet-bound violations are zero;
unavailable/regressed clocks are zero. The predecessor retains maximum107570
and zero gap violations. Zero reported audio-fault counters do not erase this
out-of-envelope observation or prove continuous waveform integrity. The service
receipt's `completed=true` is lifecycle completion, not a delivery-quality pass.
Do not enlarge the bound or repeatedly rerun this configuration to get a pass.

Replacement consumer history has272 total events and16 overwritten entries;
producer history has188 events and no overwrite. Bounded histories are not a
complete replay or PCM record. Final render padding is unavailable after
closure, not zero. The baseline live padding sample is merely diagnostic.
`direct_api_completed=true`, `retained_evaluate_attempted=false`,
`waveform_quality_verified=false` and `qualified=false` remain explicit.

## Acquisition and software follow-up

All38 compact files are individually length/SHA256-verified against the ZIP
and remote closed files; relative directories are preserved. Archive221721bytes:
`356D1FC04D08EA20ABD588A18E3EA49E12FD2E4C7160113DE9C4B71C9510EFC9`.
SSH stdout stalls after archive closure; its client is interrupted only after
independent task/owner inspection. Existing artifacts are fetched with SCP,
not regenerated and not a native rerun. A separate bounded closed-file verifier
confirms all38 entries. Final02:08:03 audit confirms task Ready, no owners,
unchanged protected OBS/QQMusic creation identities and running driver hash.
Worker before/after checks retain unchanged default roles and render volume/mute.

The executed example passes eight pure tests after real scope/host-local-generation
RED/GREEN, full Windows non-test strict Clippy, release build and isolated fmt.
Worker helpers pass86 checks on both PS5 and PS7, independently rerun by the
parent. Concurrent clean-source measured regression passes85 tests. These are
software checks, not waveform or physical acceptance.

The existing production software-host replacement test is strengthened for
both F32/F64: exact service-generation increment, original measured request,
absence of live-window metadata, predecessor cleanup, host-local native identity
and cleanup when clearing the profile. A duplicate-generation mutation in an
isolated source copy fails the new exact assertion; production source is not
mutated. Restored focused regression passes1 (covering both precisions); the
full app library with gpui-test passes671/3ignored. Strict app library/tests
Clippy with gpui-test and isolated workspace formatting pass. No new
full-workspace test pass is claimed for this test-only slice.

## Separate retained refusal and next boundary

The [prior retained attempt](2026-11-03-service-retained-prefix.md) still refuses
overlapping packet/read envelopes. Publication frontiers constrain acquired SPSC
visibility, not exact cross-writer chronology: a commit after the read's final
acquisition but before its ending timestamp, or later, can produce identical
frontiers. A compatible serialization cannot establish native historical order.
Keep that conservative refusal; no speculative RT ordering machinery is added.

This closes one direct service replacement question, not S1-S3. Lower-target
waveform tests, desktop acceptance, successful retained Evaluate/Apply, matched
route/clock evidence, sustained delivery and calibrated physical qualification
remain open. First-packet work stays deferred. Independent development can
continue with isolated remote tests when they answer a distinct useful question;
no unchanged long soak or new native retry is left running by this slice.
