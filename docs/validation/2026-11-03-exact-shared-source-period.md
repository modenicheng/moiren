# Exact owned-source Shared period

This original S1 experiment control extends `6ec4910`; it does not change
production defaults or establish matched waveform or physical latency.
Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-shared-source-period-20261103-0325`.

## Contract

`live-vad --vad-source-mode shared --source-shared-period-frames N` and the
owned Process source accept one decimal integer in64..4096. Exclusive VAD,
external-PID Process, missing, duplicate, signed or out-of-range requests
are refused before artifacts/native setup. Omission keeps the prior legacy
Shared initializer without a Client3 query or a claimed actual period.

Both owned sources use the same preparation-only adapter. Explicit requests
query `IAudioClient3::GetSharedModeEnginePeriod`, check bounds and fundamental
multiple without rounding/clamping, initialize once, then read the current
engine period and descriptor. Unknown, malformed, mismatched or locked results
fail without changing mode, format or period. Buffer capacity still comes from
`GetBufferSize`; it is not relabeled as period or latency.

Requested/mix/client descriptors, caps, successful initialization API and raw
engine period/rate remain separate evidence. Failed negotiation survives the
joined partial VAD outcome before Ready. A COM readback failure retains its
actual HRESULT; a locally rejected descriptor retains the returned raw frame
count and diagnostic, not a synthetic native HRESULT or invented normalized
period. Supported mismatched engine descriptors are retained in the failure
message. The native descriptor allocation is released on success and failure.

Process parent/worker command and all receipt phases pin the optional request.
Explicit Ready and Final require exact requested/actual/raw engine frames,
48k stereo mix/client/native encoding agreement and no conversion/fallback or
failure. Final also matches the joined negotiation attempt. Omitted fields
remain absent and legacy fixtures remain valid. Dry-run reports request only;
it opens no child/native source and creates no PCM.

Matching one period request does not match actual source capacity, encoding,
delivery, clock alignment, render demand, waveform quality or physical delay.
No source-path winner, smaller default or S1-S3 completion follows.

## Verification Status

Parser and exact-initializer tests first fail behaviorally on the old path;
Process tests separately fail for missing Ready readback and incomplete Final
evidence. Compile-only failures are retained but not counted as behavioral RED.
Initial combined16tests pass. Independent review then catches local descriptor
errors being relabeled as native HRESULTs; the final implementation separates
those failures and adds a dedicated regression. A narrowly justified Clippy
expectation preserves explicit preparation/peer-stop arguments without a new
configuration framework.

The final clean export passes733probe tests/1ignored with `probe-tools`, and
the same733/1 with `probe-tools,vad-probe`. The final workspace with
`moiren-app/gpui-test,moiren-windows-audio/vad-probe` passes2356tests/9ignored
across71harnesses. All-target workspace check, strict Clippy, workspace fmt
and the release probe build pass. The final binary SHA256 is
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
The earlier full checks and actual Clippy failure are retained separately as
pre-review results, not silently overwritten with passing exits.

Independent code review finds no remaining blocker after the descriptor fix.
No host native audio/UI or process manipulation occurs. A remote read-only
preflight finds no test owners or running Moiren task, only the healthy bare
control bus, and unchanged protected OBS/QQMusic creation identities.

The three originally requested focused checks are also rerun on the exact
export while the remote native experiment runs: `--lib zero_refill` passes5;
the `latency_marker`, `latency_marker_rate` and `latency_qualification` suites
pass19+8+24=51; `moiren-app --features gpui-test measured_ui_tests` passes20.
These are software/headless checks, not device-latency acceptance.

## Native Contract

The reviewed evidence-only runner defines one120s VAD source480 request/readback
and lifecycle experiment, not an unchanged soak. Source480 is a hypothesis;
VAD caps, observed actual period and capacity remain unknown until acquisition.
There is no alternate-period retry, Process stage, input buffer override, MIC,
raw recorder or physical calibration. Dry-run selfcheck must pass with matching
intent before native acquisition. The finite task uses a held child180s and
outer300s deadline, Limited Interactive session1, isolated environment and
new paths. Twenty pure PS5 checks pass; identity conversion uses a non-symmetric
Engine-byte/ABI-display regression rather than comparing incompatible strings.

The existing hard SRC stream32GiB, summary192MiB, journal128MiB and log16MiB
limits are accepted with a fresh40GiB disk guard (remote read-only free space
is151064764416bytes). The200MiB post-join artifact check is not a live disk cap.
Source/runtime pins, owner closure, exact endpoint identity, unchanged defaults,
render controls, protected processes, loaded driver/boot and removed exports
remain separate required audits.

## Actual Remote Result

The source implementation is committed as `50c97e5`; all1383other tracked files
in its archive match the tested export exactly (only this new validation doc
was excluded from that code comparison). The package is pinned to that commit
and the release hash above at
`D:\MoirenTest\20261103-shared-source-period-0325`.

Selfcheck worker runs03:53:01-03:53:19 +08:00, audit complete with native absent.
The single native worker runs03:54:15-03:56:41; its child PID16160 runs
03:54:23.778-03:56:30.869,127.088seconds including setup/cleanup. Its native
source runs120.009767seconds. The probe exits1/unqualified/child_failed;
the outer worker/task exit0 means complete audited closure, not a quality PASS.
There is no timeout, forced termination or retry, and no Process second stage.

The VAD source returns Client3 caps default=fundamental=min=max480. Requested,
raw engine and client-rate actual periods are480 at48k, exact stereo F32,
without fallback/conversion/readback failure. Native buffer capacity remains
1056, not480. The source acknowledges all5664000scheduled frames; total native
submissions are5761536 with1056silent preroll. It reports zero source event
timeouts and normal native Stop/join. Submission is not delivered waveform.

The physical DAWN master remains actual336/capacity738/queue512 with published
whole-wake demand704; no input target/default changes. The host closes without
fault or cleanup error, owner generation1 agrees, export is Removed/Closed,
and no test owner remains. Protected OBS/QQMusic creation identities, default
roles, pinned render format/volume/mute, driver hash and boot are unchanged.

**The run is not zero-fault quality evidence.** Running provider telemetry
retains carrier drop1 and epoch reset1; its snapshot separately reports4dropped
packets. Joined cleanup telemetry retains epoch reset2/recovery1. Joined
physical diagnostics retain17native wait timeouts and5758016written frames,
with zero reported native drop/silence counters. These distinct scopes are not
collapsed, erased or used to assign causality.

The actual scalar history persists89970sequential callbacks in44chunks,
67124813bytes, history complete with zero missing/unretained callbacks.
Completeness is not clean audio. Remote streaming analysis retains two
Engine-frame-axis event intervals:

| Interval | Engine Seconds | Observations |
| --- | --- | --- |
| Initial | 0-0.138667 | 103xrun/discontinuous/short-transfer callbacks,32explicit edge-muted frames |
| Running | 79.112-79.184 | 53xrun/discontinuous/short-transfer callbacks, SRC/boundary generation change,32explicit edge-muted frames |

The53callbacks are not53independent resets. The running interval cannot be
reclassified as first-packet behavior. There is no recorder or calibrated
source/common physical clock; scalar flags do not locate exact acoustic
damage, establish a root cause, or prove physical latency. No new unchanged
soak, buffer enlargement, gate weakening or speculative RT fix follows.

## Retained Visualization

Analysis executes on the remote machine using only the Python standard library;
its pure synthetic sequence/event-grouping regression passes first. Only
38compact metadata/log artifacts are downloaded and individually length/SHA256
verified:109605-byte archive SHA256
`F0A5BD22B58CD6476C5B7D6212A55FD305D01D028F7B101A07255E5759626155`.
The67MBscalar stream stays remote, and no raw PCM is transferred.

Remote `source-period-summary.json` and `source-period-diagnostics.svg` retain
the source period/capacity distinction, Engine-axis event intervals and
per-second processing-step range. Both compact outputs are length/hash verified;
the SVG is visually inspected via a local headless rendering, not a native
Moiren/audio/UI run. Paths:

```text
remote: D:\MoirenTest\20261103-shared-source-period-0325\source-period-diagnostics.svg
local:  <evidence root>\captured\source-period-diagnostics.svg
local:  <evidence root>\captured\source-period-diagnostics.png
```

Independent evidence review confirms the interpretation above and finds no
reproduced production root cause requiring an immediate code change. Preserve
the failed quality boundary, keep Compatible defaults, and continue independent
development rather than repeating this stability scan. Matched source-buffer/
waveform evidence, lower-setting acceptance and calibrated physical latency
remain open; lack of physical recording equipment does not block that work.

First-packet investigation stays deferred. Native audio/UI stays remote-only;
the original27dirty/cache files remain outside the export and commit scope.
