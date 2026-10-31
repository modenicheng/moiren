# Exclusive Playback PCM16 positive control - 2026-10-31

## Verdict and scope

On physical target `LAPTOP-NP49RG6H`, installed diagnostic driver `2a5aee5`,
`oem203.inf` / `3.1.1.609`, passes the existing four-case Exclusive event
Playback probe. The intended stereo PCM16 case actually negotiates 480 frames
at 48 kHz and returns matching nonzero PCM through Direct I/O. The bounded
native event-buffer observer and original READ-copy observer both retain
nonzero PCM16 prefixes from that case.

Virtual speaker/Playback remains **Moiren Input/Source**, receiving other apps'
rendered audio. Virtual microphone/MIC remains **Moiren Output/Sink**, supplying
other apps' capture. PR44 already contains this direction correction; main
`6b4ffe6` is an ancestor of the tested experimental branch. PR26 is not merged.

This is a direct native-client/READ control, with no Engine, Bus, MIC stream or
physical render. It is **not S1**, a changing-marker continuity test, a
mode-only A/B, a Process comparison or physical latency qualification. It does
not invalidate the earlier Shared-source zero observation or identify its
cause. Session 0 is compatible with this particular nonzero Exclusive control;
that does not establish Shared/Process behavior or an interactive-session cause.

## Provenance and execution

Evidence root: `D:\coding\moiren-evidence\20261031-exclusive-playback-control`.
No new build, driver install, reboot, trust change, default-role write, volume
write or physical output client occurs. App `af70aa9` remains deployed unchanged.

Retained probe, freshly length/hash-attested on the target:

- `D:\MoirenTest\20261031-c985fc3\bin\vad_native.exe`.
- Source `0fd3fbfddf1ded6180ab8630d07250cf755879b7`, 551424 bytes.
- SHA256 `FED8891CFC344D0EC2ECA657992B35CC13F5E05B007A0C29A86F9828FF9C63F0`.
- Installed SYS SHA256
  `C9F7FF8273239B97436DF75A5D9E7584347F54A79CCFA21E1889EE546E4C591F`.

Command: `vad_native.exe --exercise --intent=playback --seconds=2`.
No `--timer` or `--minimum-period`. It creates all four owned virtual endpoints,
but exercises only Playback; cleanup removes every owned child.

An initial detached supervisor leaves no script-entry receipt or native process.
A subsequent synchronous preflight refuses because another task is compiling
`ui_capture` in `D:\moiren\.worktrees\perf-platform-frame-pacing`. That build
is not stopped. After a fresh idle audit, one native acquisition runs over a
kept-open SSH session; observation timeouts do not terminate audio. These
preflight attempts are not native-test failures or passing acquisitions.

Actual acquisition: UTC `2026-10-31T02:00:45.0674845Z` through
`2026-10-31T02:00:57.5986803Z`; probe PID3976, confirmed exit0,
11.7577148 seconds. Native report schema3 and all four cases validate.
Trace start/stop and catalog exit0; no runner/cleanup errors, remaining audio
owners or present VAD children. Root PnP remains OK and service Running.
Boot remains `2026-10-30T09:50:49.6598450+08:00`. Complete catalog observations
retain unchanged render mute/volume and all six default roles.

The runner's three review findings were corrected before native acquisition:
aggregate exit/report validation, independently retained cleanup failures and
unknown-safe catalog comparisons. Root reruns all52 independent PS5.1 fixtures;
all pass. Runner SHA256:
`316B4A260D49D8D9F13E50F6F152E6A0B64978D1EE649B4092E370DEBB2FCD51`.
Four app graph VAD direction/binding regressions pass freshly.

## Actual case results

| Channels | Format | Native/driver frames | Matching signal frames | Observed silent frames |
| --- | --- | --- | --- | --- |
| Mono | PCM16 | 480 / 480 | 95040 | 0 |
| Mono | Float32 | 480 / 480 | 95520 | 0 |
| Stereo | PCM16 | 480 / 480 | 95040 | 0 |
| Stereo | Float32 | 480 / 480 | 95040 | 0 |

All query default100000/minimum30000 hns, request100000 hns, without alignment
retry. These are period/capacity observations, not measured end-to-end latency.
The probe allows silence and requires only48000 matching frames per two-second
case. Its passing result therefore is not a continuity acceptance. Unobserved
time, dropped deliveries or non-exported faults cannot be inferred absent from
the zero observed-silence count.

## Stereo PCM16 native and READ evidence

Stored identity: endpoint6/session869953863456/generation5/gate13;
PCM16/stereo/480/align4, QPC10MHz. Native sealed revision199 and READ sealed
revision200 both pass the existing frozen strict decoders. Decoder SHA256 is
`811893FEB2069D3A1D3109B080E88FE14991DB70A197A4A729CB291185D71E2C`;
READ decoder is
`C1825BCC4371AAD32A6097E83D40AAD60B48FBB8AB479B2D848D176032D8FED5`.

- Actual native path EVENT2, PacketCount2, packet1920 bytes, allocation4096,
  RT sizes1920/1920 and offsets2176/0; no mapped timer extent.
- Native scans198 calls/sequences2..199, 95040 frames/380160 bytes. All scanned
  frames are nonzero, 190080 nonzero bytes. Sequence gaps0; no counter saturation.
- READ scans199 accepted copies/sequences1..199, 95520 frames/382080 bytes.
  All scanned frames are nonzero; source/destination differ in0 bytes.
- Native first/last callback QPC870023074012/870042774669; READ
  QPC870022975017/870042774669. Exact selecting READ witness matches. Callback
  ranges overlap, but there is no exact per-sample alignment or all-interval
  coverage. Neither observer reaches its480000-frame lifetime cap.
- Both seal on identity change before the later Float32 stream; do not relabel
  either record as Float32. First/last exported native EOS flags0; intermediate
  EOS states are not exported individually.
- First native and READ prefixes repeat `00 18 00 E4`: PCM16 L6144/R-7168,
  corresponding to the probe's stereo constant L0.1875/R-0.21875. Native EVENT
  destination duplicates its one live-buffer read, not an independent copy.
  The separate READ observer does retain post-copy paired equal regions.

Matching private-PDB tracefmt reports292 events,0 lost,0 format errors and0
unknowns. Strict text decoders intentionally keep `trace_loss: unknown` and
`qualified: false`; their input alone does not attest ETL provenance. Native
and READ observations are bounded diagnostics, not raw continuous PCM files.

## What this changes

Unlike the earlier Shared Float32 changing-source fixture, this Exclusive
stereo PCM16 client supplies exact native-format PCM with a nonzero full-buffer
prime before Start. It also changes waveform, native client mode, endpoint
ownership and source pumping. The event packet shape/layout matches the earlier
PCM16/480 observation, but that alone does not make the runs matched A/Bs.

The current installed driver's event/READ PCM16 boundary is therefore capable
of passing this exact nonzero control. A universal PCM16/480/layout/session0
failure is not supported. The earlier Shared nonzero Release -> zero consumed
event-buffer interval remains unresolved. Next isolate the source/stream route
with a changing waveform and exact IO/Bus, keeping format/sharing/prime changes
explicit rather than repeating the same zero fixture or promoting defaults.
S1-S3 and calibrated sub-10-ms acceptance remain open.

## Archive

Target originals are retained. Host ZIP verifies length/SHA and all14 payload
members against the archived target manifest; the15th member is that manifest.
Independent review also verifies all member CRCs, byte/hash equality against
loose originals, both frozen decoder outputs and receipt/status consistency.
Its verdict is no blocking finding, with the limits above; it does not repeat
ETL/private-PDB decoding or attest unobserved intermediate state. Receipt:
`independent-evidence-audit-20261031.json`, SHA256
`23C45C37D2C1EA82CDA3491CFD1CA2B14B08965251DE89F5BB0DAF879C1B360C`.

| Artifact | SHA256 |
| --- | --- |
| `exclusive-playback-target.zip`, 18590 bytes | `6129F933456E6D280B742DC4B505516454EFB62451B433A499F9F60518F18959` |
| Native report `probe.stdout.log` | `BB13C39DB64D40EAEA6D95A4F0A2806A4D5E2426CC2BDE7384652730E0E95762` |
| `playback.etl` | `8F1FEA6A9BFDE1C8ACDF312DCDD625065F669A6A7C42362F64550F93A4A86189` |
| `result.json` | `41CC41AFF7AA6A40E4B8DC65554DD7172DA54FD55EFC8954DF37E4CEF56F353A` |
| Host `tracefmt.txt` | `B09F8AF98BBAEF50A07CB0540BB319E07B0614D96733FBC3E6DFFC8C6CF5E850` |

No target cleanup or Cargo cache removal is needed; preflight D free space is
241963077632 bytes. Existing driver608/607 rollback and unrelated work remain.
