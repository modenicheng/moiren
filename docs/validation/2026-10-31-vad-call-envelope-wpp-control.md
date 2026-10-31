# VAD decode-call envelope with existing WPP, 2026-10-31

## Frozen comparison and verdict

`dawn-144-e32-t881-2` repeats the [first decode-call control](2026-10-31-vad-decode-call-envelope.md)
with the same source, binary, driver and settings. Only existing driver WPP is
enabled. The Input -> unity Bus -> DAWN route keeps 48 kHz stereo Float32,
actual Exclusive render period/capacity144, Engine32, target881/capacity4800,
declared packet/gap480, jitter96 and recovery cap4. Playback/virtual speaker is
Moiren Input/Source; MIC is Output/Sink.

The305.0073755-second bounded diagnostic completes without a reported live
reset, recovery, xrun, drop, steady-state prime or native/host cleanup error.
It does **not** reproduce run1's live reset or four `0x8007051A` cleanup errors.
It remains unqualified: no retained PCM, calibrated independent physical
reference or controlled load is acquired. A finite fault-free diagnostic is
not continuous-waveform acceptance or evidence that tracing prevents a fault.

The unchanged probe returns1 for the unqualified diagnostic. Controller0,
probe1 and wrapper1 remain separate. The wrapper records
`Probe failed/unknown; controller exit0 does not mean probe success`; this
orchestration failure string does not establish a native/audio failure. The
report has `native.error=null`, backend `failure=null` and clean final host
teardown, unlike run1. No exit code or failure receipt is rewritten.

## Identity and actual observations

Root evidence directory:
`D:\coding\moiren-evidence\20261031-vad-call-envelope-native`.

- Probe `afaef5e51aa5fe0a23964899212d30c15184d4d8`,7238144 bytes, SHA256
  `4282DD7F488666CCEAC4EE421F6C17A0BDD14331B64FC469F29115E02013E507`.
  The later exact-writer fix is not in this binary.
- Installed driver610/source `883a5be0a66afa03ef5d2beb2d3c4063cda1bda6`,
 175000 bytes, SHA256
  `94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
  No install, reboot, error-policy or buffer change occurs.
- Unchanged runner SHA256
  `189F16E87E271D084037C8F221E122C6F614BD6319E506A3167F084221725EEF`;
  repeated-v1 source SHA256
  `96C45ACF1AC4ACE6BC793FBF76C3726F7E0B5AC7E521711D0EC4A03D7CCE4D9E`.
  This is300 adjacent one-second patterns, not unique indexed-v2 codes.

The61 observations end at300.3408774 seconds. All sampled live gate epochs
remain1; original native stamp is `[16,1116089887260,16,1,1]`, admission ticket2,
active revision1, reset epoch0. The final live snapshot retains101666 accepted
decode calls,14639904 represented frames and101665 eligible intervals. No
ambiguous/full/error/stale calls, owner reopens, segment breaks, unavailable/
regressed clocks or saturation are reported for that live summary.

The first-before/last-after QPC frontier is1116094606830/1119144556183 at
10000000Hz, spanning304.9949353 seconds. Maximum eligible call envelope is37080
in100ns units, or3.708ms; completion maximum is separately37075, or3.7075ms.
Neither number is physical latency, a carrier-publication effect, consumer
whole-wake coverage or a bound including the unobserved prefix/suffix. The
summary still has `carrier_publication_effect_known=null` and
`complete_delivery_coverage=null`; a successful decode call does not repair the
known exact-writer abandonment race in the frozen binary.

Live QUERY and packet counters retain dropped0, with epoch reset0/recovery0.
The boundary retains initial-prime muted32, recovery-prime muted0,
short-read callbacks0 and xruns0. Those counters do not substitute for PCM.
Cleanup separately retains101667 calls, reset1/recovery1 and invalidated original
identity. The final provider is Removed/Closed, gate6; Engine is Dormant/Closed,
`cleanup_error="None"`, and backend failure is null. The source reports Stop
success, joined worker and no failure;14400000 acknowledged source frames mean
ReleaseBuffer submission only, not delivered waveform.

## Owned trace and offline decode

Task-scoped session `MoirenVAD-20261031-call2` uses the existing WPP provider,
flags0xf/level4, perf clock and a sequential, noncircular16MiB maximum. It is
started once, ownership retained before fallible evidence writes and stopped
once in finally. Stop exits0 with no helper timeout or evidence-read error;
the terminal ETL is40960 bytes, SHA256
`A5D106740A7A88ED468A261462806676158A3B19322B95797F238A9A817CB42A`.
Root separately confirms the owned trace name absent after stop. No unrelated
session is stopped. Prior trace-name inventory is not an exhaustive audit of
providers enabled by other sessions; that limitation remains.

Host `tracefmt.exe`10.0.26100.0 x64 decodes the frozen ETL using the hash-pinned
610 TMFs under `20261031-mic-no-copy-witness/tmf-883a5be`, not a newer build's
symbols. Matching PDB1380352 bytes has SHA256
`F5D63D11D6C06E388FA76C53CE9F3DA39AD065ADFB22ECD66F568F5C4C680EF7`.
Command/output/stderr and summary are retained in `run2-trace-decode`.
Decode exits0:123 events, reported loss0, format errors0, unknown formats0.
No ClockRebase message is present. This is consistent with the retained live
reset0, not proof of every callback, absence of every scheduling fault, or the
cause of run1. Existing native/copy witnesses are bounded and explicitly retain
`all_interval_coverage=0`; they do not cover the whole305-second waveform.

## Acquisition integrity and terminal target state

The unchanged audio helper creates17 payloads plus manifest,76533 ZIP bytes,
SHA256 `7DC1314E313AFF8046848E1E29F8B868F248B78D2EC2C15266ECDB2F7E536DE7`.
Root verifies archive CRC, exact member set, every length/hash, frozen local
inputs and provenance before extraction to `dawn-144-e32-t881-2-verified`.

The separate WPP ZIP has17 payloads plus manifest,18104 bytes, SHA256
`4CC52484520FD59CF82E945145F175B0158597FB53A72455A67FD14D09A26DE1`.
Manifest6191 bytes has SHA256
`8EB0B6FC83179B47E06E4B94C652E97D5976981E3006E22844B8FB0786ADAE6F`.
It retains all16 trace-directory files and the exact wrapper, including empty
logs. Original runner-result bytes link both archives and the wrapper hash.
The host-only verifier's21 fixtures pass, retaining original RED outputs;
independent actual-archive verification records `closed_present_unqualified`,
no missing members and byte-exact result linkage. It does not compare every
downloaded member with remote loose originals or confer audio acceptance.

The first inline archiver command exceeds the remote command-length limit
before execution; read-only audit confirms no manifest/ZIP/script then exists.
That failed/empty receipt is preserved. Root stages the4979-byte script instead,
verifies SHA256 `4E454337234504063FA710D3CCC59C855A3EB9F1BBC1FA394BE8625391B83F5A`,
then executes it once. The successful receipt is
`dawn-144-e32-t881-2-wpp-archive-receipt-staged.json`, not the empty original.

Fresh target audits confirm unchanged boot, driverRunning/rootPnPOK, no audio
owners/dynamic VAD children, no default-role/render-control changes and about239GB
free on D:. No cache cleanup is needed or performed. Source/endpoints and all
run1 failure evidence remain intact.

## Next boundary

Do not reduce declared gap480/jitter96 from this call-envelope maximum. Actual
complete consumer-wake demand and publication/withdrawal coverage are separate
missing boundaries. Merge the independently tested exact-writer correctness
repair only after freezing this old-binary receipt; it cannot retroactively
change run2 semantics. Physical common-reference calibration, acquired waveform,
controlled load and matched Process/VAD conditions remain open.

Stable sub10ms, device-limited impossibility and a physical theoretical lower
bound are all unestablished. The user's fallback remains evidence-gated; the
current declared-policy target881 is not a physical floor or a promoted default.
