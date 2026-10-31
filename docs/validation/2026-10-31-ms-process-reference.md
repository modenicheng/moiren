# Independent Process capture: bounded Microsoft-derived diagnostic

Date: 2026-10-31, Asia/Shanghai. Target: `LAPTOP-NP49RG6H` through
`frp-rug.com:14857`. This control does not qualify S1-S3, continuity, a smaller
default, calibrated physical latency or a VAD/Process winner.

## Purpose and reference provenance

The previous [source-process control](2026-10-31-source-process-control.md)
records all-zero Float32 PCM while the exact source remains active. A separate
implementation tests whether the observation depends on Moiren's Process
recorder. It is not a matched-format implementation A/B: the reference uses
44.1 kHz PCM16 rather than 48 kHz Float32.

All eight immediate `Samples/ApplicationLoopback/cpp` files and root LICENSE
from `microsoft/Windows-classic-samples` commit
`434f6002bdf9cf9829406c3ff2b33387982d6168` are retained byte-for-byte.
The exact upstream WIL dependency, `1.0.210204.1`, is retained. The unmodified
sample builds on the development host but is never executed: review finds
unsafe SILENT-pointer handling, advisory rather than actual packet length,
unbalanced error-path leases and misleading ordinary exit0 behavior.

An explicitly separate evidence-only diagnostic fork repairs those packet
boundaries, retains first asynchronous/Stop/finalize failures and emits one
parseable final summary. It uses a heap WRL COM owner so late callbacks can
retain the parent; cancellation/completion signaling is not a callback join.
Stop/sample/finalize use the same lock and late Stopped callbacks cannot write
payload. Individual packet length is bounded by8MiB; total data is bounded by
8MiB minus4096. Activation,44.1kHz stereo PCM16, Shared flags, MF work queue and
fixed ten-second main sleep remain upstream behavior. Activation/Stop waits
remain internally unbounded and require an external owned-process deadline.

Extracted real packet-path tests retain upstream RED11cases/21failed
assertions and cumulative-cap RED3failed assertions. Final22packet/summary
cases pass under `/W4 /WX`; x64 Release build succeeds. Independently tested
runner-summary guards pass116cases; strict RIFF parsing passes7cases. These
are host checks, not native callback-interleaving or signal qualification.

Reviewed diagnostic binary:57,856bytes, SHA-256
`AE6D7FEEC8ECCBAB80D34482BF634C2A794009E986C13290F97FE8FB22921A9F`.
Only that binary is deployed side-by-side as
`D:\MoirenTest\20261031-ms-process-reference\bin\MoirenProcessReference.exe`;
the existing VC runtime is checked, with no global installation.
Frozen runner SHA-256:
`1E33D78140C48D243F6FB38B5AA439C0AEB0354BE63765BC478EB265DC8C764D`.

## Exact native boundaries and source overlap

`ms-source-process1` uses unchanged driver `6d9e32f`/`oem201.inf`/`3.1.1.607`,
holder `ac93295` and source `0fd3fbf`. The source renders Shared to the exact
service-owned virtual Playback selected by root/stable ID/direction/confirmed
revision and matching native identity, not default or friendly name.
Playback/virtual speaker is Moiren Input/Source; MIC is Moiren Output/Sink.
The active holder still runs Playback -> Gain0.5 -> MIC, not graph-free or the
exact S1 unity-Bus fixture. No Process graph, DAWN or physical client starts.

Source PID10092/creation134378724090585752 is Running before recorder launch.
The retained source process handle pins its creation while the reference
selects PID10092/includetree; the sample itself has no creation-time pin.
Recorder exits before source Stop publication starts by61.964ms.
Source start-before to Stop-after spans11,934.4503ms, below its20second
schedule. Final source playback is11.7502413seconds,564000acknowledged
nonzero-source frames plus1056silent native preroll =565056submitted frames.
The20contiguous one-second markers are not exhausted. Source exit or schedule
exhaustion therefore does not explain this recording. Submission still is not
proof of played PCM or a sample-by-sample source/recorder clock mapping.

## PCM result: not intended waveform recovery

The reference completes997packets/439677frames,9.97seconds of44.1kHz stereo
PCM16. The WAV is1,758,754bytes with an18-byte fmt chunk and1,758,708-byte data
chunk; independent RIFF parsing validates declared lengths and alignment.
Native summary and independent host scans agree on201805nonzero samples.
The complete879354sample histogram is only:

| PCM16 value | Count |
| --- | ---: |
| -1 | 100901 |
| 0 | 677549 |
| +1 | 100904 |

Peak is1LSB/0.000030517578125; normalized RMS is0.00001461955503332823.
There are336848adjacent scalar changes and no all-zero stereo run longer
than16frames. An independent channel audit finds168413unequal-channel
frames and L/R correlation approximately0.00735, while the configured source
uses identical-channel markers. These tiny differences do not establish
recovery of that waveform. The peak is approximately58.27dB below the commanded
source amplitude0.025, not a measurement of path attenuation. Conversion/dither
is only a hypothesis; it is not an established cause.

Native summary retains zero SILENT packets/frames, discontinuity/timestamp
flags, release failures and partial writes; all four start/async-start/Stop/
finalize HRESULTs succeed. API/packet completion and unflagged tiny PCM do not
prove continuity, correct amplitude or intended-source linkage. Neither the
earlier Float32 zeros nor this different-format diagnostic establishes a sole
recorder, driver, source, session or Windows API cause. A useful Process
positive control recovering the intended source waveform remains absent.

- WAV SHA-256: `2E1B7E01B97454350850FB503E160E8484F7D07EA6DD6E7DBDCE0B9ACC9C3537`.
- Data SHA-256: `C30F15894B5444C9298CCAC3B875FF6F8BD73D41DBFF17B60080A6909F0AF747`.

## Cleanup, CI and verified archive

Collector/holder/source/reference all exit0 with no forced termination,
retained errors or owned process. Holder reports passed after60599ms,
60000ms idle and59timeline windows, no recorded timeline glitch/epoch change.
Recursive independent audits of the full report, seven checkpoints and14
progress snapshots find no nested errors. Service joined/control released
are true; holder endpoint_absence_confirmed remains false. Final PnP separately
finds only healthy `ROOT\MEDIA\0001`.

Boot remains `2026-10-30T09:50:49.659845+08:00`; installed SYS SHA-256 remains
`AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268`.
No target build overlaps acquisition; no driver, endpoint setting, speaker
volume/default, privacy, autologin, cache or original evidence is changed.
Independent CI run `41132992563` at exact `af00457` completes success with20
jobs; it is not restarted or substituted for native acceptance.

Host evidence: `D:\coding\moiren-evidence\20261031-ms-process-reference`.
Remote originals:
`D:\MoirenTest\20261031-ms-process-reference\runs\ms-source-process1`.
Archive `ms-source-process1.zip` contains38payload members plus manifest:
3,066,359payload bytes excluding manifest,244,153ZIP bytes. It includes the
complete run, frozen runner and exact diagnostic binary. Terminal collection,
owner exits and frozen-input provenance are checked before archiving. Target
and host verify each member's length/hash, count and total; host compares all
38members with independently downloaded originals. Original files remain.

- ZIP SHA-256: `8024E668380ED174BF2FDFB4FB8F539A9962FAD2C6AEF0D4700FDB77A3160EDF`.
- Manifest SHA-256: `929BC5E8FCFBFF5E81C0E6D8617E1B1FED788A7AEACE3544ACF47602DEABFAF5`.

Do not repeat equivalent headless Process variants or promote1LSB noise into
a waveform pass. The next useful boundary is actual encoded/source or
provider/native PCM, with explicit lifetime, format and capacity controls;
any new observation must distinguish submitted data from recovered waveform.
