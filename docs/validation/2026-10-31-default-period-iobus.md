# Default-period virtual IO/Bus collection - 2026-10-31

## Verdict

Two bounded native acquisitions exercise the exact virtual
**Playback/Input -> Bus -> Microphone/Output** graph on physical target
`LAPTOP-NP49RG6H`, diagnostic driver609. Both retain nonzero microphone PCM
after a changing source completes. Both nevertheless fail EngineHost cleanup
with `SET_GATE` revision mismatch `0x8007051A`. Neither is a passing collection,
continuity result, completed S1-S3 or physical latency measurement.

Direction is unchanged: other apps render into the virtual speaker, which is
Moiren's Input/Source; Moiren's Output/Sink supplies the virtual microphone,
which other apps capture. No Gain or other DSP node is in the graph. There is
no DAWN/physical output client, independent reference channel or calibration.

The second acquisition adds only the existing control-side stderr logger.
It reproduces the cleanup failure; it is not a fix or an independent latency
comparison. Existing concurrent-close fixes `121118c` and `94cc6ce` are already
ancestors of probe source `0fd3fbf`. An older probe artifact alone does not
explain this error, and the HRESULT is not evidence of an ABI incompatibility.

## Source, route and configuration

Retained `D:\MoirenTest\20261031-c985fc3\bin\e2e_latency_probe.exe` is freshly
attested at7038464 bytes, source `0fd3fbfddf1ded6180ab8630d07250cf755879b7`,
SHA256 `651B0596700791E3C0F3C68E35C52D2D9F0432196F50618C5E9FD8E445D1B94E`.
Installed SYS remains `2a5aee5`, `oem203.inf` / `3.1.1.609`, SHA256
`C9F7FF8273239B97436DF75A5D9E7584347F54A79CCFA21E1889EE546E4C591F`.
No rebuild, install, trust change, reboot, physical playback, volume/default
write, cache cleaning or unrelated process termination is performed.

The retained source constructs the exact three-node graph in
`examples/e2e_latency_probe/vad_route.rs::minimal_project`, selected by
`live_native.rs` for the owned MIC route. Both runs use:

- Explicit stable Playback/MIC IDs ending0061/0062; only owned MIC is recorded.
- 48 kHz stereo Exclusive source, Float32-first; actual source buffer480 frames.
- Explicit Exclusive native MIC recorder, actual Float32/480, ring8192 frames.
- Engine block32, virtual target1024, capacity4800, jitter96, declared packet/gap
  bounds480, recovery cap8. No production/default buffer setting is promoted.
- Correctness policy with3000ms jitter tolerance. Fault retention remains
  separate from operational completion; latency-sensitive gates are not relaxed.
- The previous schema1 changing schedule:29 markers of1024 frames, amplitude
  0.025 over1440000 source frames, and schema2 Exclusive collection config.
  Exact input SHA256 values are respectively
  `6862D8FD1234B5FC20DADF28D644DBEACAEFDA556381465CD8C47D87303C8D4D` and
  `02D42FB160F994243D0130A867E9031ED09F09505920F90C387ECDB943B2AC91`.
- Omit `--vad-minimum-period`, unlike the inspected earlier integrated1,
  target768 and repeat2 acquisitions. No baseline with this default-period
  configuration was found in that bounded evidence set; no global absence claim.

This is not exact PCM16 A/B with the earlier direct positive control: source
Float32, changing waveform, Engine/Bus/MIC and source priming differ. It is
also not sharing-only A/B with the Shared zero fixture.

Guarded runner checks source/driver/input hashes, idle build/audio owners,
disk space and fresh directories. An unrelated remote UI build is preserved;
acquisition starts only after a fresh idle observation. The controller keeps
SSH alive through native exit, without a timeout-to-kill path. Root reruns all58
independent PS5.1 base-runner fixtures successfully. Independent debug-runner
review finds zero PS5.1 parse errors and exactly the output path, trace-session
name and existing logger filter/receipt changes; it does not rerun all58 debug
fixtures. Logging may perturb scheduling, so this is not timing equivalence.
Base runner SHA256:
`EFC3F0D7CE9A6115F8F2E465B68FECAEC1BDD26BE14AE075FA300657E217358E`.
Debug runner SHA256:
`B8FE49B1C8AA6DA61067B7F1E9B73D7FE62000D345D298A16A50B5D03834644C`.

## Actual outcomes

Host evidence roots are `D:\coding\moiren-evidence\20261031-default-period-iobus`
and sibling `20261031-default-period-iobus-debug`; each contains a verified
`target` mirror and raw ETL. Original target artifacts are retained.

| Observation | Base | Existing-debug-logger repeat |
| --- | --- | --- |
| Start UTC | 02:09:47.5519280 | 02:14:32.5916327 |
| Final receipt UTC | 02:10:21.4789964 | 02:15:05.6719243 |
| Probe PID / confirmed exit | 10316 / 1 | 848 / 1 |
| Probe elapsed seconds | 32.418629 | 31.5917628 |
| Recorder frames / packets | 1455840 / 3033 | 1455840 / 3033 |
| Retained PCM bytes | 11646720 | 11646720 |
| Nonzero scalar samples / frames | 62942 / 31471 | 62944 / 31472 |
| Absolute peak | 0.0474268645 | 0.0475459881 |
| Source schedule submitted | complete | complete |
| Runtime collection / cleanup complete | false / false | false / false |
| Quality passed / complete | false / false | false / false |

All PCM samples are finite, whole stereo frames; channels are bit-identical,
not independent references. In both, source acknowledges1440000 scheduled frames,
29696 nonzero source frames and1450560 native submitted frames including480
silent preroll frames. Source Stop succeeds and worker joins, with0 source
event timeouts. Native recorder Stop succeeds; sink retains3033 complete packets,
0 incomplete/missing packets,0 sequence gaps and0 nonfinite samples. Native
evidence metadata reports0 discontinuities/drops/gaps/overflow/timestamp errors;
its0 `silent_frames` is a flag count, not absence of numerically zero PCM.
These counters and nonzero values do not establish waveform continuity.

Both reports retain `quality_failure: null`, but that is not a pass: final
graph outcome is unknown/failed and `quality_complete` is false. External
post-run checks find no present VAD children or audio owners, root PnP OK,
driver Running and unchanged boot `2026-10-30T09:50:49.6598450+08:00`.
Complete before/after catalog observations retain equal volume/mute and all six
default roles. These snapshots do not attest every intermediate state. Clean
external absence does not erase the internal shutdown error.

## Native boundary and waveform limits

Base tracefmt with matching609 private-PDB TMFs reports160 events,0 lost,0
format errors and0 unknowns. Frozen strict native26/READ15 decoders accept
endpoint9/session875378860136/generation5/gate13/revision1000, Float32 stereo480.
Both hit480000 inspected frames, with10240 nonzero frames and81920 nonzero
bytes, without sequence gaps/saturation. READ scanned source/destination regions
differ in0 bytes. Native path is EVENT2/PacketCount2:3840-byte packets,
4096-byte allocation, RT offsets256/0. Native sequences2..1001 and READ1..1000
are not exact sample alignment. First nonzero prefixes repeat `CD CC CC BC`
(Float32 approximately-0.025). This bounded first-ten-second observation is not
the whole recorder interval or a physical delay measurement. Event destination
is the same single read, not an independent paired native copy.

A retained host-only exploratory marker-presence script finds23 global peaks
above its fixed0.7 normalized-correlation threshold in the base PCM. The source
schedule contains29 markers, but **this is not23 identified markers out of29**:
the script anchors expected windows to the first peak without an independent
source-origin mapping. Nonzero PCM precedes that peak. Weak/late windows and
end-of-recording extrapolation cannot identify lost markers. No threshold is
lowered, no raw delay distribution is reported and no continuity pass follows.

## Cleanup failure and next evidence boundary

Both retain:

`ShutdownHost: VAD EngineHost cleanup: resource unavailable: SET_GATE
GetOverlappedResultEx(VAD): ... (0x8007051A)`

The debug repeat explicitly inherits `RUST_LOG=moiren::audio::vad=debug` and
retains `event=probe_logger_ready destination=stderr`. Existing logs observe
Playback token11/session878229120356/generation1 progressing from Running/gate1
to Paused/gate2 while still open and admission revoked. The transport then
attempts close and retains the revision refusal. A later lifecycle observation
sees the same generation at Closed/gate5/open, and its close succeeds at gate6.
MIC subsequently closes separately. Log order is observational, not atomic
cross-thread timing or evidence that these snapshots bracket the refused call.

The decisive single `finish_close` confirmation QUERY and its acceptance/refusal
decision are not logged. Changed-stream, still-open and intervening gate-epoch
cases must not be collapsed into successful cleanup. Existing source already
forbids refreshing SET_GATE to close a newly admitted stream. No such guard is
weakened, no retry added and no error suppressed. The next scoped source slice
adds only non-RT diagnostic request/confirmation/result logging, then verifies
the exact refusal before considering a behavioral repair. Replaying equivalent
audio acquisitions without that additional evidence is not the next step.

## Integrity and remaining gates

Root verifies20 payload files plus one manifest per target archive by length
and SHA256; independent base review also verifies each ZIP CRC and loose bytes,
strict decoder outputs, all3033 metadata rows and parameter/report consistency.
Independent debug review separately verifies all ZIP CRCs, payload/manifest/loose
byte equality, PCM statistics, failure/report consistency and catalog snapshots.
That debug review does not decode ETL or verify trace loss. Host-derived marker
summaries are not target payload members.

| Artifact | SHA256 |
| --- | --- |
| Base ZIP,517114 bytes | `CF642B88A7C9515B2F9102CC4C3740E860B9E2D58EEDB13B08DE1BBC5D993CDE` |
| Debug ZIP,521492 bytes | `7A17B5B8E3C13C4D8BD5765DDC3521ED7C16E9C78490440500083BE4AA695A04` |
| Base PCM | `48126ADFB3E41F499BF6EEA08FC9E262D235885F486ACBA4942B6A49DEB532A2` |
| Base packet metadata | `5D4C6506365B222BB653B8D7CCE7E70A4747E54DC93AC48C457FE93B43428D7C` |
| Base collection | `60001F15BB94366AC3C5904B38368F397183FE61089A010BAEC00A8F53387E8B` |
| Base report | `4BEE506835C29456F62A3056177B200D17CC0F139476DDFE466B9914C989689D` |
| Base ETL | `0ECBE7FD52F2DDCE21DEB4C4249B2A749201E0A4E82F2257986DB6798B22E168` |

Base independent receipt is `independent-evidence-audit-20261031.json`, SHA256
`E2EAC042B8CE9FB958E3BC2EF31E0DCCE560701CE1164BEA5E5E354E8EDEA7B1`.
The sibling debug root retains its own independent receipt of the same name,
SHA256 `169300BFB18AEAECC43F088A261E3254DCC65E753767EE80E6CD31AE1884EE3F`.
This evidence advances exact-route diagnosis, not S1 acceptance. S2 lower-period
stability, S3 sustained observed-delivery admission, native app acceptance,
matched Process/VAD physical comparison and calibrated sub-10-ms output remain
open. No lower operating target/default or VAD latency winner is selected.
