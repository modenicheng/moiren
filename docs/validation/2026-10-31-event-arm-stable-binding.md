# Event-arm stable-binding control - 2026-10-31

## Outcome and experiment boundary

**Native collection and cleanup complete; waveform/quality acceptance does not.**
This separately frozen control keeps driver611 and the exact9968505 probe/audio
configuration from the [post-install setup failure](2026-10-31-event-arm-native-setup-failure.md),
but does not install or restart anything. MIC preparation now succeeds with an
observed144-frame buffer. The source is fully Release-acknowledged and the
recorder retains10112 packets. Offline identity analysis still exits1 with
13 unique identities and16 missing; quality remains incomplete. No rebase or
preceding-arm witness is acquired. Neither a scheduling fix nor a stable device
floor, sub-10-ms impossibility or physical latency follows.

Host evidence: `D:\coding\moiren-evidence\20261031-event-arm-stable-binding`.
Target evidence: `D:\MoirenTest\20261031-event-arm-stable-binding` on
`LAPTOP-NP49RG6H`. Run `event-arm-stable1`; exact task
`Moiren-EventArmStable-waveform1`; owned trace
`MoirenVAD-20261031-EventArmStable1`.

The retained `hypothesis.json` distinguishes already-bound611 child/MMDevice
state from first use after upgrade. This is one new request under new frozen
paths, not replay of the failed `event-arm-boundary1`. Successful preparation
supports keeping binding state as a discriminating condition; it does not prove
the unique cause of the predecessor's SetEventHandle0x88890004. No production
timer, buffer, epoch safety or quality policy changes are made.

## Frozen provenance and unchanged route

- Probe source `99685052997e971c9aacf4cc19c20765ad1f8632`,7437312 bytes,
  SHA256 `69BB890BDBEADF2F23B994EA70AFDAC225208BD47455CB63746CA83464EB0C77`.
  This intentionally excludes the later0ef5a29 final retired-query repair.
- Installed611/oem205 driver source `48ecf086941da9ba7d6c90abe48d2cdc2c1c3bee`,
  signed SYS177048 bytes, SHA256
  `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
  Its preceding-arm instrumentation adds two QPC reads per event arm; this
  is not perturbation-free observation. Installed identity is not in-memory
  attestation. Package/PDB/rollback pins remain in the predecessor receipt.
- Exact native argv and indexed input bytes are compared independently:
  Playback Exclusive480, MIC-only Exclusive queried minimum,48kHz stereo,
  Input/Source -> unity Bus -> MIC Output/Sink, Engine32,
  target1024/capacity4800/jitter96, maximum packet/gap480, recovery8.
- The29 unique1024-frame indexed-v2 markers cover a30-second source schedule
  of1440000 frames. Correctness grace stays3000ms; requested ceiling60s.
  WPP stays0xf/level4/perf/sequential16MiB. No physical renderer is selected.
- Runner SHA256
  `48B0595CAB9E93E7AC426424E7E3E5FE69EEC739C43701F57F9F02658C5E6656`;
  freeze SHA256
  `CDC9ADA3D1DAC8AEFA25E5CC125E33EDD7E0032E26745E6AD0F30EE9A8E8C7CE`.
  Runner48, WPP21, actual Invoke/Read/Wait8 and archive54 inert checks pass.
  The existing explicit function capture before `GetNewClosure()` is retained.

Only the exact Interactive/Limited task runs audio; the administrative wrapper
owns WPP. Delayed task execution is observed without a replacement launch.
Trace start is14:09:21.9418686UTC; probe PID16176 launches at
14:11:27.1331222UTC and its final receipt is14:12:03.0669214UTC.
Probe/task/controller/wrapper exit0; the owned trace stops0 at
14:12:05.0594736UTC. These are retained target timestamps, not a merged
host/target clock calibration.

## Acquired native result, not a quality pass

- MIC default100000/minimum30000hns query, first Initialize and actual buffer
  query return0. No alignment retry occurs; actual buffer144,48kHz/stereo.
  Recorder Start/Stop/join succeed; no native or cleanup failure is reported.
- Source acknowledges1440000/1440000 scheduled frames and29696 nonzero
  frames. Native submission totals1451040 including preroll/tail; native
  Playback buffer480. Release acknowledgement is not delivered-waveform proof.
- Recorder stores1456128 frames/10112 complete packets,11649024 PCM bytes.
  Evidence metadata reports zero drops, silence flags, gaps, discontinuities,
  timestamp-error packets, sequence gaps and nonfinite samples. Actual stored
  zero-valued signal intervals are not the same as a native silence flag.
- Final MIC submitted/completed10112/10112; Playback3024/3020. Both final
  telemetry and Removed snapshots retain zero reported drops/silence.
  Final lifecycle reset/recovery counters are MIC1/1 and Playback2/2, not
  erased or relabelled as steady-state fault-free qualification.
- Boundary initial prime is32; steady soft-event/xrun/recovery-prime/short-read
  deltas are0. Fifteen observations remain unknown. `QualityPassed=true` does
  not override `QualityComplete=false`, boundary quality incomplete or
  `qualified=false`. Collection/source submission/cleanup are complete.
- Generic `native_capture.delivery.packets=0` is unpopulated on this recorder
  path; it is not evidence against the10112 retained evidence packets.

Read-only queries over14:11:25-14:12:05UTC return no matching entries from
System, Application, Kernel-PnP Configuration or UMDF Operational. Unlike the
predecessor's16 retained Kernel-PnP events, this supplies no in-window rebinding
witness. Empty/unavailable queries do not establish complete log coverage or
the absence of every lifecycle fault. WPP has one Prepare/publish per direction
rather than the predecessor's repeated preparation.

## Retained offline identity refusal

The exact frozen9968505 local executable analyzes the archived PCM without
another native acquisition. PCM SHA256 is
`61C730088143EFFC8317CA0FB1117896A072160F6CE451E61CF42C019F41221C`.
Both stereo channels match throughout;31468 recorded frames are nonzero.

The same support heuristic as the [earlier indexed analysis](2026-10-31-indexed-marker-analysis.md)
proposes29 unlabelled channel0 windows: nonzero support separated by more than
32 zero frames, padded64 frames on each side. Each competes all29 original
codes. Thresholds remain correlation0.7, margin0.05 and RMS0.0001; admitted
work163714048 stays under256000000. No source identity or origin is assigned
from support order and no threshold is lowered.

Analyzer exit1 retains13 unique identities,16 missing and no duplicate;
`identity_set_complete=false`. All16 missing best candidates fail section
coverage; eight also fail global correlation. Missing classifications do not
prove16 missing source markers: this exact nominal-rate/no-resampling model
does not resolve filtering/SRC or other transformations.

Stored packet coverage is complete, but9611 native-QPC intervals violate the
existing nominal-rate diagnostic policy. They are not9611 lost frames, gaps
or calibrated clock drift. Source-to-recorder origin and native epoch remain
unknown. Operational exit0 and zero reported driver faults cannot override
this refusal or establish waveform continuity/physical delay.

Config SHA256:
`8CFFE41319CF41AC090DB44ABB568AB39441A317B9991B27E6D625B6A59E95B1`.
Diagnostic JSON SHA256:
`67699A8CF3F7E4E64CB0F0C3C1ECFC0520888AFF7B1AC069DBA092CE93C9EBB0`.
Analysis receipt SHA256:
`89F5E69C9F87E7D3B6B8E7748028536955C4C04B7D4539D228A07A2B1262AE69`.
The original argv, window proposal, inputs, stdout/stderr and failure remain
under `root-indexed-analysis`; this is diagnostic evidence, not acceptance.

## Archive, decode and exact cleanup

The44-payload archive plus manifest is2183778 bytes, SHA256
`EC4B699570107B09B7045016DEBEA2864FE3C2DE8F25457034327BDAD0B96366`.
Manifest11820 bytes, SHA256
`FF08D9DE9DCDCC1263D43A3983314C2A95DB96BE3B153E2D6AEF87C2B93FB8BC`.
Closed ETL73728 bytes, SHA256
`B80498D2BF66A8A5E1BFF9DC834107B3ED1681AAAEEB09D795E05681219921C2`.
Target/root archive validation and independent ZIP/extracted44-payload
byte/hash checks pass before and after cleanup.

Matching611 TMFs decode159 events with reported lost/format/unknown0; raw
trace-loss availability remains unknown. No rebase/dispatch/no-copy witness
is exported. Schema2 refuses `missing rebase/no-copy witnesses`, writes no
successful association and retains `root-stable-decode/association-refusal.json`.
Do not rerun merely to obtain a fault or infer that instrumentation fixed it.

The adapted exact-task cleanup helper passes50 guard and4 actual extracted-flow
PS5.1 inert checks. Fresh read-only guards pass before one unregister of the
exact unchanged terminal task; the owned trace is already absent. No raw file,
failure, cache, driver package or rollback is removed. At
14:19:39.5168923UTC the independent post-audit has no task, owned trace,
build/audio owner or raw child. Driver611 remains Running/root OK; boot stays
`2026-10-30T09:50:49.6598450+08:00`, with all six default roles and twelve
render master volume/mute controls unchanged.

## Remaining discriminating work

Preserve both terminal experiments and their different outcomes. The new
preceding-arm boundary still needs a real matched rebase/no-copy witness;
no scheduling change is justified by this control. The retained waveform
permits offline investigation of transformation versus loss without replaying
acquisition or weakening identity/continuity gates. Native verification of the
later final-QUERY repair, sustained lower-budget delivery, matched VAD/Process
and calibrated physical acceptance remain open. S1-S3 are not complete.
