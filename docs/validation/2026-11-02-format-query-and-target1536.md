# Format-query receipts and bounded target1536 failure

Date: 2026-11-02. Baseline: `3693266`. S1-S3 remain incomplete.

## Example-only diagnostic repair

The previous target1536attempt stopped before source Start because neither
format query yielded exact `S_OK`, but its HRESULTs and selected Playback
identity were lost. Static inspection finds no descriptor/driver-format
contradiction; it cannot explain the previous native failure.

The probe now retains resolved preparation identity before stream preparation,
and each attempted Float32/PCM16candidate's activation success/error and raw
`IsFormatSupported` HRESULT. An unexecuted query remains null. Only actual0
selects; nonzero query results continue to the same second candidate as before,
while activation error still aborts immediately. No retry, Shared fallback,
changed format, production scheduling, native period or queue default is added.
COM calls run outside the short publication lock, on the source owner thread;
the existing selection makes at most two attempts. Joined partial outcomes
retain these receipts without inventing initialized stream information.

Important provenance distinction: typed `Activate` returns `Result<T>` and
erases successful HRESULTs. Final code serializes `activation_succeeded` and
`activation_error_hresult`, not an invented raw success code. The intermediate
binary used for the acquisition below emitted `activation_hresult=0` inferred
from typed success; that old field is **not a captured raw activation HRESULT**.
Its `format_hresult=0` is captured raw. Original artifacts are not rewritten.

## Software verification

Evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-format-query-receipt-20261102-1910`.
Clean `git archive 3693266` plus the two intended example files excludes all
27preserved dirty/cache files. Only two test initializers change in
`vad_collection.rs`; production library/driver/application code is untouched.

- A real preimplementation regression fails (`format_queries: null != []`),
  then passes after outcome wiring.
- Tests cover Float32success, PCM16after failure, positive nonzero/S_FALSE
  refusal, both queries failing, immediate activation error without query or
  fallback, no lock across native operations, and nonnull preparation identity
  surviving a subsequent failure. Two fixture initializer omissions were fixed
  after the first compile check; the failure log is retained.
- Corrected final full example suites, feature off/on(`probe-tools`), each
  pass694tests/1ignored. Strict example/test Clippy, workspace formatting and
  release build pass. Independent read-only review is recorded.
- The final corrected binary receives a separate remote dry-run only. No
  second audio acquisition is performed for its reporting-only correction.

Final probe on the remote test machine:
`D:\MoirenTest\20261102-format-query-1915\e2e_latency_probe-final.exe`, SHA256
`93CBC10054C67967C520E3D38558C7A359FD39DFF051A1649769FC4454820F1C`.
This is a probe, not a new desktop application or driver package. The acquisition
used the separately retained intermediate `e2e_latency_probe.exe`, SHA256
`D0B235B5A23EED8A280C30B744500A8FB2F8B0A5A4C6F9A9575A45BC72FDC081`.
Do not relabel the intermediate receipt as execution of the corrected binary.
A prior corrected dry-run binary had SHA256
`D23D79723458AB0ABCE206AACF5D453CD4F43A9102969BB907CA7527A1F3308D`.
After a comment-only source edit its rebuild hash changed; byte identity was
not assumed. The exact final binary above was separately uploaded, hash-pinned
and dry-run checked, without another audio attempt.

## One native attempt, stopped on active failure

Remote root: `D:\MoirenTest\20261102-format-query-1915`.
Host root: evidence directory `s2-format-query-target1536-20261102-1915`.
Actual native wrapper window:19:15:34.729-19:16:55.711+08:00.

The exact previous complete configuration is retained: Playback -> unity Bus ->
MIC,48k/stereo, Engine64, recoverycap8, target1536, capacity4800, jitter96,
declared packet480/gap512. Gap512is still an assumption, not certified delivery.
The intended source is180seconds of continuously changing bipolar PCM at.025,
repeating a deterministic1-second sequence. Quality mode is fail-fast
`latency-sensitive`; no load injection or parameter scan is added.

Unlike the previous pre-Start refusal, Float32activation succeeds and its raw
format query returns `S_OK`. The source opens Exclusive Float32/480; the owned
MIC recorder is Shared/48k/F32/native1056, provider Pcm16/480. This establishes
this attempt's selection, **not the cause of the previous format failure**.

At the first retained failure,76,275mscontrol observation:

- MIC gate_epoch changes1->2 while stream_generation stays1 and shape remains
  Stereo/Pcm16/480. It is still Present/Running/gate-open. The generic identity
  error therefore identifies a gate epoch change, not proven stream replacement.
- MIC QUERY silence_packets is1and worker epoch_resets/recovery_events are1/1
  before cleanup. Last-live QUERY silence rises to3; retired QUERY reaches4.
  Do not call all four active failures or all four teardown-only events.
- Engine remains Running, no fault/progress timeout; its virtual clock already
  records3clamped periods/192frames, max backlog10againstcap8, timeline_resets0.
  MIC boundary_epoch1/SRCreset_generation2are retained; boundary xruns stays0.
- Playback drops/missing/stale/recovery/reset counters remain0. Its cumulative
  rounded decode-call envelope max16.2012ms exceeds the declared512-frame
  duration; this is not packet arrival, a supply-gap proof or causation.

The collector stops on the gate mismatch. Source Start/Stop/join succeed with
3,648,960native submitted frames, including480preroll; source schedule remains
incomplete. `collection_complete`, `source_submitted_complete`, `cleanup_complete`
and quality verdicts remain false. Shutdown separately reports slot3
`0x8007051A`; it is not the retained initiating error. Probe exits1normally,
without timeout/forced termination. Both exports reach Removed/Closed; external
OBS/QQMusic identities, driver/boot and bare control bus are unchanged.

Code trace permits native clock rebase to invalidate the MIC gate epoch;
lifecycle changes can also advance it. Late/old WRITE rejection itself counts
drops rather than advancing the gate, while no-copy MIC callbacks count silence.
The worker can reset after observing a new driver gate epoch. Engine clock
clamping is a separate mechanism; timeline_resets0does not establish that it
caused or did not cause supply pressure. The specific initiating callback and
causal ordering are not proven, so no speculative production fix follows.

## Remote closed-PCM analysis and visualization

Raw PCM29,287,680bytes and SRC chunks85,621,681bytes stay remote. A619061-byte
compact archive is verified by length/SHA256
`70C0BF38E0239BE71947751AB14E71C4CFAA92C98724A5E9516FC282EEAC557C`,
with27individual input hashes. The isolated existing Python environment runs
with `-I -B` and single-thread numeric libraries, after the audio owner exits.
Five analyzer selfchecks pass. An initial wrong metadata-key analysis attempt
fails before output; its corrected closed-file analysis finishes in1.711s,
without a native rerun or audio activity. Input lengths/hashes are unchanged.

- Recorder stores3,660,960stereo frames/76.270s in7627packets. No storage sequence,
  stored-coverage or device-position gap, overflow/drop or nonfinite sample is
  reported. The sole retained packet flag is packet1/discontinuity; first-packet
  investigation remains deferred.
- Both channels have an exact-zero prefix17103frames and terminal1920frames
  (40ms); these spans are observed, not automatically classified as lost audio.
  No other exact-zero run >=64frames appears. RMS largely holds steady, but
  energy/zero checks do not prove source identity, continuity or absence of
  repeated/dropped samples.
- Recorder QPC interval min9.4336/p5010.01545/p9510.35635/max19.6615ms is packet
  timestamp spacing, not E2E latency. The repeated source and unknown cross-clock
  map do not justify a latency estimate.

The reviewed three-panel plot is `failed-target1536.png` in the host run root
and the remote `analysis` directory:361417bytes, SHA256
`68B5571EF3DBF2BA934D6D5204044FF57EBA8C5F0D212D39E4D6E83EC5B257A2`.
It shows100msRMS, packet QPC spacing and the final2seconds including terminal
zeros. Three compact analysis artifacts are hash-verified. No waveform/physical
qualification PASS or lower-default promotion follows. This is a real bounded
candidate failure, not reason to revisit first packets or repeat unchanged soaks;
independent development can continue while deeper timing work stays scoped.
