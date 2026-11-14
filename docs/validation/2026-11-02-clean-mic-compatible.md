# Clean Compatible playback -> Bus -> MIC functional collection

Date: 2026-11-02, Asia/Shanghai. Source: `dc0acfd` on
`feat/minimal-io-latency`. This is a new finite functional receipt for the
original S1 virtual route, not a smaller-setting scan, stability soak, physical
latency qualification, or a production fix. First-packet work stays deferred.

## New question and result

The current clean application has VAD -> physical-output evidence, but the
recent MIC945/cap1 record has dirty-build provenance and a running completion
fault. Neither establishes current clean Compatible playback -> unity Bus ->
MIC operation. This run asks whether that exact existing route can start both
native clients, submit a changing source, collect bounded PCM, and retire both
owned exports normally. It does not try to reproduce or excuse the old fault.

**Functional collection and cleanup complete; quality remains unqualified.**
The sole native owner runs18:27:28.027-18:27:52.720, exits0without timeout or
forced termination, and retains source/collection/cleanup complete with null
collection/native errors. The20s schedule is submitted completely; acquired
PCM lasts20.78s. Raw `quality_passed=false` and its original refusal survive.

No second native run, parameter scan, gate relaxation, larger buffer, new
default or first-packet investigation follows. This closes the clean-build
functional receipt gap, not lossless whole-waveform or physical qualification.

## Target, source and isolation

- Native target `LAPTOP-NP49RG6H`, Windows build22631, existing running SYS
  SHA256 `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.
  Driver and boot identity are unchanged; no install, device-default, volume,
  timer, OBS or QQMusic control is performed.
- Existing clean probe:
  `D:\MoirenTest\20261102-probe-exact-shared-1805\e2e_latency_probe.exe`, SHA256
  `59C12E05880386F2DE952F984A408B192B4D17EA5AFAE8E1C9B18CA0A2DFE336`.
  Its source is `git archive 6be7470` plus only the three Rust modules committed
  byte-identically in `dc0acfd`; the intervening changes are documentation.
  The original18dirty files and9cache files are excluded, not built or staged.
- Native evidence root
  `D:\MoirenTest\20261102-clean-mic-compatible-1825`.
  The1825suffix is an identifier; actual native start is18:27:28.
  Isolated APPDATA/LOCALAPPDATA/TEMP; no host native/audio/UI operation.
- Exact playback identity `4d6f6972656e5332506c617931383235`; exact MIC identity
  `4d6f6972656e53324d69633138323558`. The bare bus is `ROOT\MEDIA\0001`.
  Exact root/stable identity resolves the recorder; friendly names are not
  selectors. Both owned exports reach `Removed`/`Closed` after recorder release.

The binary's preceding source receipt has683passing example tests with each
feature setting and1074passing audio regressions. Those are linked historical
checks, not suites rerun or renamed by this functional acquisition. See
[exact Shared probe repair](2026-11-02-probe-exact-shared-period.md).

## Exact route and acquired evidence

| Quantity | Retained evidence |
| --- | --- |
| Graph | VAD Playback -> unity Bus -> VAD MIC; no Gain, physical master or bypass |
| Engine |48kHz/F32/64frames; pure virtual realtime owner |
| Source |Exclusive Float32/48k/stereo, native480frames; no minimum-period request |
| MIC recorder |Shared Float32/48k/stereo, native buffer1056frames; provider Pcm16/480 |
| Budget |Compatible, default recoverycap8; no input target/capacity/jitter/recovery override |
| Prepared boundaries |4800capacity; playback target2448, MIC target2352; not measured delay |
| Source schedule |20s/960000frames,19indexed-v2 codes1024frames, amplitude0.025, intentional quiet gaps |
| Native submissions |985440frames incl480preroll and silent tail; acknowledged source960000 |
| Recorder |2078packets/997440stereo frames,20.78s,7979520PCM bytes |
| Storage |276880metadata bytes; no missing/incomplete/sequence-gap/nonfinite/drop/overflow |
| Capture flags |Only packet_sequence1 has flag1; stored coordinate0, device position480 |
| Packet coverage |Contiguous full stored range and device positions; no repaired/dropped rows |

The ring16384frames and PCM32MiB/metadata8MiB/10000packet caps are recorder
artifact limits, not production residency targets. Collection has a40s phase
budget and100s held-process outer deadline. Real phases are recorder Running
at12ms, source activation262ms, source observed273ms, source submission complete
20261ms and postroll complete20763ms, relative to recorder activation request.
These control times are not source/recorder clock alignment or route latency.

Both retained running provider observations have drop/silence/missing-sequence/
epoch-reset/recovery counters0; the running MIC is Pcm16/480with provider error
null. The final virtual clock has clamp0/timeline-reset0/late-wake-burst0,
max backlog1. Playback startup initial-prime32is classified separately; its
observed steady delta has soft0/Xrun0/short-read0/recovery-prime0. This does not
claim full PCM provenance or exclude every unobserved fault.

Do not describe the entire lifecycle as reset-free: final teardown provider
telemetry retains Playback epoch-resets/recovery2/2and MIC1/1, with closed gates
and removed streams; final SRC checkpoints also retain discontinuity/reset
state. Those are distinct from the preceding running observations. This run
has no retained running completion error; it neither reproduces nor exonerates
the old945/cap1 `0x8007051A` result.

## Unchanged-detector analysis and visualization

All raw PCM/packet rows remain remote. One isolated existing Python environment
runs with `-I -B` and single-thread numeric settings; it uses read-only PCM,
pre/post hashes, the existing support planner and the pinned unchanged Rust
`analyze-indexed-pcm`. No new package/environment is installed. The analysis
wrapper18:31:40.933-18:31:44.900exits0; that is analysis execution, not detector
acceptance. Both detector children exit1and retain their original issues.

Each nonzero component is found in stored recorder coordinates, joined across
at most32zero frames and padded64frames on each side. No source identity is
assigned from component order. Each candidate competes against all19original
codes, with correlation0.7/onset-and-identity margin0.05/RMS0.0001and quarter
checks unchanged. Charged work117592064is admitted before execution, below
256000000per child and2500000000aggregate. There are no work-refused components.

- Each channel contains19energy components and20006nonzero frames; maximum
  absolute amplitude0.0492858887, stereo difference0. Identical stereo is not
  an independent reference or zero route delay.
- Each channel accepts17unique identities:0-13and15-17. Components14and18
  retain `missing`, not assigned identities or evidence of two lost markers.
  The strongest unaccepted candidates14/18have confidence0.70650/0.74059;
  both fail section matching and candidate14also has onset margin0.04060.
- Raw capture quality retains one first-packet discontinuity and
  `measurement evidence loss, silence or discontinuity`; no packet is deleted
  to turn `quality_passed` green. Its investigation remains deferred.
- The unchanged packet diagnostic separately reports1784QPC intervals outside
  its one-frame-plus1000ppm nominal-rate policy, per detector invocation.
  Measured packet timestamp intervals are min9.4483/p5010.0273/p9510.3197/
  max10.5538ms. These are timestamp intervals, not E2E latency, actual PCM loss,
  or a reason to tune the threshold. They prevent a diagnostic-quality PASS
  even if the deferred first-packet event is considered separately.

The reviewed four-panel PNG shows10mspeak,10msRMS, packet timestamp intervals
and accepted identities/refusals. Its title retains operational success versus
raw-quality failure. The plot does not prove continuous lossless waveform,
cross-clock mapping, acoustic/hardware presentation, calibrated delay, a path
winner or the best stable lower setting. The17identities improve evidence
beyond energy counting without justifying another diagnostic loop or a
speculative production change.

Fresh analysis checks: five remote analyzer selfchecks pass; the unchanged
support-planner suite passes9tests on the host without audio. Initial runner
preflights expose PS5singleton-array counting and a malformed endpoint-ID input;
both are corrected before native audio. Their failed logs/receipt remain,
along with final non-audio dry-run. Review also fixes wrapper native0versus
dry-run1semantics and makes safety audits independent before the sole native run.
No failed result is overwritten or promoted to a native success.

## Archival and stopping boundary

Host root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-clean-mic-compatible-20261102-1825`.
Reviewed plot: `analysis-archive\analysis\native-mic-functional.png` beneath
that root; remote original: `analysis\native-mic-functional.png`.
The32native compact entries and25analysis compact entries are independently
verified by size/SHA256 after download. Raw PCM/packet rows are not transferred.

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| native compact ZIP |446275|`83FC8FFB1A8A28C1E2D0D6A770341447CE1AE79A4EADEAF0CE0179AF7696C412`|
| analysis compact ZIP |651003|`A691C9DE0422B05E31F7242D86C64A365736A465CFC3E63D1C10462C725A2E2B`|
| reviewed functional PNG |494008|`C4B384C8A7D83D4097F988C503EA1CE2C443A5095FB5E7D8457F0511ACA9911C`|
| remote measurement.f32le |7979520|`665B0811C2E40B8C6E2C57D96DFAE85A5CFAF4DF1C55005614CFECEA0E62B0CC`|
| remote measurement.packets.jsonl |276880|`5A1B41123264B9C6F45879AD920BD7784BF022A4E42141019DCADFE26FD1DFCF`|

The18:32:18final read-only audit finds no native/analysis owner, only the bare
control bus, unchanged driver/binary and unchanged protected process creation
identities: OBS12648/1793505454949ms and QQMusic15096/1793504036422ms. Native
boot identity matches before/after; raw hashes remain unchanged after analysis.
No global process kill, user process action, driver change or cleanup of
unowned files is used. Original27dirty/cache files remain unchanged.

This is concrete clean-build functional progress on S1, not completion of the
full S1-S3 objective. Matched source/output quality, lower-setting native
qualification and the unavailable calibrated physical reference remain open.
Absent physical equipment does not block independent development, and these
finite results do not call for another unchanged soak. Local commit only;
no push or memory update.
