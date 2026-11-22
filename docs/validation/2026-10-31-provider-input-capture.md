# Provider READ input waveform discriminator

Date: 2026-10-31, Asia/Shanghai. Source `b0a0223`, physical target
`LAPTOP-NP49RG6H`, existing pinned SSH route `frp-rug.com:14857`.
This executes the bounded [capture plan](2026-10-31-provider-input-capture-plan.md)
after the [independent Process control](2026-10-31-ms-process-reference.md).
It is not S1, waveform continuity, physical latency or a smaller-buffer pass.

## Direction and implementation boundary

Virtual speaker/Playback remains Moiren Input/Source; virtual MIC remains
Moiren Output/Sink. PR43/44/45/47 are already merged separately into main
`6b4ffe6`, which is contained by this experiment. Draft PR26 is not merged.

The explicit holder-only `--input-capture-frames=480000` reserves raw bytes
and at most4096packet records before native transport. The existing Playback
READ worker copies accepted, epoch/admission/order-validated bytes immediately
before `PcmPlaybackBridge::decode_packet`. It adds no native READ owner,
Process recorder or physical render client. Normal `windows()` remains
uncaptured; opt-in f32/f64 backend factories and the lifecycle adapter share
one discovered provider. The result handle owns only detached evidence,
not the provider/control file.

First accepted data locks actual native-owner format/channels, stable ID,
five-part native stamp, reset epoch, admission ticket and active revision.
Capacity/identity change seals rather than stitching generations or skipping
startup zeros. Whole-packet capacity is not an audio drop. Actual worker join,
not cancellation/drained, enables take-once export. Outer service failure
does not suppress an already-joined artifact or replace its original error.
Later preparations run uncaptured and cannot fail merely because this
single-shot diagnostic was previously reserved.

## Software verification and deployment

The integrated source independently passes103provider tests,116holder tests,
strict audio/holder Clippy, formatting, Linux-target audio library/test
checking and the Release holder build. New cases exercise actual pump
PCM16/Float32 raw bits, zero/nonfinite preservation, stale exclusion,
reset/revision sealing, malformed/short data, carrier failure, early failure,
join/panic/take-once, enabled/disabled deterministic carrier/IO parity and
preallocated4096record-bound operation with no allocation/deallocation.
Independent source review closes the duplicate-reservation behavior defect.
The shared counting allocator moves only within the Windows library-test
binary; no production allocator changes.

Same-provider wiring is source-reviewed, not a dedicated same-instance
factory test. Error cleanup pieces are tested separately; an end-to-end
service.join-error/PCM-already-joined persistence regression remains absent.
Linux-target checking is not an actual Linux execution of Windows code.

Runner guards retain genuine RED then36GREEN fixtures under Windows
PowerShell5.1. They reject inverted/missing MIC directions, corrupt owner,
format/frame/byte/offset/sequence/QPC metadata, unknown join, open seal and
pending publication. Raw QPC ticks, not converted100ns values, define a
conservative source-active transfer window. Empty/no-overlap remains unknown.
Host PCM analyzer retains genuine numeric-conditioning RED then14GREEN;
an unresolved normalized correlation cannot be clipped into a false winner.

Only the diagnostic holder is deployed side-by-side; installed driver and
production app are unchanged. The holder's no-driver `--help` succeeds.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `vad_integration.exe` | 7421952 | `3D66F7518BCF57924505D9B80BF592CFF9BCE64B3D106F4AB182C558792F7ABD` |
| frozen runner | 35089 | `E51C2FA7C9C29B7256A7A994D53CDEB815874647A56BEC0B9F093E4ED8AFC952` |

The holder uses its existing Playback -> Gain0.5 -> MIC fixture, not exact
Input -> unity Bus -> physical Output. No kernel ABI, source payload,
routing/default, buffer, pacing or recovery policy changes in this slice.

## Native source and captured PCM

Run `provider-input1` spans07:03:15 to07:04:19 local. Existing source worker
`0fd3fbf` has PID11128/creation134378750032608222 and targets the exact owned
Shared Playback MMDevice. Twenty contiguous1second xorshift markers command
stereo amplitude0.025. Source Stop/join succeeds while its schedule remains
incomplete:585120acknowledged/nonzero-source frames plus1056silent preroll,
586176submitted total. Start-before to Stop-after is12373.1979ms, below the
20second schedule. These receipts establish submission, not played PCM.

Saved provider data is48kHz stereo PCM16, unlike the Float32 source client:

- 1000packets,480frames/1920bytes each; sequences1..1000 without gaps.
- 480000frames/10seconds,1920000raw bytes and960000scalar samples.
- All960000samples are exactly zero; both channel peaks/RMS are zero.
- All1000packets report subsequent carrier publication; exclusions are zero.
- Identity is stable `MoirenAppVadPlay`, native `[7,763483864242,7,1,1]`,
  reset0/admission2/revision2; frame capacity seals this one generation.
- Worker outcome completed and actual worker join confirmed before export.

At10MHz QPC,991packet transfer stamps lie strictly between the Running
receipt observation-after763537666785 and Stop token-before763658408874.
First/last packet transfer ticks are763536829795/763636731099. The first
nine packets outside the conservative window remain in the original artifact;
no startup sample is removed. A transfer callback is not the first sample's
capture time, source origin, full-packet overlap or a calibrated delay.

Raw SHA-256:
`124617C1F65E92D3BC895FBD869E4BB16A30754B198F59E6E973949B9AAA1B01`.
Metadata SHA-256:
`F24701CBFA9CD93E84F271A86EC5AE5F001C7CB185EA5AA00EF9D5407322D063`.
The host checks the entire original PCM, not only prefixes/hashes. Template
search returns unknown_zero_energy, no correlation/offset or recovery pass.

## Meaning, cleanup and archive

This places the all-zero observation at or before the real provider READ
boundary. It cannot be explained solely by a later decode/carrier/Engine
stage discarding an otherwise recovered nonzero source in this run. It does
not prove source waveform delivery into the Windows render stream, a sole
driver/Windows/session cause, or that the independently silent Process path
has the same cause. Session0/no-interactive-user remains an unproven confound.
The next useful discriminator is before this boundary, not another equivalent
Process recording or downstream output variation.

Collector/holder/source exit0, no forced termination/errors/residual owners.
Intermediate progress explicitly retains input_capture.error=not_joined_unknown
until join/export; that pending diagnostic is not relabeled as an audio fault
or erased from its saved snapshots. The terminal capture error is null.
Holder runs60727ms, completes60003ms idle/59windows, zero recorded timeline
glitches or epoch changes. Service joined/control released are true;
endpoint_absence_confirmed remains false in its public report. The separate
final PnP audit finds only healthy `ROOT\MEDIA\0001`. Installed SYS hash remains
`AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268`,
driver `6d9e32f/oem201.inf/3.1.1.607`; boot remains
`2026-10-30T09:50:49.659845+08:00`. No target build overlaps acquisition, and
no speaker/default/privacy/autologin settings, caches or originals change.

Host evidence: `D:\coding\moiren-evidence\20261031-provider-input-capture`.
Remote originals:
`D:\MoirenTest\20261031-provider-input-capture\runs\provider-input1`.
Archive `provider-input-capture-20261031.zip` contains37payload members plus
manifest,10812478payload bytes/2922079ZIP bytes. Terminal owned exits and
provenance are checked before archiving. Target/host verify every member's
length/hash plus embedded/external manifest, count and total; host compares
the separately downloaded originals. Originals remain.

- ZIP SHA-256: `AF49E5B835FF8C6F1464CB0CBF7180B77357182EAAF96F645807D43179EE5EB7`.
- Manifest SHA-256: `FC7CAAAF12903776BCBBFFB608BF19D024F1EEFA54E98A994DF5BCFE0C7BD71E`.

S1-S3, stable calibrated p95<=10ms, lower-buffer defaults and a matched
VAD/Process latency comparison remain open; collector exit0 is not acceptance.
