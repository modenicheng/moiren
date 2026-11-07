# Same-stream still-open close confirmation - 2026-10-31

## Verdict

One bounded exact **virtual speaker/Input -> Bus -> virtual MIC/Output**
acquisition adds control-side close-confirmation logging to the previously
pinned probe. It fails with both retained cleanup revision errors and a
separate microphone-silence quality failure. This is diagnostic progress,
not S1-S3 acceptance, stable delivery, a latency comparison or physical
sub-10-ms qualification.

The decisive existing confirmation QUERY now shows, for each endpoint,
**the same stream generation1, gate epoch2 -> 3, Paused and still open**.
`finish_close` correctly returns the original SET_GATE error rather than
accepting a gate that is not closed. No generation guard is relaxed, no
additional QUERY/retry is added and no failure is suppressed.

## Isolated source and execution

To exclude subsequent experimental changes, the deployed probe is built in
an independent clean worktree at `960b3c91503929ee255682ff47ef82c157d3cbb4`:
base `0fd3fbfddf1ded6180ab8630d07250cf755879b7` plus the logging-only patch
from `39db9f4`. Only `src/vad/client.rs` and `src/vad/provider.rs` differ
under `crates/moiren-windows-audio`. The main experimental branch separately
retains the same patch as `6bd3ca8`.

The release EXE is7044096 bytes, SHA256
`B9380C1DE1115B860C7EA440F2C11C01E359DA7D8345A6C73804C27CF2767FC8`.
Host jobs2/shared target builds,177 baseline VAD tests, strict library/tests
Clippy and workspace formatting pass. The current experimental source also
passes188 VAD tests and strict Clippy. Main's two filtered `vad_export`
regressions pass independently. These are different source/test scopes.

New diagnostics remain outside Engine render and normal PCM pumping. The
worker-tail logging does still run on its Audio MMCSS-registered thread;
it is not a claim that this thread has ordinary scheduling priority.

The runner is derived from the previous debug collection, changing only
output paths, trace-session identity and EXE provenance pins. Audio arguments,
input pins, driver pins and `RUST_LOG=moiren::audio::vad=debug` are unchanged.
Its SHA256 is
`B64F13D1E3B0A561ACBD526F4065510BF8283BDCBA820ED256CBF66FB87ABAFD`.
Root executes58/58 PS5.1 runner fixtures. Independent review checks the exact
allowlisted script diff, parse, staged EXE and source patch; it does not
independently rerun all58 fixtures. Logging/rebuilding may alter scheduling,
so neither waveform nor timing equivalence is inferred.

Target remains `LAPTOP-NP49RG6H`, SSH Session0, installed driver609 from
`2a5aee55b9e6bcb2b402a9087e4ddc0e9d608267`, `oem203.inf` /3.1.1.609,
SYS SHA256
`C9F7FF8273239B97436DF75A5D9E7584347F54A79CCFA21E1889EE546E4C591F`.
No driver install, reboot, physical playback, volume/default write, cache
cleaning or unrelated process termination occurs. Probe stages side-by-side
at `D:\MoirenTest\20261031-close-confirmation\bin\e2e_latency_probe.exe`.

All parameters and inputs match the
[default-period receipt](2026-10-31-default-period-iobus.md): actual Float32
stereo480 frames at each Exclusive native end,32-frame Engine block,
target1024/capacity4800/jitter96, packet/gap480, recovery cap8,
correctness3000ms, no minimum-period flag. Source input SHA256 remains
`6862D8FD1234B5FC20DADF28D644DBEACAEFDA556381465CD8C47D87303C8D4D`;
collection input remains
`02D42FB160F994243D0130A867E9031ED09F09505920F90C387ECDB943B2AC91`.

## Actual result

- Start UTC02:27:30.5321440; terminal receipt02:28:05.9153744.
- PID5832, confirmed exit1,33.9278111 probe elapsed seconds.
- Source completes1440000 scheduled frames and29696 nonzero source frames;
  native submission1451040 frames includes480 silent preroll frames.
- Recorder retains1455840 stereo Float32 frames,3033 complete480-frame
  packets and11646720 PCM bytes.31468 nonzero frames/62936 scalar samples,
  all finite, bit-identical channels, peak0.047552947.
- Runtime collection/cleanup complete=false; quality passed/complete=false.
  `quality_failure="microphone provider silence"` remains recorded alongside
  `ShutdownHost ... SET_GATE GetOverlappedResultEx ... 0x8007051A`.
- MIC snapshot `silence_packets=1` is already present in the first-quality
  Running observation, before teardown. Provider telemetry silence0 and
  capture silent-flag counts0 do not cancel that driver observation.
- Final external checks find no audio owners or present VAD children, root
  PnP OK, driver Running, same boot
  `2026-10-30T09:50:49.6598450+08:00`, equal observed volume/mute/default roles.
  `CleanupErrors=[]` is outer-runner evidence, not successful Engine cleanup.

Nonzero PCM, complete packet storage and submission are not waveform
continuity. No marker recovery, source-origin mapping or delay inference is
performed. The early-Running silence is a separately retained quality fault;
its causal relationship to the close failure is not established.

## Decisive close observations

Raw `target/probe.stderr.log` records:

| Lines | Observation |
| --- | --- |
| 15-17 | MIC token14/session886026745613: transport requests close at gen1/gate2/Paused/open; confirmation at gate3 remains open; rejects still-open and retains original error |
| 18-19 | MIC worker later queries gen1/gate5/Closed/open and closes successfully at gate6 |
| 39,86-87 | Playback token13/same session: worker queries gen1/gate2/Paused/open; close confirmation at gate3 still open, rejected |
| 88-89 | Playback lifecycle later queries gen1/gate5/Closed/open and closes at gate6 |

Text order is not atomic cross-thread timing. In particular, these records
do not identify a specific ACX callback as the gate2 -> 3 writer, prove a
reopen or explain the early-Running silence.

Source review at installed driver609 shows why an advanced epoch need not
mean a closed gate: native reset calls `InvalidatePcmEpoch`, which ordinarily
retains GateOpen; stream release/cleanup can reset again before publishing
Closed. The exact gate comparison in `VadSetGate` and the still-open rejection
in `finish_close` are therefore appropriate. Probe admission invalidation
does not cancel/join transport workers before its external native Stop calls.
This supports investigating retirement sequencing, not weakening the ABI
gate check or adding a blind refreshed-epoch close retry.

## Integrity and next boundary

Host evidence root:
`D:\coding\moiren-evidence\20261031-close-confirmation`.
ZIP `20261031-close-confirmation-target.zip`:524101 bytes,21 payload files
plus manifest, SHA256
`E38214FA01BD2A57D38AF2351281270967DB8F35D97177D1F6DC1B2299BECD60`.
Root and independent audit verify ZIP CRC, exact members, manifest lengths
and SHA256, and extracted bytes. Original target evidence remains retained.
The archive operation initially fails to load ZipArchiveMode; loading both
required compression assemblies resolves packaging without rerunning audio
or rewriting existing payloads.

Independent receipt `independent-evidence-audit-20261031.json`, SHA256
`3BEA4230D85AC19B19A20E2B78796B9BE9E4530154C985EA4554E9CC2C74C5F5`,
verifies the two confirmation records, source/artifact pins, PCM statistics
and retained failure. No ETL decode/loss or continuity qualification is
claimed for this audit.

Next: design a route-scoped retirement/quiescence regression before any
behavioral repair. Preserve stop-broadcast/deadlock protections, all failure
and panic cleanup stages, single-confirmation same-stream guards, raw
quality counters and external-client release before endpoint removal.
This receipt does not select a repair or change production defaults.
