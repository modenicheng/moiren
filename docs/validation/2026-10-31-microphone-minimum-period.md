# Explicit owned-MIC minimum-period control, 2026-10-31

## Scope

`87c85ef` integrates independently developed `488cd11`. The latency probe
accepts `--vad-microphone-minimum-period` only for its owned microphone route
with schema2 Exclusive collection. It is separate from the existing Playback
source flag `--vad-minimum-period`. Omission preserves the default request and
omits the new optional serialized receipt. No production buffer, driver,
provider, recovery cap, quality guard or validation classification changes.

The collection's existing native capture path now requests the queried
Exclusive minimum when explicitly selected. Minimal library plumbing adds an
optional WasapiOptions receipt handle, a by-value stream receipt and an
optional CaptureReport field. Other construction sites pass None. Shared,
output and explicit-period combinations refuse this option before COM setup.

Evidence retains raw default/minimum queries, query HRESULT, exact durations
passed at the first/retry Initialize call boundaries, separate HRESULTs,
alignment attempt/query/frames and actual buffer/query result. Query success
does not pretend Initialize occurred. Alignment preparation does not pretend
a fresh-client retry occurred. Failed preparation copies partial evidence into
the capture report. No fallback to the default minimum request occurs.

The handle's mutex is accessed only during preparation. It is not held across
COM calls and never accessed by the capture loop; the opened stream keeps a
value copy. It is neither audio-thread synchronization nor a delivery probe.

## Verification

The author retains genuine parser/dry-run RED, then GREEN evidence under
`D:\coding\moiren-evidence\20261031-probe-mic-period-control`. Audio library
tests pass657 with2 ignored, standalone probe tests548, strict Windows and
Linux-target Clippy and Linux lib/tests/examples compilation pass. Formatting
passes after correcting an initial formatting failure. Initial compile/CLI/
Clippy failures remain in the logs rather than being presented as successes.

Independent review identifies and closes two evidence bugs before integration:
the receipt previously rounded duration differently from the actual native
call, and query/alignment preparation previously populated unexecuted request
fields. Regressions cover256 frames/48kHz rounding, unexecuted stages, separate
first/retry results, failed queries, no mutex held during Initialize, invalid
option combinations and noflag serialization. Final read-only review finds no
blocking issue. Root then runs the combined marker-v2/MIC-period probe555/555
and14 focused library period tests: all pass. Root logs and subsequent build
evidence live under `D:\coding\moiren-evidence\20261031-mic-minimum-period`.

## Native experiment boundary

The first new experiment must retain the previous Playback default and source
v1 schedule, Engine32, target1024/capacity4800/jitter96, declared packet/gap480
and recovery cap8. Only the MIC minimum request changes. The independently
reviewed adapted runner preserves the operational aggregate and raw quality/
loss counters. Its58 reused control fixtures,42 period-request fixtures and8
adaptation invariants pass; root independently reruns the42 fixtures. No
512/256 scan or default promotion is justified by these software checks.

A negotiated minimum or buffer size is not actual delivery cadence, waveform
quality or latency. Partial failures remain useful evidence, not a pass.
Previous MIC tail silence/reset/recovery evidence is not erased by this new
control. S1-S3 and calibrated stable physical sub-10-ms acceptance remain open.

## First controlled native MIC-only acquisition

Root builds the integrated release at87c85ef with jobs2/worktree-local target:
7087616 bytes, SHA256
`E38943A2114D1F0D9BC5B2D6EF1E11E204CF787D5B513064512A046768414116`.
It is staged at
`D:\MoirenTest\20261031-mic-minimum-period\bin\e2e_latency_probe.exe`.
The frozen runner is15386 bytes, SHA256
`47E51BF94264C59952CC8D07C9416F41B13325C2829AAC277FC985EF38F6FD56`.
An independent implementation-aware review and8 additional PS5.1 fixture
checks find no blocker. This runner retains its control's unbounded child
WaitForExit: an outer timeout is not a terminal result and does not kill audio.
Root waits for actual terminal evidence, not a controller deadline.

`mic-minimum-only1` starts UTC04:00:39.0613402 and finishes04:01:14.8059434.
Probe PID19088 exits0, collection and cleanup complete, failures null, exact
source schedule submitted complete. The operational aggregate passes. Source
Playback remains default480 frames; only MIC requests its queried minimum.
MIC queries default100000/minimum30000 hns, query/first Initialize/actual
buffer HRESULTs are0, first/final request30000 hns, no alignment/retry, actual
buffer144 frames at48kHz stereo Float32. The actual request is therefore
confirmed, not assumed from an argv switch.

Recorder stores10110 complete144-frame packets,1455840 frames and11646720
PCM bytes. All packet flags are0; metadata retains no missing/incomplete
packets, sequence/position gaps, overflow, timestamp errors or nonfinite
samples. Recorded QPC differences have10109 samples, min21449, p50 nearest-
rank29433, p9533833, p9934665 and max35450 in100ns units, with no nonpositive
difference. These are packet timestamp intervals, not host callback arrival
times or end-to-end latency. The capture report's default `delivery` object
has0 observations and must not be described as measured zero-gap delivery.

PCM has31472 nonzero frames/62944 nonzero scalars, peak0.0476635620 and
bit-identical stereo. These counts do not recover original v1 marker identity
or prove continuity through intentionally silent intervals.

Quality remains incomplete. Playback retains32 startup-prime muted frames,
zero classified steady delta and15 unknown observations. The MIC active
snapshot has silence_packets0 but the final Removed cleanup snapshot again
has1; its separate telemetry silence count remains0. Both final cleanup
telemetries retain epoch_resets1/recovery_events1. The tail timing/cause is
not resolved and is not silently excluded. Existing `quality_passed=true`
does not override explicit `quality_complete=false` or revalidate cleanup.

Before/after endpoint snapshots retain equal render volume/mute/default roles;
no owner or VAD child remains, root PnP is OK, driver609 stays Running and boot
is unchanged. No physical output, driver install, default/volume write or reboot
occurs. No512/256 target experiment or default promotion follows this receipt.

Archive `mic-minimum-only1-evidence.zip` is1234231 bytes,22 payloads plus
manifest, SHA256
`10ED929ACDE676E868AB7529A94D57E45F0B725E4F4EF9C6D56ECCDF7EAE10AC`.
Root verifies CRCs, exact members, manifest lengths/SHA256 and extracted bytes.
Target originals remain. This closes the missing explicit MIC native request
control/evidence gap, not S1-S3, complete waveform quality or physical latency.

Independent archive/PCM/metadata audit passes43 checks and confirms the same
request, native periods, packet/frame totals and retained cleanup faults. Audit
JSON SHA256 is
`3245FA29133E00581A1679913DBC46490DFE540068C174CD110FAE8C535D54D5`.
Provider completion intervals have different provenance from recorder packet
timestamps: cleanup MIC max36498 hns, Playback105950 hns. Neither is treated
as physical end-to-end latency or a future delivery guarantee.
