# Direct remote waveform pair and VAD cleanup refusal

Date: 2026-11-07, Asia/Shanghai. This follows the earlier
[waveform checkpoint](2026-11-07-matched-waveform-checkpoint.md), preserving
that failed acquisition rather than replacing it. The full S0-S5 pipeline
remains open. No latency, continuity, Windows-name or lower-default pass follows.

## Direct remote scope

Local evidence: `D:\coding\moiren-evidence\20261107-matched-waveform-2255`.
Remote evidence: `D:\MoirenTest\20261107-matched-waveform-2255`.
The standalone `moiren-lan` target is used directly. This slice requires neither
VM startup nor visible desktop input/unlock. It makes no lock-state diagnosis.
There is no host native operation, installation, endpoint enable/reset,
identity rotation, default/volume/security change or rename round trip.

The unchanged clean `b08f61b` binaries retain the preceding checkpoint's sizes
and SHA256 pins. Driver3.1.1.612, the six-code schedule, source Shared480/native
capacity1056, DAWN Shared336/capacity738/target512/wake64/callback64 and Engine
F32/block64 unity `Input -> Bus -> Output` route are unchanged.

## Runner correction and false-acceptance guard

This scope incorporates the already established OBS close correction:
confirmed inactive -> close owned websocket -> normal window close/join.
Unresolved recording still retains ownership through held cleanup. The
15 established tests pass; an AST comparison confines the recovered correction.

A new regression against the actual `acquire('vad')` path proves that retaining
`native.error` without refusing it can incorrectly accept a failed VAD report.
The guarded runner now requires that error to be null. Its two focused tests
pass after the original red case; a null error still permits the existing
unqualified probe exit1 contract. Fourteen actual lifecycle cases and twelve
pure controller cases pass. Nine safety-receipt checks pass under PS5 and PS7;
both shells parse the entry/launcher. The fresh no-audio selfcheck ends Ready0.

The reviewed eight-file native intent has SHA256
`BDA378DFD79281928F892453375DF7425EE460AB8539EDB999A02C304B8EA848`.
Both launch and entry retain the matching fresh successful before-audit gate.
The native task is launched once; timeout recovery observes the same task,
never creates a replacement acquisition.

## Actual acquisition

The before audit at23:01:42 and after audit at23:03:07 both verify the pinned
driver/INF/boot, all six default roles, render controls, security and protected
external process identities. The terminal native task is Ready/result1 with no
owners. Audit success does not convert the route failure into acceptance.

- Process completes its source schedule, Stop/join and graph/monitor cleanup.
  Source23560 and monitor29324 exit0; reference19672 retains expected
  unqualified exit1. OBS15816 exits0 without force.
- VAD source Stop/native Stop/worker join complete. Engine cleanup is
  Closed/Dormant, faultNone and cleanup_errorNone. Nevertheless, the probe
  reports `VAD cleanup did not reach Removed`; the new runner guard refuses it.
  Graph27500 exits1 and OBS14252 exits0 without force.
- The final exact runtime21 snapshot remains Removing/Closed, gate closed,
  audio_readyfalse, with stamp `[21,7386780980884,21,6,21]` and provider errornull.
  The existing20s deadline is unchanged. Raw VAD reference retains383520 stereo
  PCM16 frames/799 published packets and a joined worker.
- VAD OBS Stop dispatch and inactive observation succeed. Its explicit closed
  path is retained under `records_after_error`, not silently promoted to a
  successful primary `records` result.

A later23:09:36 read-only inventory finds no present raw VAD children or test
owners. That is not direct Removed evidence inside the original deadline.
The raw Windows name still contains the adapter suffix; no naming gate closes.

## Offline analysis without changing acceptance

Canonical v3 runs once to Ready0 with28 synthetic checks, unchanged input
hashes and three analyzed boundaries. It refuses the VAD return because the
failed route has no primary `records` object.

The narrow v4 extension accepts an explicitly retained after-error path only
when the primary key is absent and the exact Stop/inactive/no-responsibility
witness is true. Invalid primary paths, including explicit null, cannot fall
back. Original native errors remain unchanged. Four focused regression tests
pass locally and remotely; the original28 tests also pass locally/remotely.
Independent review finds no additional blocker after the null-primary repair.

The ten-file/tool v4 intent SHA256 is
`8DFB50F32938048FD485E3BAA9461C919403AFD4C6E8D440698640A581230BC7`.
Actual v4 analysis finishes at23:10:17, Ready0, owners[], unchanged inputs and
partial_acquisitiontrue. Both stereo channels retain the following results:

| Boundary | Stored frames | Resolved identities | Unresolved identities |
| --- | ---: | --- | --- |
| Independent Process reference | 431520 | 0,1,2,3,4,5 | none |
| OBS Process return | 696320 | 0,2,3,4,5 | 1 |
| Raw VAD Playback reference | 383520 | 0,1,2,3,4,5 | none |

V4 locates the closed VAD return container,18300840 bytes and30.700s, but the
unchanged `0 < duration < 30` short-run guard refuses it before decode/analysis.
Neither the cap nor the recording is altered to admit it. It is not a fourth
analyzed boundary. Its extra duration includes cleanup waiting, not a latency
measurement. No native rerun follows this refusal.

Correlation0.7, margin0.05, RMS0.0001, nominal48k model and the1e9 work cap are
unchanged; actual admitted work is60966912 correlation samples. Code1 has a
candidate but fails section matching; this is not a proven lost-marker count.
No SRC/gain fitting, identity-by-order or cross-origin latency subtraction is
introduced. The actual reference and return figures are visually inspected.
OBS WASAPI output is not calibrated physical/DAC measurement. All quality,
continuity and physical qualification flags remain false.

## Cleanup investigation and next boundary

Source review establishes that source Stop consumes the session and joins the
WASAPI worker before returning a data-only outcome. It does not establish a
retained source-handle defect. Closed Engine/stream state is distinct from
final PDO destruction: `VadChildDestroy` in `driver/vad/src/Children.cpp`
publishes Removed through `FinishLifetime`.

`Release` in `driver/vad/src/audio/CircuitDevice.cpp` returns a failed
`AcxDeviceRemoveCircuit` status without publishing it in the endpoint snapshot.
Thus provider errornull cannot discriminate that path from successful release.
This is an observability gap, not proof that the API failed in this run.
Runtime21 needs accepted removal -> ReleaseHardware result -> child cleanup ->
final destroy evidence before selecting a product repair. Portable tests that
manually invoke cleanup/destruction cannot supply that native sequence.
No timeout extension, reset, identity replacement or unchanged acquisition
rerun is justified. Matched full return correspondence, native budget/quality,
desktop input, naming/settings preservation and sustained calibrated p95/p99
qualification remain open.

## Verified preservation

All166 compact evidence members verify independently against their lengths
and SHA256. `compact-evidence.zip` is848944 bytes, SHA256
`17E539178A7977D2550BD900596495D93ED7A9AA24BEA387E3E1D280CA0FA3FF`.
The archive includes both route reports, native/analysis intent and receipts,
before/after audits, original packet metadata and both v3/v4 outputs. Raw
MKV/PCM and the large scalar trace remain remote; no media is copied to host.
The original29dirty/cache file pins all remain unchanged.

The read-only inventory helper initially fails on PS5 hashtable property
measurement; that failed output is preserved and a corrected observation is
separate. The archive itself succeeds once, using explicit compression
assemblies, direct JSON-array assignment and capped exact members.
All seven current-scope tasks are terminal Ready; no test owner remains.

PR26 remains Draft/open. At this checkpoint the preceding publication1ccc0ad
has no exact-head Actions run; earlier0fdc2c6 green CI does not cover this slice.
No current CI-green or whole-branch merge claim is made.
