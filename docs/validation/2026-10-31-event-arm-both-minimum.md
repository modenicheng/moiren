# Both-minimum stable-binding control - 2026-10-31

## Outcome and experiment boundary

**Playback144/MIC144 collection and cleanup complete; quality acceptance does
not.** This separately frozen control changes only the source request from
the [stable-binding control](2026-10-31-event-arm-stable-binding.md): add
`--vad-minimum-period`. The MIC minimum request, installed611 binding, frozen
probe, indexed source, graph and buffer/WPP policy remain unchanged. Both native
buffers actually negotiate144 frames. Nominal offline analysis still refuses
the complete identity set:14 unique identities and15 missing. No new rebase or
preceding-arm witness is acquired. This is neither a timer fix nor a device
floor, stable sub-10-ms result or physical acceptance.

Host evidence: `D:\coding\moiren-evidence\20261031-event-arm-both-minimum`.
Target evidence: `D:\MoirenTest\20261031-event-arm-both-minimum` on
`LAPTOP-NP49RG6H`. Run `event-arm-both1`; exact task
`Moiren-EventArmBoth-waveform1`; owned trace
`MoirenVAD-20261031-EventArmBoth1`.

The new `hypothesis.json` freezes the sole audio-argv delta and retains both
predecessors: the successful but unqualified stable-binding control and the
[older609 both-minimum failure](2026-10-31-both-minimum-period.md). This request
does not replay either run. There is no driver install/restart, native retry,
buffer enlargement, default promotion or quality/epoch-policy relaxation.

## Frozen provenance and route

- Probe source `99685052997e971c9aacf4cc19c20765ad1f8632`,7437312 bytes,
  SHA256 `69BB890BDBEADF2F23B994EA70AFDAC225208BD47455CB63746CA83464EB0C77`.
  It intentionally excludes the later0ef5a29 final retired-QUERY repair; this
  run is not native verification of that repair.
- Installed611/oem205 driver source `48ecf086941da9ba7d6c90abe48d2cdc2c1c3bee`,
  signed SYS177048 bytes, SHA256
  `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
  The preceding-arm instrumentation adds two QPC reads per rearm and is not
  perturbation-free. Installed identity is not in-memory attestation.
- Exact software route: Playback Input/Source -> unity Bus -> MIC Output/Sink,
  48kHz stereo, Engine32, target1024/capacity4800/jitter96, declared maximum
  packet/gap480 and recovery8. Both endpoints use Exclusive queried minimum;
  requested and observed periods are retained separately. No physical renderer
  is selected and no speaker audio is emitted by this experiment.
- Unchanged29-marker indexed-v2 source:1024 frames per marker,30 seconds,
  1440000 scheduled frames. Correctness grace3000ms and ceiling60s remain.
  WPP stays0xf/level4/perf/sequential16MiB.
- Runner SHA256
  `59FF94572AF506D5A0ACEEF1F747D749919DD93BCADB93E05A6DCA4A3BBF01F3`;
  freeze SHA256
  `1ADAA131EC1762FD941D3A9DEA311F6DE93A8F888E4B8625FD8C212B9B4D4F6A`.
  Actual local inert fixture receipts retain runner55, WPP21, extracted
  Invoke/Read/Wait8 and archive54 passing checks under Windows PowerShell5.1.
  The runner validates the source minimum flag at
  `native.vad_playback.period_request.minimum_period`; it does not invent that
  field in the separate source request DTO.

The sole Interactive/Limited task request is at14:41:05.6318130UTC. Early
observations retain a Running task without a probe; the same task eventually
executes, without a replacement request. The outer runner starts at
14:43:06.0326585UTC; probe PID17696 is recorded at14:43:09.3605271UTC and the
final receipt at14:43:45.2306877UTC. Probe/task/controller/wrapper exit0.
The owned trace starts at14:41:04.1494777UTC and stops0 at
14:43:47.4850427UTC. Target timestamps are not a host/target clock calibration.

## Native result, not delivered-waveform proof

- Playback and MIC query default100000/minimum30000hns. The source final
  request is30000hns, and its actual native buffer is144. MIC query,
  Initialize and actual-buffer HRESULTs are0, actual buffer144, without
  alignment retry. Both workers start, stop and join without a reported native
  failure; collection and runtime cleanup complete.
- Source Release-acknowledges1440000/1440000 scheduled frames and29696
  nonzero frames. Native submitted1450512 includes144 preroll frames and tail.
  Release acknowledgement does not establish playback/capture of every marker.
- Recorder retains10111 complete144-frame packets /1455984 frames /
  11647872 PCM bytes. Evidence metadata reports zero drops, silent flags,
  overflow, gaps, discontinuities and timestamp-error packets. Retained packet
  sequence/storage coverage is complete, all flags0, and stored PCM is finite.
  Both stereo channels match;31473 frames contain a nonzero channel0 sample.
  Zero signal values are not a native silence flag, and duplicate stereo is not
  independent reference evidence.
- Last Running/open snapshots have reset/recovery0/0 for both directions.
  Final Removed snapshots retain Playback submitted/completed10074/10070 and
  MIC10111/10111, reset/recovery1/1 in each direction and reported
  drop/silence0. Lifecycle transitions and the Playback completion difference
  remain visible, not relabelled as whole-run fault-free delivery.
- Boundary initial prime is32; steady soft-event/xrun/recovery-prime/short-read
  deltas are0. Fourteen observations remain unknown. `QualityPassed=true`
  does not override `QualityComplete=false`, incomplete boundary evidence or
  `qualified=false`. Generic recorder `delivery.packets=0` is unpopulated on
  this path, not a contradiction of10111 retained evidence packets.

## Offline identity refusal

Only the already archived PCM is analyzed; no new native acquisition occurs.
PCM SHA256:
`93A50AF454280896A515FB988535DE43CB00C636DA5BE0FFF36E7916F6DD7F70`.
The predecessor's support heuristic proposes29 unlabelled channel0 windows:
nonzero support separated by more than32 zero frames, padded64 frames each
side. Every window competes all29 original codes. No identity, source origin
or order is inferred from support order. Thresholds remain correlation0.7,
margin0.05 and RMS0.0001; charged work163862528 is below256000000.

The frozen9968505 analyzer exits1 with14 unique identities,15 missing and no
duplicate; `identity_set_complete=false`. All15 missing best candidates fail
section coverage; four also fail global correlation. These are classification
failures under the unchanged nominal-rate/no-resampling model, not15 proved
lost source markers. The [predecessor's one-window transformation diagnostic](2026-10-31-stable-binding-src-window0.md)
does not establish a transformation or repair for this different recording.

Stored packet coverage is complete, but9686 native-QPC intervals violate the
existing nominal-rate diagnostic policy. They are not9686 lost frames, packet
gaps, calibrated clock drift or physical delay. Source-to-recorder origin and
native epoch remain unknown. A14/29 versus13/29 classification count is not a
qualified quality or latency improvement.

Config SHA256:
`2D11D620F0812AD89EA613358950509F966BC00CDDE48D1711B2D93EA6C9D17A`.
Diagnostic JSON SHA256:
`480A9DAF36C8766D49352D12E1B0C0F27E5C8E80E49DD6F5C2E8A93EB19111A1`.
Analysis receipt SHA256:
`1E9D6596F5799A90E8CE878045F4D85899456D0EB9C7D4C30714EAA4771DC3A4`.
The create-new proposal/config/argv/input pins/stdout/stderr remain under
`root-indexed-analysis`; its failure is retained unchanged.

## Archive, decode and cleanup

The44-payload archive plus manifest is2187324 bytes, SHA256
`CDB1473C42B4A689768EA7EF7610626D715C7119186D6B5CFC8AB89FE69FD107`.
Manifest11750 bytes, SHA256
`AC78C4122AA00284D7AA3A3EC517306E8758F18B3626C8DAEE7E2FDC0DECA0F1`.
Closed ETL65536 bytes, SHA256
`E64C8FD04E83DC46AFC11A40802127EBB44AD46390D3B93ADC5687CA6F3BAAF2`.
Target/root provenance verification and independent ZIP/extracted payload
byte/hash checks pass before and after cleanup.

Matching611 TMFs decode159 events with reported lost/format/unknown0; raw
trace-loss availability remains unknown. No matched rebase/no-copy witness
exists. Schema2 exits1 with `missing rebase/no-copy witnesses`, creates no
successful association and retains `root-both-decode/association-result.json`.
The existing15 parser regressions pass. No fault rerun or timer change is
justified by an absent witness.

The identity/pin-adapted exact-task cleanup helper passes50 guard and4 actual
extracted-flow PS5.1 inert checks using this run's terminal snapshot. Default
read-only guards pass before one unregister of the exact unchanged terminal
task. No raw evidence, failure, cache, driver package or rollback is removed.
At14:47:47.4748809UTC the independent post-audit has no task, owned trace,
build/audio owner or raw child. Driver611 remains Running/root OK; boot stays
`2026-10-30T09:50:49.6598450+08:00`. All six default roles and twelve render
master volume/mute controls remain unchanged; no privacy, volume, role or reboot
write occurs.

## Remaining work

This adds a native144/144 operational point under the current611 binding to
the S2 period matrix. It does not erase the older609 failure or establish the
cause of different outcomes. Retain these frozen recordings for bounded
transformation-versus-loss analysis rather than replaying acquisitions or
weakening identity/quality gates. The real preceding-arm/rebase association,
native verification of the final-QUERY repair, sustained lower-budget
delivery, matched VAD/Process comparison and calibrated physical acceptance
remain open. S1-S3 are not complete.
