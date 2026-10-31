# Both-endpoint minimum-period failure, 2026-10-31

## Scope and fixed conditions

This is a retained native RED result, not a qualified low-latency setting.
Virtual speaker / Playback is Moiren Input / Source; virtual microphone / MIC
is Moiren Output / Sink. The software route is Playback -> unity Bus -> MIC,
with no physical render endpoint. No driver installation, reboot, endpoint
volume/mute/default write, buffer enlargement or quality-guard change occurs.

The experiment reuses the frozen87c85ef release from the
[MIC-only acquisition](2026-10-31-microphone-minimum-period.md):7087616 bytes,
SHA256 `E38943A2114D1F0D9BC5B2D6EF1E11E204CF787D5B513064512A046768414116`.
It deliberately does not claim to be a rebuild after the later main merge.
Engine32, target1024/capacity4800/jitter96, declared packet/gap480, recovery8,
correctness3000ms, schema2 Exclusive recording and the original29-marker v1
schedule are unchanged. The only audio argv delta from MIC-only is adding
`--vad-minimum-period`; `--vad-microphone-minimum-period` remains selected.

The runner is15668 bytes, SHA256
`44D9380289FACF13F54531F69DA96D6189EB0E8DEBEEE735E97AAD040E4C5718`.
Its23 PowerShell fixtures,26 static equivalence checks and14 independent
PS5.1 checks pass. Those are runner checks, not native quality acceptance.
The host-load snapshot retains concurrent unrelated local work; this is not
a quiet-host or matched calibrated physical qualification.

## Native result

`both-minimum1` starts UTC04:10:48.6875606 and finishes04:10:58.8843040.
Probe PID12364 exits1. Collection, source-schedule completion, native cleanup,
quality_passed and quality_complete are false. The requested60-second run
ends early on a genuine operational identity failure. Correctness tolerance
does not exempt identity changes or terminal native cleanup faults.

Both endpoints actually use144 frames at48kHz stereo Float32. Playback and
MIC report default100000/minimum30000 hns and initial/final request30000 hns,
with no alignment retry. MIC additionally retains explicit query, first
Initialize and actual-buffer HRESULT0. The source DTO does not serialize a
raw first Initialize HRESULT: its started/actual-buffer observations must
not be relabeled as that missing field. A negotiated144-frame buffer alone
does not prove delivery cadence, continuity or end-to-end latency.

The first retained failure is `provider generation/gate/shape changed` at
approximately7.078 seconds of control progress. Comparing retained health
shows gate_epoch1 ->2 on both endpoints, while stream_generation remains1
and shape remains stereo Float32/144. The generic error therefore does not
establish an actual format or stream-generation change.

The last bounded startup sample has both endpoints Running, gate1 and no
provider quality fault, but is only the256th startup observation, about2.69
seconds after its first sample. It is not a sample immediately before failure.
At the first-failure read, Playback's provider snapshot has dropped0 and MIC
has silence1. Later retained Running snapshots have Playback dropped2 and
MIC silence4; final Removed/Closed snapshots have dropped3 and silence6.
Health, provider snapshots and logs are separate non-atomic observations.
Their different counters must not be collapsed into one simultaneous state
or used to infer which endpoint caused the reset.

stderr first logs MIC's Running/gate1 -> Running/gate2 with silence0 ->1,
then Playback's Running/gate1 -> Running/gate2 with dropped0 ->1. Four
`pcm_shutdown_terminal_fault` records then retain slots0..3, each
HRESULT `0x8007051A` (revision mismatch). This is observed log order, not a
native-event causal timeline. Cleanup remains failed even though the runner's
outer CleanupErrors array is empty and no owned process remains afterwards.

## Retained driver clock observation

Root subsequently decodes the original ETL locally using matching609 private-
PDB TMFs. The PDB SHA256 matches the frozen driver package manifest. Tracefmt
reports167 events,0 EventsLost,0 format errors and0 unknowns. These are tool
counters, not independently proven complete trace or waveform coverage.

Exactly two first-rebase fragment groups join on endpoint/session/generation/
stream/callback_qpc. Both have reason1 (EventDeadline), packet_frames144,
packet_count2, stream_generation1 and Running/open before and after. The
driver records each gate1 ->2 atomically with its clock invalidation under
Shared->Lock. This directly attributes the retained first gate reset on each
endpoint to a scheduling clock rebase rather than a format renegotiation.

MIC endpoint20 retains callback_qpc948068697612, next deadline948068692532
and5080 late ticks. Playback endpoint19 retains callback_qpc948068697665,
next deadline948068696161 and1504 late ticks. Frequency is10000000. Root
independently recomputes both next deadlines from the old clock and
`(completed_packets + 1) * packet_frames`. Thus the callback is0.5080ms and
0.1504ms past the NEXT boundary, respectively; relative to the completed
packet's boundary, lateness is3.5080ms and3.1504ms. These are scheduling
quantities, not audio-path latency or complete callback-arrival distributions.

This narrows the immediate mechanism but not why those callbacks missed the
schedule. The driver retains only the first rebase per generation. Passive
WPP emission time is not the retained callback time; close cross-endpoint
QPCs do not identify a common environmental cause. No scheduling tolerance,
identity guard, driver default or recovery policy is changed from this result.
The frozen receipt is `both-minimum1-clock-rebase-audit.json`, SHA256
`852C57D9FBA113A1ECC580FF7AC0176D70F7BCC28B24ADB9D514EE7418A442EF`.
Independent rechecking of the original ETL, package/PDB/TMF pins, fragment
identity, deadline arithmetic and fixed-version atomic recording passes46
checks with0 failures. The separate read-only review JSON SHA256 is
`0EB889CA7D64E6D7F577C59C87B8B0DF2DC5D88418E402E979BD9A867B605ED3`.

## Captured evidence and limits

The recorder stores2359 packets of144 frames:339696 frames,2717568 PCM
bytes. Stored flags are0, samples are finite, and packet-storage metadata
retains no sequence or device-position gaps. This bounded captured data does
not erase the independently observed provider drop/silence and gate faults.
Source acknowledged334080 frames with7168 nonzero frames, not the complete
schedule. No v1 marker identity or source-to-recorder origin is recovered.

Playback boundary evidence retains32 startup-prime muted frames,14 unknown
observations and final_after_join=false. Both native quality flags are false.
Final cleanup telemetry retains Playback reset/recovery1 and MIC2. Unknown
coverage, partial source progress and terminal errors are not converted to
zeros, ignored as teardown, or promoted to a qualified result.

Before/after render volume/mute/default roles and boot are unchanged. Driver
609 remains Running; root PnP is OK. No VAD child or probe owner remains after
cleanup. This establishes eventual resource retirement, not successful native
PCM shutdown. There is no physical output or calibrated latency evidence.

## Archive and next boundary

Host evidence root is
`D:\coding\moiren-evidence\20261031-mic-minimum-period`.
`both-minimum1-evidence.zip` is1883013 bytes,23 payloads plus manifest,
SHA256 `EEECACB084A02C296D0C6BD9D81E6487DAEEF843B138B08D5B3B2E21EDB5E962`.
Root checks CRCs, exact members, lengths, SHA256 and extracted bytes. An
independent audit passes38 archive/consistency checks with0 failed checks;
that audit verifies the RED result rather than changing it into a PASS.
Target originals and the earlier MIC-only result remain intact.

The oversized PowerShell failure-summary JSON is non-authoritative; use the
verified archive, compact root verification and independent audit instead.
The later matching-TMF decode and its explicit tool counters are separate
host-side derivatives; they do not rewrite the original archive. No underlying
scheduling root cause or source-minimum causal winner follows from this run.
Investigate callback service and admission before tuning another setting.
No512/256 scan or production-default promotion is admitted. S1-S3 and stable
calibrated physical sub-10-ms output remain open.
