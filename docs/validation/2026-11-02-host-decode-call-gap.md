# Retained Playback decode-call gap diagnostic

Date: 2026-11-02. Baseline: `6bc5397`. S1-S3 remain incomplete.

## Software change

The probe previously summed VAD quality counters without comparing retained
call-interval evidence with the experiment's declared gap. Zero in that existing
counter is not proof that the declared delivery bound was observed.

`live` report JSON now adds `host_decode_call_gap_diagnostic`. This is a
control-side projection of the existing last running Playback provider snapshot;
no new audio-thread instrumentation, production scheduling or queue policy is
introduced. It retains:

- Declared sample rate, packet/gap frames and declaration source, with jitter
  separate; Compatible has no declared bound.
- Original publication snapshot, numerical rounded-envelope comparison,
  evidence usability and explicit refusal reasons.
- Unknown `complete_delivery_coverage` and `carrier_publication_effect_known`,
  even when the observed rounded envelope is within the declared bound.

The measured interval is **previous successful decode-call before -> current
successful call after**, rounded UP to100ns. It includes intervening unsuccessful
calls, not atomic carrier publication, native packet arrival, source age or
physical delay. Exact `u128` cross multiplication avoids frame conversion
rounding/overflow. An exceeds result can include rounding across a fractional
frame boundary; it does not prove an actual supply gap. A within result does not
certify prefix/suffix, complete delivery, another owner or a future window.

Absent/insufficient evidence is unavailable. Identity mismatch, failed/missing
calls, segment/reopen/clock faults or saturation are inconclusive; their raw
maximum and numerical comparison remain visible. MIC and Process telemetry
cannot substitute for Playback publication evidence. Separate teardown snapshots
are not silently relabeled as running observations. Existing quality, exit and
qualification decisions are unchanged.

## Checks

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-host-decode-gap-20261102-1848`.
Builds use a clean `git archive 6bc5397` plus only the two intended Rust files,
excluding the preserved27dirty/cache files.

- Actual dry-run regression fails before implementation (`Null != 480`) and
  passes after wiring the new report field.
- Eight focused tests pass: exact480-frame equality/next100ns, fractional64-frame
  rounding, large-value arithmetic, insufficient/missing/faulted telemetry,
  owner/revision mismatch, non10MHz QPC units, failed calls and route separation.
- Full example suites, feature off and `probe-tools` on: each691passed/1ignored.
- Strict `cargo clippy --locked -p moiren-windows-audio --examples --tests
  --features probe-tools -- -D warnings`, workspace formatting and release build
  pass. An initial nonexistent `low-latency-probe` feature invocation failed
  before compilation; the corrected feature run is retained separately.
- One temporary clean-export-only test projects the actual retained Compatible
  report: no declared bound. A separately labeled counterfactual480-frame bound
  compares its106424x100nsmaximum (10.6424ms) against10ms and reports rounded
  envelope exceeds. This does not turn the old Compatible run into a target run.
- Independent read-only review finds no blocking issue. No default promotion,
  production fix, physical measurement or relaxed quality threshold follows.

## Bounded remote test attempt

The new executable is available on **the test machine only** at
`D:\MoirenTest\20261102-target1536-1900\e2e_latency_probe.exe`, SHA256
`47DC359B58CF4EF3D1B6994A9C42FA7B9B899BB15C0F50400838BDF228CBBCD0`.
It is a probe, not the desktop application or a driver package. Its source is
the clean baseline plus the exact two Rust overlays checked above.

A single new question was prepared:180seconds of continuously changing,
non-silent deterministic PCM through Playback -> unity Bus -> MIC, at48k,
Engine64/cap8, target1536, capacity4800, jitter96, declared packet480/gap512.
Gap512is an explicit assumption, not a certified observed supply bound. Native
quality is fail-fast (`latency-sensitive`), not the prior correctness-mode run.
The declared reserve arithmetic is1154frames; it does not promise native success.

The corrected pre-native dry-run succeeds. The original34-digit endpoint ID
attempt refuses before native work; both that error and the corrected32-digit
identities remain in evidence. At18:58:46.466-18:58:52.109+08:00the sole native
attempt **fails before source Start and before Engine/backend target admission**:
`VAD MMDevice has no exact Float32/PCM16 format`.

Both `IsFormatSupported` calls failed to yield exact `S_OK`; their HRESULTs were
not retained by the existing probe. Genuine unsupported format versus another
query failure/race is therefore unknown. MIC Shared recorder preparation
succeeded, but measurement never activated. Source submitted0frames, started
false, worker joined; Engine/admission absent. This is not a lower-target
stability result or an observed running `0x8007051A` recurrence.

`collection_complete`, `source_submitted_complete` and `cleanup_complete` remain
false. The probe exits1normally; the wrapper's completed flag only establishes
its process/audit completion, not functional collection success. No timeout or
forced kill occurs. Both exports are Removed/Closed, no owned process remains,
and the original bus, driver/boot and protected OBS/QQMusic creation identities
are unchanged. No rerun, format fallback or parameter scan follows this refusal.

Remote evidence: `D:\MoirenTest\20261102-target1536-1900`; host evidence has the
same suffix under the evidence root. The final compact56977-byte archive is
transferred, SHA256
`0540CD43450C71332C343FB631DBED6C9432BE1A7A58583CD83052BC25932999`;
36individual input artifacts are length/hash verified. An earlier57011-byte
collector archive was also transferred; no raw PCM or large recording is copied.
Earlier collector
assembly/quoting/self-manifest errors are retained, not native retries; the final
archive excludes its own manifest from the hashed input set. No PCM exists to
analyze or plot for this attempt. First-packet investigation remains deferred;
this bounded refusal does not justify blocking independent product development
or claiming any calibrated/sub10ms/path-winner result.
