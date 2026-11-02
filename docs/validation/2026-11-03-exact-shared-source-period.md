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

## Pending Native Boundary

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
remain separate required audits. Native results will be retained separately;
no success is claimed before the task is terminal and artifacts inspected.

First-packet investigation stays deferred. Native audio/UI stays remote-only;
the original27dirty/cache files remain outside the export and commit scope.
