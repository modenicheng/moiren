# Native VAD packet-period and Engine-block matrix

Date: 2026-10-29 on the development host. Physical target:
`LAPTOP-NP49RG6H` / `192.168.12.97`. Target wall time remains unchanged;
cross-machine wall-clock subtraction is not a latency measurement.

This bounded S2 follow-up uses deployed probe `5382369`, application
`e195bfe` and the existing installed driver. It neither compiles nor cleans
caches, reinstalls the driver, changes defaults/volume/mute, or changes a
production buffer default. The [cache retention policy](2026-10-29-remote-build-cache-policy.md)
continues to apply.

## Exact route and independent settings

Route: `VAD playback -> Bus -> VAD microphone`, 48 kHz, with an owned native
microphone recorder. There is no physical render master or Gain node.
The source schedule is 240,000 frames; correctness mode permits 3000 ms of
extra progress grace within the unchanged 30-second collection ceiling.
Grace is neither audio latency allowance nor a larger production buffer.

Both directions explicitly request capacity4800, target1024, jitter96,
declared maximum packet/gap480 and recovery cap1. No setting falls back or
silently grows. The two playback source modes are:

- Default Exclusive: no period override; actual native source buffer and
  playback provider packet are480 frames.
- Minimum Exclusive: query default100,000/minimum30,000 hns, request30,000 hns,
  no alignment retry; actual buffer and playback packet are144 frames.

The microphone provider and Shared recorder packets remain480 frames in
every case. These are not requests for a128-frame native period, evidence
that48-frame packets are accepted, or observations of physical render support.

| Engine block | Admitted burst | Native transfer | Low water | Modeled minimum |
| --- | --- | --- | --- | --- |
| 128 | 128 | 129 | 161 | 737 |
| 256 | 256 | 257 | 289 | 865 |

Both directions retain FIR lookahead32, wake allowance96, prime/reference1024,
high water4320 and capacity4800. Preparation's `native_packet_frames=null`
stays unknown: later observed144/480 packet shapes do not retroactively turn
it into a preparation-time measurement. Period, Engine block, capacity,
operating target and physical end-to-end delay are distinct quantities.

## Original native outcomes

Case names share the prefix `vad-period-block1-`. Every process terminates
without watchdog expiry.

| Case suffix | Exit | Collection / cleanup complete | Source acknowledged | Recorder frames |
| --- | --- | --- | --- | --- |
| `minimum-b128-t1024` | 0 | true / true | 240,000 | 256,320 |
| `minimum-b256-t1024` | 0 | true / true | 240,000 | 255,840 |
| `default-b256-t1024` | 0 | true / true | 240,000 | 255,840 |
| `default-b128-t1024` | 1 | false / false | 87,840 | 93,600 |

All four retain `quality_passed=false` and `qualified=false`. Each recorder
has one discontinuity: first packet flags1, device position480, stored start0.
Later packet adjacency mismatches, gaps, dropped frames, overflow and timestamp
errors are zero. This does not prove an intact startup prefix, exactly480 lost
frames, or lossless continuous PCM; there is no first-packet exemption.

`default-b128` independently retains one microphone driver dropped packet and
two silence packets in the running snapshot. Telemetry's silence-packet count
is0; the raw views are retained separately rather than rewritten to agree.
The other three running provider snapshots have zero drop/silence counts.
All cases retain playback initial prime mute32, zero classified steady deltas,
zero recovery-prime/short-read counts and final boundary xruns0. These facts
do not cancel recorder/provider quality failures. Teardown histories include
recovery/epoch transitions and later microphone silence; they are not relabeled
as steady-state zero-fault evidence.

## Premature termination diagnosis

The original `default-b128` first failure says the graph is not Running.
Its same retained Engine observation instead has executionRunning, active/plan
revision1, owner/resource generations1/0, no fault or cleanup error, and a
null timeline. Native owners are also Running. Using zero-based array indices,
startup `samples[183]` has frame93184; `samples[184]` and `samples[185]` have
an unavailable timeline.

The probe's original `VadCleanup::current_health` requires both executionRunning
and an available positive timeline to set `engine_running=true`. However,
`VirtualOwner::Published::snapshot` deliberately attempts one seqlock read and
may returnNone during publication. The Engine supervisor publishes that optional
observation independently of confirmed Running state. Thus an unavailable
progress observation is incorrectly classified as stopped execution. The
archive does not prove whether this exact read saw an odd sequence, a changed
sequence, or another timeline-unavailability path.

Cleanup repeats this predicate and retains its failure, so original aggregate
`cleanup_complete=false` stays false. Separately, both exports becomeRemoved,
the Engine confirmsClosed without cleanup error, and the recorder/source stop
and join. Do not overwrite the original failed receipt or infer that this probe
classification caused the independent driver drop/silence.

## Evidence and verdict

Archive: `evidence-vad-period-block-5382369.zip`, 1,044,252 bytes,
SHA-256 `707EBC3CC2221CCA60F01F32B56843815107156979345DC7086FC2773D1371E4`.
All51 member files /52 entries, lengths and SHA-256 hashes are verified on the
host. The archive includes original reports, PCM/packet metadata, source and
collection settings, case/runner manifests, binary/driver identities and cache
policy. Its convenience PowerShell summary truncates deep nested objects;
the original per-case `report.json` owns full first-failure evidence.

This establishes a bounded native software-route144/480 packet by128/256
Engine-block matrix at explicit target1024: three operation/cleanup completions,
four quality failures, and one diagnosed probe lifecycle/progress conflation.
It does not establish calibrated physical delay, a VAD/Process latency winner,
a stable lower default, naming propagation under held clients, or completion
of S1-S3. A common physical reference/calibration and a usable unmuted output
setup remain unprovided; no system setting is changed to manufacture a pass.
