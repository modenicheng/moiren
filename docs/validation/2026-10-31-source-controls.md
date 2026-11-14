# Exact source-client controls and zero provider READ

Date: 2026-10-31, Asia/Shanghai. Diagnostic continuation of the
[native source write witness](2026-10-31-source-write-witness.md), not S1-S3
or physical latency qualification. Virtual speaker/Playback is Moiren
Input/Source; virtual MIC is Moiren Output/Sink. Mapping does not change.

Before adding kernel instrumentation, inspect a cheaper possible confound:
master mute/volume on the actual source's existing client and exact device.
This does not call the broad catalog, enumerate other sessions/devices,
create another audio client, or write volume/default/privacy settings.

## Implementation and checks

Source `e459526` plus module-order-only `08ed599` integrates as `24c5184` and
`b6b26b7`. The existing worker-only `--source-write-witness` also enables one
optional `final.playback.source_controls.snapshot`. Default-off behavior
does not acquire diagnostic interfaces or call these getters.

After Initialize, the same owner thread acquires IAudioSessionControl and
ISimpleAudioVolume from its existing client, and IAudioEndpointVolume from
the already identity/flow-validated device. Only after successful Start and
first nonzero Release, with committed source progress, it reads session state,
session master/mute and endpoint master/mute once. No render lease or publication
mutex spans a getter. Each getter checks Stop first; no retry or fallback.
Original signed HRESULTs and null/unknown values are retained, including
successful acquisition with a null interface. No session PID getter is used;
the existing client is the session binding, with exact source process creation
and MMDevice identity retained. COM references never enter the exported DTO.

Preparation and completed snapshot are published immediately, so later audio
or Stop failures do not overwrite them. Synchronous COM getters may hang or
perturb servicing: bounded call count is not a hard native shutdown deadline.
Sequential getter readings are not an atomic control snapshot. These facts
remain explicit in help, runner and inference; no quality fault is waived.

Retained behavioral REDs precede 5 focused GREENs. Root independently runs all
543 example tests and strict example Clippy successfully. Initial formatting
fails module order; the focused style commit fixes it and fresh fmt passes.
Linux-target example check and Release build pass; this is not Linux execution.
Independent source review finds no blocker. Tests cover real observation order,
QPC brackets, disabled preparation, getter failure/null, stop-between-getters
and retained snapshot plus original later audio error. Native COM concurrency
and hard getter completion are not proven by those tests.

Frozen PS5.1 runner has 142 passing fixtures and a separate independent helper
review. It validates exact identities, raw signed HRESULT/null consistency,
Release-to-snapshot ordering, committed counters and activation-to-Stop envelope.
The snapshot need not follow the later Running-receipt observer timestamp.
Its 20 inherited helpers remain byte-identical to the frozen predecessor.
Missing/muted/zero controls stay unknown/potential confounds, not audio acceptance.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| new worker `b6b26b7` | 7064576 | `CE4EEDF7FD57FFB88D4B796F46CCB253312F35C26F1F3223B4680A16AF38B9D4` |
| unchanged holder `b0a0223` | 7421952 | `3D66F7518BCF57924505D9B80BF592CFF9BCE64B3D106F4AB182C558792F7ABD` |
| frozen volume runner | 59963 | `E43481217937122F07C194A7B4C1933FCDA7707F71DE5CCC835D1C271AA87F33` |

Only the diagnostic source worker changes side-by-side. Installed driver
`6d9e32f`, holder, production app, Playback -> Gain(0.5) -> MIC fixture, 20-second
schedule and 12-second observation remain fixed. No target build overlaps
acquisition; no physical client or Process recorder starts. This is not exact S1.

## Actual target observation

Run `volume1` spans 08:08:04.965 to 08:09:07.816 target-local time on
`LAPTOP-NP49RG6H`, not a synchronized cross-host wall-clock interval. Source
PID 7608/creation 134378788912061053 addresses the exact owned Playback endpoint
`{0.0.0.00000000}.{c9c8ce6a-634f-4027-9138-a18d5982b89f}`.

All three interface preparations, process-creation read and five control reads
return HRESULT 0. Session state 1/Active, session master 1.0/mute=false and endpoint
master 1.0/mute=false are observed; no control issue is retained. At that point
source progress is 480 acknowledged/nonzero frames plus 1056 preroll, 1536 submitted.
At 10 MHz QPC:

- Nonzero 16 B Float32 destination equals encoded `CD CC CC BC` repeated 4 times,
  source frame 0/native frame 1056, flags 0/HRESULT 0, Release return 802432386877.
- Controls before 802432386880, after 802432387669, Stop not yet requested.
  The 78.9 microsecond bracket is getter observation time, not audio latency.
- Running receipt observation-after 802433532492 occurs later. The controls
  are inside the activation-to-Stop envelope, not claimed inside that later
  conservative source-active observer window.
- Final 582720 acknowledged/nonzero frames plus 1056 preroll gives 583776 submitted;
  schedule incomplete, event timeouts 0. Start-before to Stop-after 14132.9941 ms
  is below the unchanged 20-second schedule.

Provider READ still retains 480000 stereo PCM16 frames/1920000 bytes, with all
960000 scalar samples exactly zero. There are 1000 contiguous 480-frame packets,
sequences 1..1000, 1000 subsequent carrier publications and no stale/epoch
exclusions. All 1000 transfer callbacks follow witnessed Release; 990 also lie
strictly inside source-active observation 802433532492..802553704776.
The original first 10 outside-window packets are retained, not skipped.
Those 990 callbacks' first/last QPC 802433586940/802532484898 are transfer stamps,
not source sample alignment or full-packet duration coverage.

Owner `MoirenAppVadPlay`, native `[11,802361215267,11,1,1]`, reset 0/admission 2/
revision 2 remains stable; seal frame_capacity, worker joined/completed.
Whole original PCM analysis returns all_zero_observed/unknown_zero_energy.

- Raw SHA-256: `124617C1F65E92D3BC895FBD869E4BB16A30754B198F59E6E973949B9AAA1B01`.
- Metadata SHA-256: `353DCCD7307B1021CDC51B539ABB1A9DBB6C1602AB74C1AFE32639B22AE44B99`.

## Interpretation and retention

The selected session/endpoint master controls are not muted or zero at this
one observation. This excludes those particular readings as evidence of a
master-mute/zero-volume confound at that instant, not every gain/policy/APO,
transition or later control change. Scalar 1.0 is not an effective PCM-gain
measurement. The nonzero limited client prefix and zero READ still do not
identify a sole Windows/driver cause or prove waveform delivery/continuity.
Do not repeat another equivalent headless Process control or tune queues as
though this were a latency result. The missing kernel render/READ content
boundary remains the next distinct observation if diagnostics continue.

Collector/holder/source exit 0, errors empty, no forced/residual owners. Holder
60783 ms contains 60003 ms idle/59 verified windows, zero recorded timeline glitches
or epochs; this is idle progress, not continuous nonzero PCM. Service joined/
control released are true. Public endpoint_absence_confirmed remains false;
independent final PnP audit finds only healthy `ROOT\MEDIA\0001`. Driver Running,
oem201.inf/3.1.1.607, SYS hash
`AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268`
and boot 2026-10-30T09:50:49.659845+08:00 are unchanged. No settings, cache or
original evidence is changed or deleted.

Host originals: `D:\coding\moiren-evidence\20261031-provider-input-volume-snapshot\volume1`.
Target originals: `D:\MoirenTest\20261031-provider-input-volume-snapshot\runs\volume1`.
Archive `provider-input-volume-snapshot-20261031.zip` contains 38 payload members
plus manifest, 17961993 payload bytes/5586123 ZIP bytes. Target and host verify
every member length/hash, embedded/external manifest and independently downloaded
loose originals. Archive guards separately pass 8 Python and 14 PS5.1 fixtures.
RunnerRevision `b6b26b7` is an operator checkout checkpoint, not git-tracked
evidence-script provenance; the exact script hash independently pins it.

- ZIP SHA-256: `83F49C248C2BB7086665867041DE9871ADA4A80938991AB1CD9D27862F41A2C6`.
- Manifest SHA-256: `5795E1CE8B764F642757DF4CAA7A03D5EA21A0BCBD07CFFA8EF0BDDA277413BF`.

Host-tooling snapshot `provider-volume-snapshot-host-tooling-20261031-v2.zip`
contains 60 explicit stable payload members plus manifest: 735884 payload bytes,
185067 ZIP bytes. It retains selected source, RED/GREEN logs, runner and archive
fixtures, analysis and verifier tooling; not executable artifacts or native PCM.
The root and independent reviewer compare every payload with its current original
and verify the embedded/external manifest. Fresh resume checks remain loose and
are not claimed as members. The prior snapshot captured an in-progress resume
log; preserve it as an incomplete snapshot, not a current-original PASS. The v2
member set is the prior set minus that one log, not a new broad directory scan.

- Tooling ZIP SHA-256: `2DA84E2720C7194B8FE7EDABCAE68799431137CE2606A8F25238F644585AB058`.
- Tooling manifest SHA-256: `D787B47E567B5A6C03AB898DA8B0A210CE004A59CD4139506C715858E4124539`.

Prior remote checkpoint `a89552c`, CI run `41146723636`, is independently
rechecked as completed/success with 20/20 jobs. That run does not validate the
new source at `b6b26b7`; its local checks above are not a substitute for new CI.

S1-S3, calibrated p95 <= 10 ms, lower-buffer defaults and a matched VAD/Process
latency comparison remain open; collector exit 0 is not acceptance.
